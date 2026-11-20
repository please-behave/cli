//! User-level trust in Git repositories, stored outside every project.
use super::cache::Lock;
use crate::{config, limits::MAX_METADATA_BYTES};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    #[serde(default)]
    repositories: BTreeSet<String>,
}

fn directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute(), "XDG_CONFIG_HOME must be absolute");
        return Ok(path.join("please"));
    }
    let home =
        PathBuf::from(std::env::var_os("HOME").context("HOME unavailable; set XDG_CONFIG_HOME")?);
    ensure!(home.is_absolute(), "HOME must be absolute");
    Ok(home.join(".config/please"))
}

fn read(directory: &Path) -> Result<Store> {
    let path = directory.join("trust.toml");
    if !path.try_exists()? {
        return Ok(Store::default());
    }
    let store: Store = toml::from_str(&config::read_bounded(&path, MAX_METADATA_BYTES)?)
        .context("invalid trust store")?;
    ensure!(store.version == 0, "unsupported trust store version");
    Ok(store)
}

fn update(change: impl FnOnce(&mut BTreeSet<String>) -> bool) -> Result<bool> {
    let directory = directory()?;
    fs::create_dir_all(&directory).context("cannot create trust store directory")?;
    let _lock = Lock::acquire(&directory.join("trust.lock"))?;
    let mut store = read(&directory)?;
    let changed = change(&mut store.repositories);
    if changed {
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(toml::to_string_pretty(&store)?.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(directory.join("trust.toml"))
            .context("cannot publish trust store")?;
        fs::File::open(&directory)?.sync_all()?;
    }
    Ok(changed)
}

/// Lexical identity of a validated Git location; no provider-specific rules apply.
pub fn identity(git: &str) -> String {
    let trim = |text: &str| {
        let text = text.trim_end_matches('/');
        text.strip_suffix(".git").unwrap_or(text).to_owned()
    };
    let local = |path: &str| {
        let path = trim(path);
        fs::canonicalize(&path)
            .ok()
            .and_then(|canonical| canonical.to_str().map(str::to_owned))
            .unwrap_or(path)
    };
    if let Some(path) = git.strip_prefix("file://") {
        return local(path);
    }
    if let Some((scheme, address)) = git.split_once("://") {
        let (authority, path) = address.split_once('/').unwrap_or((address, ""));
        let (userinfo, host) = authority
            .rsplit_once('@')
            .map_or(("", authority), |(user, host)| (user, host));
        let scheme = scheme.to_ascii_lowercase();
        let host = host.to_ascii_lowercase();
        let host = match (scheme.as_str(), host.rsplit_once(':')) {
            ("https", Some((name, "443")))
            | ("http", Some((name, "80")))
            | ("ssh", Some((name, "22")))
            | ("git", Some((name, "9418"))) => name.to_owned(),
            _ => host,
        };
        let userinfo = if userinfo.is_empty() {
            String::new()
        } else {
            format!("{userinfo}@")
        };
        return format!("{scheme}://{userinfo}{host}/{}", trim(path));
    }
    if Path::new(git).is_absolute() {
        return local(git);
    }
    match git.split_once(':') {
        Some((host, path)) => {
            let (userinfo, host) = host
                .rsplit_once('@')
                .map_or(("", host), |(user, host)| (user, host));
            let userinfo = if userinfo.is_empty() {
                String::new()
            } else {
                format!("{userinfo}@")
            };
            format!("{userinfo}{}:{}", host.to_ascii_lowercase(), trim(path))
        }
        None => trim(git),
    }
}

/// Fails unless every named repository is trusted, reading only the local store.
pub fn require<'a>(repositories: impl IntoIterator<Item = &'a str>) -> Result<()> {
    let wanted: BTreeSet<String> = repositories.into_iter().map(identity).collect();
    let trusted = read(&directory()?)?.repositories;
    if let Some(repository) = wanted
        .iter()
        .find(|repository| !trusted.contains(*repository))
    {
        anyhow::bail!(
            "Git repository {repository} is not trusted; review it and run: please trust {repository}"
        );
    }
    Ok(())
}

/// Returns whether the store changed.
pub fn add(git: &str) -> Result<bool> {
    let identity = identity(git);
    update(|repositories| repositories.insert(identity))
}

/// Returns whether the store changed.
pub fn remove(git: &str) -> Result<bool> {
    let identity = identity(git);
    update(|repositories| repositories.remove(&identity))
}

pub fn list() -> Result<Vec<String>> {
    Ok(read(&directory()?)?.repositories.into_iter().collect())
}
