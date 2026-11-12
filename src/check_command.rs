//! Decode check commands once and resolve their explicit references at execution time.
//!
//! A command is one string: an executable and arguments without a shell. A token starting with
//! `@` is a reference. Rule inputs are passed explicitly as arguments, never through the
//! environment, so a script decides itself what it forwards to the tools it starts:
//!
//! | Token | Resolves to |
//! | --- | --- |
//! | `@rule/<path>` | a file or directory inside the rule directory |
//! | `@repo/<path>` | a path inside the consumer repository |
//! | `@source/<path>` | a path inside the rule source repository |
//! | `@param/<name>` | the value of a declared rule parameter |
//! | `@context` | the absolute path of the run context file |
//!
//! `@@` escapes a literal leading `@`.
use crate::parameters::Environment;
use serde::{Deserialize, Deserializer, de::Error};

/// A tokenized command and the original text it was written as.
#[derive(Debug, Clone)]
pub struct Command {
    arguments: Vec<String>,
    original: String,
}

impl std::ops::Deref for Command {
    type Target = [String];
    fn deref(&self) -> &Self::Target {
        &self.arguments
    }
}

impl serde::Serialize for Command {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.original)
    }
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Command, D::Error>
where
    D: Deserializer<'de>,
{
    let original = String::deserialize(deserializer)?;
    let arguments = parse(&original).map_err(D::Error::custom)?;
    for argument in &arguments {
        reference(argument).map_err(D::Error::custom)?;
    }
    Ok(Command {
        arguments,
        original,
    })
}

enum Reference<'a> {
    Path(&'a str, &'a str),
    Parameter(&'a str),
    Context,
}

/// Validate reserved tokens without consulting the filesystem.
fn reference(argument: &str) -> anyhow::Result<Option<Reference<'_>>> {
    use anyhow::ensure;
    if !argument.starts_with('@') || argument.starts_with("@@") {
        return Ok(None);
    }
    if argument == "@context" {
        return Ok(Some(Reference::Context));
    }
    let (prefix, suffix) = argument.split_once('/').ok_or_else(|| anyhow::anyhow!(
        "check.command: invalid reference {argument:?}; use @context, @param/<name>, or @rule/, @repo/ or @source/ followed by a relative path, or @@ for a literal @"
    ))?;
    if prefix == "@param" {
        ensure!(
            suffix
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
                && suffix
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "check.command: invalid parameter reference {argument:?}; use @param/ followed by a declared parameter name"
        );
        return Ok(Some(Reference::Parameter(suffix)));
    }
    ensure!(
        matches!(prefix, "@rule" | "@repo" | "@source"),
        "check.command: unsupported reference {argument:?}; supported are @context, @param/<name>, @rule/, @repo/ and @source/; use @@ for a literal @"
    );
    ensure!(
        !suffix.contains(['\\', ':', '\0'])
            && suffix
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "check.command: invalid path reference {argument:?}; use nonempty relative components separated by /, without ., .., backslashes or drive prefixes"
    );
    Ok(Some(Reference::Path(prefix, suffix)))
}

impl Command {
    /// Parse a command written in a rule definition.
    pub fn new(text: &str) -> anyhow::Result<Self> {
        let arguments = parse(text).map_err(anyhow::Error::msg)?;
        for argument in &arguments {
            reference(argument)?;
        }
        Ok(Self {
            arguments,
            original: text.to_owned(),
        })
    }

    /// Names of the rule parameters this command refers to.
    pub(crate) fn parameter_names(&self) -> Vec<&str> {
        self.arguments
            .iter()
            .filter_map(|argument| match reference(argument) {
                Ok(Some(Reference::Parameter(name))) => Some(name),
                _ => None,
            })
            .collect()
    }

