//! Shared delivery settings and HTTP transport for the gateway commands.
use crate::{
    config::{self, Config, PublishSettings},
    discovery, repository_identity,
};
use anyhow::{Context, Result, ensure};
use std::{path::Path, time::Duration};
use ureq::http::{HeaderValue, Uri};

pub(crate) fn endpoint(gateway_url: &str, path: &str) -> Result<Uri> {
    let invalid_url = "--gateway-url must be an HTTP or HTTPS origin without credentials, a path, query or fragment";
    let base: Uri = gateway_url
        .parse()
        .map_err(|_| anyhow::anyhow!(invalid_url))?;
    let scheme = base.scheme().context(invalid_url)?;
    let authority = base.authority().context(invalid_url)?;
    let host = authority.host();
    ensure!(
        matches!(scheme.as_str(), "http" | "https")
            && !gateway_url.contains(['?', '#', '@'])
            && base.path() == "/"
            && !host.is_empty()
            && (authority.as_str() == host || authority.port_u16().is_some()),
        "{invalid_url}"
    );
    Uri::builder()
        .scheme(scheme.clone())
        .authority(authority.clone())
        .path_and_query(path)
        .build()
        .map_err(|_| anyhow::anyhow!(invalid_url))
}

pub(crate) fn settings(root: &Path) -> Result<PublishSettings> {
    let config =
        discovery::document::<Config>(root, &config::path("config.toml")).map_err(|_| {
            anyhow::anyhow!("cannot read .please/config.toml; check the project configuration")
        })?;
    let settings = config
        .publish
        .context("configure [publish] in .please/config.toml")?;
    settings.validate()?;
    Ok(settings)
}

pub(crate) fn repository(root: &Path, settings: &PublishSettings) -> Result<String> {
    settings.repository.clone().or_else(|| repository_identity::read_origin(root))
        .context("cannot determine repository identity; set a network Git remote named origin or publish.repository")
}

pub(crate) fn authorization(key_env: &str) -> Result<HeaderValue> {
    let key = std::env::var(key_env)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .context("publish API key environment variable is unset, empty or invalid; check publish.key_env")?;
    ensure!(
        key.bytes().all(|byte| byte.is_ascii_graphic()),
        "publish API key is invalid"
    );
    let mut authorization = HeaderValue::from_str(&format!("Bearer {key}"))
        .map_err(|_| anyhow::anyhow!("publish API key is invalid"))?;
    authorization.set_sensitive(true);
    Ok(authorization)
}

pub(crate) fn post(
    endpoint: Uri,
    authorization: HeaderValue,
    body: &[u8],
    destination: &str,
) -> Result<ureq::http::Response<ureq::Body>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    agent
        .post(endpoint)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|_| {
            anyhow::anyhow!("could not reach the {destination} gateway or receive its response")
        })
}
