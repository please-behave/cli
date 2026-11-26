//! Shared delivery settings and HTTP transport for the gateway commands.
mod auth;

use crate::{
    config::{self, Config, PublishAuth, PublishSettings},
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
    let config = project_config(root)?;
    settings_from(&config)
}

pub(crate) fn project_config(root: &Path) -> Result<Config> {
    discovery::document(root, &config::path("config.toml")).map_err(|_| {
        anyhow::anyhow!("cannot read .please/config.toml; check the project configuration")
    })
}

pub(crate) fn settings_from(config: &Config) -> Result<PublishSettings> {
    let settings = config
        .publish
        .clone()
        .context("configure [publish] in .please/config.toml")?;
    settings.validate()?;
    Ok(settings)
}

pub(crate) fn repository(root: &Path, settings: &PublishSettings) -> Result<String> {
    repository_identity::for_delivery(root, settings)
        .context("cannot determine repository identity; set a network Git remote named origin or publish.repository")
}

pub(crate) fn authorization(settings: &PublishSettings, endpoint: &Uri) -> Result<HeaderValue> {
    match settings.auth {
        PublishAuth::ApiKey => api_key_authorization(settings),
        PublishAuth::GithubOidc => {
            let request_value = |name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .with_context(|| {
                        format!(
                            "GitHub OIDC requires {name}; grant id-token: write in the workflow"
                        )
                    })
            };
            let url = request_value("ACTIONS_ID_TOKEN_REQUEST_URL")?;
            let request_token = request_value("ACTIONS_ID_TOKEN_REQUEST_TOKEN")?;
            let agent = ureq::Agent::new_with_defaults();
            auth::request_token_from_github(&agent, &url, &request_token, endpoint)
        }
    }
}

pub(crate) fn api_key_authorization(settings: &PublishSettings) -> Result<HeaderValue> {
    let key_env = settings
        .key_env
        .as_deref()
        .context("publish.key_env is required for api-key authentication")?;
    let key = std::env::var(key_env)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .context("publish API key environment variable is unset, empty or invalid; check publish.key_env")?;
    auth::bearer(&key).context("publish API key is invalid")
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