    /// Explicit references and literal arguments can name local snapshot inputs.
    pub(crate) fn input_paths(
        &self,
        repo: &std::path::Path,
        rule: &std::path::Path,
    ) -> anyhow::Result<Vec<std::path::PathBuf>> {
        self.arguments
            .iter()
            .map(|argument| {
                match reference(argument)? {
                    Some(Reference::Path(prefix, suffix)) => {
                        return Ok(if prefix == "@rule" { rule } else { repo }.join(suffix));
                    }
                    Some(Reference::Parameter(_) | Reference::Context) => {
                        return Ok(repo.join(argument));
                    }
                    None => {}
                }
                if let Some(literal) = argument.strip_prefix("@@") {
                    return Ok(repo.join(format!("@{literal}")));
                }
                Ok(repo.join(argument))
            })
            .collect()
    }

    pub(crate) fn resolve(
        &self,
        locations: &Locations<'_>,
    ) -> anyhow::Result<Vec<std::ffi::OsString>> {
        use anyhow::Context;
        self.arguments.iter().map(|argument| {
            if let Some(literal) = argument.strip_prefix("@@") {
                return Ok(format!("@{literal}").into());
            }
            match reference(argument)? {
                None => Ok(argument.into()),
                Some(Reference::Context) => locations
                    .context
                    .map(|path| path.as_os_str().to_owned())
                    .ok_or_else(|| anyhow::anyhow!("check.command: {argument:?} needs a run context, which is not available here")),
                Some(Reference::Parameter(name)) => locations
                    .parameters
                    .get(name)
                    .map(std::ffi::OsString::from)
                    .ok_or_else(|| anyhow::anyhow!("check.command: parameter {name:?} has no value")),
                Some(Reference::Path(prefix, suffix)) => {
                    let root = match prefix {
                        "@rule" => locations.rule,
                        "@source" => locations.source,
                        _ => locations.repo,
                    };
                    crate::config::source_path(root, std::path::Path::new(suffix))
                        .and_then(|path| Ok(path.canonicalize()?))
                        .map(|path| path.into_os_string())
                        .with_context(|| format!("check.command: cannot resolve {argument:?} under {}; referenced paths must exist and contain no symlinks", root.display()))
                }
            }
        }).collect()
    }
}

/// Everything a command's references may resolve against.
pub(crate) struct Locations<'a> {
    pub repo: &'a std::path::Path,
    pub source: &'a std::path::Path,
    pub rule: &'a std::path::Path,
    pub context: Option<&'a std::path::Path>,
    pub parameters: &'a Environment,
}

