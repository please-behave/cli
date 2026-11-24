use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::time::Duration;
use ureq::http::{HeaderValue, Uri, uri::Scheme};

const MAX_TOKEN_RESPONSE_BYTES: u64 = 64 * 1024;

pub(super) fn bearer(token: &str) -> Option<HeaderValue> {
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
        return None;
    }
    let mut header = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
    header.set_sensitive(true);
    Some(header)
}

pub(super) fn request_token_from_github(
    agent: &ureq::Agent,
    url: &str,
    request_token: &str,
    gateway: &Uri,
) -> Result<HeaderValue> {
    #[derive(Deserialize)]
    struct TokenResponse {
        value: String,
    }

    let endpoint: Uri = url
        .parse()
        .ok()
        .context("invalid GitHub OIDC request URL")?;
    ensure!(
        endpoint.scheme() == Some(&Scheme::HTTPS)
            && endpoint
                .authority()
                .is_some_and(|authority| !authority.as_str().contains('@'))
            && !url.contains('#'),
        "GitHub OIDC request URL must be an HTTPS URL without credentials or a fragment"
    );
    let authorization = bearer(request_token).context("invalid GitHub OIDC request credential")?;
    let scheme = gateway.scheme_str().context("missing gateway scheme")?;
    let authority = gateway.authority().context("missing gateway authority")?;
    let audience = format!("{scheme}://{authority}").to_ascii_lowercase();
    let mut response = agent
        .get(endpoint)
        .query("audience", &audience)
        .header("Authorization", authorization)
        .header("Accept", "application/json")
        .config()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .call()
        .ok()
        .context("could not request a GitHub OIDC token")?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "GitHub OIDC token request failed (HTTP {})",
        status.as_u16()
    );
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_TOKEN_RESPONSE_BYTES)
        .read_to_vec()
        .ok()
        .context("could not read the GitHub OIDC token response")?;
    let token: TokenResponse = serde_json::from_slice(&body)
        .ok()
        .context("invalid GitHub OIDC token response")?;
    bearer(&token.value).context("invalid GitHub OIDC token")
}
