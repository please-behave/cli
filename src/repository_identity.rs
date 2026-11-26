use crate::{
    config::{PublishAuth, PublishSettings},
    git_snapshot,
};
use std::{path::Path, time::Duration};
use ureq::http::uri::Authority;

pub(crate) fn for_delivery(root: &Path, settings: &PublishSettings) -> Option<String> {
    let mut identity = settings.repository.clone().or_else(|| read_origin(root))?;
    if matches!(settings.auth, PublishAuth::GithubOidc) {
        identity.make_ascii_lowercase();
    }
    Some(identity)
}

pub(crate) fn for_local_rule(root: &Path, settings: Option<&PublishSettings>) -> Option<String> {
    match settings {
        Some(settings) if matches!(settings.auth, PublishAuth::GithubOidc) => {
            for_delivery(root, settings)
        }
        _ => read_origin(root),
    }
}

pub(crate) fn from_identity_or_remote(value: &str) -> Option<std::borrow::Cow<'_, str>> {
    if let Some((host, path)) = value.split_once('/')
        && let Ok(authority) = host.parse::<Authority>()
        && authority.host() == host
        && !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.chars().any(char::is_control)
    {
        return Some(value.into());
    }
    normalize(value).map(Into::into)
}

pub(crate) fn normalize(remote: &str) -> Option<String> {
    let (authority, path) = if let Some((scheme, address)) = remote.split_once("://") {
        if !matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "ssh" | "git" | "git+ssh" | "ssh+git"
        ) {
            return None;
        }
        address.split(['?', '#']).next()?.split_once('/')?
    } else if let Some(end) = remote.find("]:") {
        (&remote[..=end], &remote[end + 2..])
    } else {
        let (host, path) = remote.split_once(':')?;
        if host.len() == 1
            && host.as_bytes()[0].is_ascii_alphabetic()
            && path.starts_with(['/', '\\'])
        {
            return None;
        }
        (host, path)
    };
    let host_port = authority.rsplit('@').next()?;
    let address: Authority = host_port.parse().ok()?;
    let host = address.host();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty()
        || path.is_empty()
        || path.chars().any(char::is_control)
        || path.starts_with('\\')
    {
        return None;
    }
    let identity = format!("{}/{path}", host.to_ascii_lowercase());
    (identity.chars().count() <= 2048).then_some(identity)
}

pub(crate) fn read_origin(root: &Path) -> Option<String> {
    let result = crate::process::run(
        git_snapshot::git(root).args(["remote", "get-url", "origin"]),
        Duration::from_secs(10),
    )
    .ok()?;
    if !result.status.success() || result.timed_out || result.stdout.truncated {
        return None;
    }
    normalize(result.stdout.text.trim_end_matches(['\r', '\n']))
}