fn parse(command: &str) -> Result<Vec<String>, &'static str> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;
    for c in command.chars() {
        if c.is_control() && c != '\t' {
            return Err("check.command: control characters other than tab are unsupported");
        }
        if escaped {
            argument.push(c);
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            escaped = true;
            started = true;
        } else if quote == Some(c) {
            quote = None;
        } else if quote != Some('\'') && matches!(c, '$' | '`') {
            return Err(
                "check.command: variable expansion and substitution are unsupported; quote with single quotes or escape literal characters",
            );
        } else if quote.is_some() {
            argument.push(c);
        } else if matches!(c, '\'' | '"') {
            quote = Some(c);
            started = true;
        } else if matches!(c, ' ' | '\t') {
            if started {
                arguments.push(std::mem::take(&mut argument));
                started = false;
            }
        } else if matches!(c, '|' | '&' | ';' | '<' | '>' | '(' | ')' | '#') {
            return Err(
                "check.command: shell operators and comments are unsupported; quote or escape literal characters",
            );
        } else {
            argument.push(c);
            started = true;
        }
    }
    if escaped {
        return Err("check.command: trailing escape");
    }
    if quote.is_some() {
        return Err("check.command: unclosed quote");
    }
    if started {
        arguments.push(argument);
    }
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn command(value: &str) -> Command {
        #[derive(Deserialize)]
        struct Definition {
            #[serde(deserialize_with = "deserialize")]
            command: Command,
        }
        toml::from_str::<Definition>(value).unwrap().command
    }

    fn locations<'a>(
        repo: &'a std::path::Path,
        source: &'a std::path::Path,
        rule: &'a std::path::Path,
        parameters: &'a Environment,
    ) -> Locations<'a> {
        Locations {
            repo,
            source,
            rule,
            context: None,
            parameters,
        }
    }

    #[test]
    fn references_resolve_after_tokenization() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("consumer root");
        let source = temp.path().join("cached source");
        let rule = source.join("rules/checks/current rule");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&rule).unwrap();
        for root in [&repo, &source, &rule] {
            fs::write(root.join("file with spaces"), "data").unwrap();
        }
        let parsed = command(
            r#"command = '"@rule/file with spaces" "@repo/file with spaces" "@source/file with spaces" @@rule/file @@unknown @@@literal ordinary/path --file=@rule/literal'"#,
        );
        let parameters = Environment::new();
        let resolved = parsed
            .resolve(&locations(&repo, &source, &rule, &parameters))
            .unwrap();
        for (actual, root) in resolved.iter().zip([&rule, &repo, &source]) {
            assert_eq!(
                actual,
                root.join("file with spaces")
                    .canonicalize()
                    .unwrap()
                    .as_os_str()
            );
        }
        assert_eq!(
            &resolved[3..],
            &[
                "@rule/file",
                "@unknown",
                "@@literal",
                "ordinary/path",
                "--file=@rule/literal"
            ]
        );
    }

    #[test]
    fn context_and_parameters_are_explicit_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path();
        let parameters = Environment::from([
            ("scopes_file".to_owned(), "/abs/scopes.txt".to_owned()),
            ("label".to_owned(), "two words".to_owned()),
        ]);
        let parsed = command("command = 'tool @context @param/scopes_file @param/label'");
        assert_eq!(parsed.parameter_names(), ["scopes_file", "label"]);
        let mut locations = locations(path, path, path, &parameters);
        assert!(parsed.resolve(&locations).is_err(), "no context available");
        let context = path.join("context.json");
        locations.context = Some(&context);
        assert_eq!(
            parsed.resolve(&locations).unwrap(),
            [
                "tool",
                context.to_str().unwrap(),
                "/abs/scopes.txt",
                "two words"
            ]
        );
        let missing = command("command = 'tool @param/absent'");
        assert!(format!("{:#}", missing.resolve(&locations).unwrap_err()).contains("no value"));
    }

    #[test]
    fn malformed_references_are_rejected_when_loading() {
        for text in [
            "command = 'tool @unknown/path'",
            "command = 'tool @rule'",
            "command = 'tool @param/'",
            "command = 'tool @param/a/b'",
            "command = 'tool @context/extra'",
            "command = 'tool @source/../escape'",
        ] {
            #[derive(Deserialize)]
            struct Definition {
                #[serde(deserialize_with = "deserialize")]
                #[allow(dead_code)]
                command: Command,
            }
            assert!(toml::from_str::<Definition>(text).is_err(), "{text}");
        }
    }

    #[test]
    fn missing_references_have_no_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let rule = temp.path().join("rule");
        fs::create_dir(&rule).unwrap();
        fs::write(temp.path().join("only-in-repo"), "exists").unwrap();
        let parsed = command("command = 'tool @rule/only-in-repo'");
        let parameters = Environment::new();
        let error = parsed
            .resolve(&locations(temp.path(), temp.path(), &rule, &parameters))
            .unwrap_err();
        assert!(format!("{error:#}").contains("@rule/only-in-repo"));
    }

    #[cfg(unix)]
    #[test]
    fn references_reject_symlinks_even_within_the_root() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("real")).unwrap();
        fs::write(root.join("real/file"), "data").unwrap();
        symlink(root.join("real"), root.join("linked")).unwrap();
        symlink(root.join("real/file"), root.join("file-link")).unwrap();
        symlink("/", root.join("escape")).unwrap();
        let parameters = Environment::new();
        let here = locations(root, root, root, &parameters);
        for prefix in ["@rule", "@source", "@repo"] {
            for suffix in ["linked/file", "file-link", "escape/tmp"] {
                let parsed = command(&format!("command = 'tool {prefix}/{suffix}'"));
                assert!(parsed.resolve(&here).is_err());
            }
        }
        // Resolve anew at execution time rather than trusting a previous path.
        let parsed = command("command = 'tool @rule/real/file'");
        assert!(parsed.resolve(&here).is_ok());
        fs::remove_file(root.join("real/file")).unwrap();
        symlink("/tmp", root.join("real/file")).unwrap();
        assert!(parsed.resolve(&here).is_err());
    }
}
