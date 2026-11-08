//! Decode string commands once; legacy arrays and execution keep their existing contract.
use serde::{Deserialize, Deserializer, de::Error};

/// A tokenized command retaining whether explicit references are supported.
#[derive(Debug, Clone)]
pub struct Command {
    arguments: Vec<String>,
    original: Option<String>,
}

impl std::ops::Deref for Command {
    type Target = [String];
    fn deref(&self) -> &Self::Target {
        &self.arguments
    }
}

impl serde::Serialize for Command {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.original {
            Some(text) => serializer.serialize_str(text),
            None => self.arguments.serialize(serializer),
        }
    }
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<Option<Command>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        String(String),
        Array(Vec<String>),
    }
    match Option::<Input>::deserialize(deserializer)? {
        Some(Input::String(text)) => {
            let arguments = parse(&text).map_err(D::Error::custom)?;
            for argument in &arguments {
                reference(argument).map_err(D::Error::custom)?;
            }
            Ok(Some(Command {
                arguments,
                original: Some(text),
            }))
        }
        Some(Input::Array(arguments)) => Ok(Some(Command {
            arguments,
            original: None,
        })),
        None => Ok(None),
    }
}

/// Validate reserved tokens without consulting the filesystem.
fn reference(argument: &str) -> anyhow::Result<Option<(&str, &str)>> {
    use anyhow::ensure;
    if !argument.starts_with('@') || argument.starts_with("@@") {
        return Ok(None);
    }
    let (prefix, suffix) = argument.split_once('/').ok_or_else(|| anyhow::anyhow!(
        "check.command: invalid path reference {argument:?}; use @rule/, @repo/ or @source/ followed by a relative path, or @@ for a literal @"
    ))?;
    ensure!(
        matches!(prefix, "@rule" | "@repo" | "@source"),
        "check.command: unsupported path reference {argument:?}; supported prefixes are @rule/, @repo/ and @source/; use @@ for a literal @"
    );
    ensure!(
        !suffix.contains(['\\', ':', '\0'])
            && suffix
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."),
        "check.command: invalid path reference {argument:?}; use nonempty relative components separated by /, without ., .., backslashes or drive prefixes"
    );
    Ok(Some((prefix, suffix)))
}

impl Command {
    pub(crate) fn resolve(
        &self,
        repo: &std::path::Path,
        source: &std::path::Path,
        rule: &std::path::Path,
    ) -> anyhow::Result<Vec<std::ffi::OsString>> {
        use anyhow::Context;
        self.arguments.iter().map(|argument| {
            if self.original.is_none() {
                return Ok(argument.into());
            }
            if let Some(literal) = argument.strip_prefix("@@") {
                return Ok(format!("@{literal}").into());
            }
            let Some((prefix, suffix)) = reference(argument)? else {
                return Ok(argument.into());
            };
            let root = match prefix {
                "@rule" => rule,
                "@source" => source,
                "@repo" => repo,
                _ => unreachable!(),
            };
            crate::config::source_path(root, std::path::Path::new(suffix))
                .and_then(|path| Ok(path.canonicalize()?))
                .map(|path| path.into_os_string())
                .with_context(|| format!("check.command: cannot resolve {argument:?} under {}; referenced paths must exist and contain no symlinks", root.display()))
        }).collect()
    }
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
            command: Option<Command>,
        }
        toml::from_str::<Definition>(value)
            .unwrap()
            .command
            .unwrap()
    }

    #[test]
    fn references_resolve_after_tokenization_and_arrays_stay_literal() {
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
        let resolved = parsed.resolve(&repo, &source, &rule).unwrap();
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
        let array =
            command(r#"command = ['@rule/missing', '@unknown', '@@literal', '@source/../escape']"#);
        assert_eq!(
            array.resolve(&repo, &source, &rule).unwrap(),
            array
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        );
        // Serialization must retain string/array identity for a subsequent load.
        for original in [parsed, array] {
            let serialized = serde_json::to_value(&original).unwrap();
            assert_eq!(serialized.is_string(), original.original.is_some());
        }
    }

    #[test]
    fn missing_references_have_no_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let rule = temp.path().join("rule");
        fs::create_dir(&rule).unwrap();
        fs::write(temp.path().join("only-in-repo"), "exists").unwrap();
        let parsed = command("command = 'tool @rule/only-in-repo'");
        let error = parsed.resolve(temp.path(), temp.path(), &rule).unwrap_err();
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
        for prefix in ["@rule", "@source", "@repo"] {
            for suffix in ["linked/file", "file-link", "escape/tmp"] {
                let parsed = command(&format!("command = 'tool {prefix}/{suffix}'"));
                assert!(parsed.resolve(root, root, root).is_err());
            }
        }
        // Resolve anew at execution time rather than trusting a previous path.
        let parsed = command("command = 'tool @rule/real/file'");
        assert!(parsed.resolve(root, root, root).is_ok());
        fs::remove_file(root.join("real/file")).unwrap();
        symlink("/tmp", root.join("real/file")).unwrap();
        assert!(parsed.resolve(root, root, root).is_err());
    }
}
