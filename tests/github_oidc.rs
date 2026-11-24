// Exercise the private token exchange with a transport that performs no network I/O.
#[path = "../src/gateway/auth.rs"]
mod auth;

use std::io::{Cursor, Read};
use ureq::{
    Agent, Error,
    unversioned::{
        resolver::DefaultResolver,
        transport::{Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport},
    },
};

const REQUEST_TOKEN: &str = "fixture-request-token";
const ID_TOKEN: &str = "fixture.id.token";
const ENDPOINT: &str = "https://127.0.0.1/token?runner=fixture";

#[derive(Debug)]
struct TokenEndpoint {
    status: u16,
    body: &'static str,
}

impl Connector for TokenEndpoint {
    type Out = TokenConnection;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _: Option<()>,
    ) -> Result<Option<Self::Out>, Error> {
        assert!(details.needs_tls());
        assert_eq!(details.uri.path(), "/token");
        let response = format!(
            "HTTP/1.1 {} Test\r\nLocation: https://127.0.0.1/redirect\r\nContent-Length: {}\r\n\r\n{}",
            self.status,
            self.body.len(),
            self.body
        );
        Ok(Some(TokenConnection {
            buffers: LazyBuffers::new(4096, 4096),
            response: Cursor::new(response.into_bytes()),
        }))
    }
}

#[derive(Debug)]
struct TokenConnection {
    buffers: LazyBuffers,
    response: Cursor<Vec<u8>>,
}

impl Transport for TokenConnection {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, _: NextTimeout) -> Result<(), Error> {
        let request = std::str::from_utf8(&self.buffers.output()[..amount]).unwrap();
        assert!(request.starts_with(
            "GET /token?runner=fixture&audience=https%3A%2F%2Fgateway.example%3A8443 HTTP/1.1\r\n"
        ), "{request}");
        assert!(request.lines().any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.trim() == format!("Bearer {REQUEST_TOKEN}")
            })
        }));
        assert_eq!(request.matches(REQUEST_TOKEN).count(), 1);
        Ok(())
    }

    fn await_input(&mut self, _: NextTimeout) -> Result<bool, Error> {
        let read = self.response.read(self.buffers.input_append_buf())?;
        self.buffers.input_appended(read);
        Ok(read != 0)
    }

    fn is_open(&mut self) -> bool {
        false
    }

    fn is_tls(&self) -> bool {
        true
    }
}

fn token_endpoint(status: u16, body: &'static str) -> Agent {
    let config = Agent::config_builder().proxy(None).build();
    Agent::with_parts(
        config,
        TokenEndpoint { status, body },
        DefaultResolver::default(),
    )
}

#[test]
// REQ-0358: request a token with the normalized audience and treat it as sensitive.
fn obtains_a_gateway_scoped_bearer() {
    let agent = token_endpoint(200, r#"{"value":"fixture.id.token"}"#);
    let gateway = "https://Gateway.Example:8443/v0/attestations"
        .parse()
        .unwrap();
    let result =
        auth::request_token_from_github(&agent, ENDPOINT, REQUEST_TOKEN, &gateway).unwrap();
    assert_eq!(result.to_str().unwrap(), format!("Bearer {ID_TOKEN}"));
    assert!(result.is_sensitive());
    assert!(auth::bearer(REQUEST_TOKEN).unwrap().is_sensitive());
}

#[test]
// REQ-0358: failing, redirecting and malformed responses omit credential values.
fn rejects_unusable_responses_without_disclosing_credentials() {
    let gateway = "https://Gateway.Example:8443/v0/rule-registrations"
        .parse()
        .unwrap();
    for (status, body, expected) in [
        (302, REQUEST_TOKEN, "HTTP 302"),
        (503, ID_TOKEN, "HTTP 503"),
        (200, REQUEST_TOKEN, "invalid GitHub OIDC token response"),
        (200, r#"{"value":""}"#, "invalid GitHub OIDC token"),
    ] {
        let agent = token_endpoint(status, body);
        let error =
            auth::request_token_from_github(&agent, ENDPOINT, REQUEST_TOKEN, &gateway).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains(REQUEST_TOKEN) && !error.contains(ID_TOKEN));
    }
}
