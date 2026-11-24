#[path = "mod.rs"]
mod support;

use crate::{
    VerificationMode, adapter::Configured, config, gateway, publish, register, rule_digest,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::Duration,
};

const TOKEN: &str = "fixture.oidc.token";
const SENDER: &str = "github.com/owner/mixedcase-repo";
const RULE_PATH: &str = ".please/checks/example";

fn git(root: &Path, args: &[&str]) {
    let output = support::command("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

fn exchange(
    path: &'static str,
    status: u16,
    send: impl FnOnce(&str) -> anyhow::Result<()>,
) -> Value {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    thread::scope(|scope| {
        let server = scope.spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "missing {path} request");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = BufReader::new(&mut stream);
            let mut lines = request.by_ref().lines();
            assert_eq!(lines.next().unwrap().unwrap(), format!("POST {path} HTTP/1.1"));
            let headers = lines
                .map(Result::unwrap)
                .take_while(|line| !line.is_empty())
                .collect::<Vec<_>>();
            assert!(headers.iter().any(|header| {
                let (name, value) = header.split_once(':').unwrap();
                name.eq_ignore_ascii_case("authorization") && value.trim() == format!("Bearer {TOKEN}")
            }));
            let length = headers.iter().find_map(|header| {
                let (name, value) = header.split_once(':').unwrap();
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            }).unwrap();
            let mut body = vec![0; length];
            request.read_exact(&mut body).unwrap();
            let document: Value = serde_json::from_slice(&body).unwrap();
            let response = if path == "/v0/rule-registrations" {
                let rule = &document["rules"][0];
                assert_eq!(rule["source"], SENDER);
                let digest = rule_digest::digest(rule.clone()).unwrap();
                json!({"outcomes": [{"outcome": "created", "digest": digest}]}).to_string()
            } else {
                assert_eq!(document["repository"], SENDER);
                String::new()
            };
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            ).unwrap();
            document
        });
        send(&url).unwrap();
        server.join().unwrap()
    })
}

#[test]
// REQ-0348, REQ-0352, REQ-0354: both deliveries use the OIDC sender, including in rule digests.
fn delivers_oidc_repository_and_rule_identity_with_a_matching_digest() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    git(&root, &["init", "--quiet"]);
    support::write(&root, ".gitignore", ".please/reports/\n");
    support::write(
        &root,
        &format!("{RULE_PATH}/rule.toml"),
        "id = 'example'\nversion = '1.0'\ndescription = 'Fixture'\nrun_at = ['check']\n[check]\ncommand = 'git --version'\n",
    );
    support::write(
        &root,
        &format!("{RULE_PATH}/rule.yaml"),
        "purpose: Fixture\n",
    );

    for (remote, override_identity, api_key_identity) in [
        (
            "https://GitHub.COM/Owner/MixedCase-Repo.git",
            "",
            "github.com/Owner/MixedCase-Repo",
        ),
        (
            "git@GitHub.COM:Owner/MixedCase-Repo.git",
            "",
            "github.com/Owner/MixedCase-Repo",
        ),
        (
            "https://github.com/owner/other.git",
            "GitHub.COM/Owner/MixedCase-Repo",
            "GitHub.COM/Owner/MixedCase-Repo",
        ),
        ("https://github.com/owner/mixedcase-repo.git", "", SENDER),
    ] {
        git(&root, &["config", "--local", "remote.origin.url", remote]);
        let repository = if override_identity.is_empty() {
            String::new()
        } else {
            format!("repository = {override_identity:?}\n")
        };
        let settings = config::PublishSettings {
            auth: config::PublishAuth::ApiKey,
            key_env: Some("PLEASE_FIXTURE_KEY".into()),
            repository: (!override_identity.is_empty()).then(|| override_identity.into()),
        };
        assert_eq!(
            gateway::repository(&root, &settings).unwrap(),
            api_key_identity
        );
        support::write(
            &root,
            ".please/config.toml",
            &format!("project = 'Fixture'\n[publish]\nauth = 'github-oidc'\n{repository}"),
        );
        support::commit_all(&root);
        let mut report = crate::verify_project_at(
            config::load(&root).unwrap(),
            &Configured,
            VerificationMode::ChecksOnly,
            Some("check"),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.overall, crate::report::Status::Pass);

        let authorize = |settings: &config::PublishSettings, _: &ureq::http::Uri| {
            assert!(matches!(settings.auth, config::PublishAuth::GithubOidc));
            Ok(ureq::http::HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap())
        };
        let registration = exchange("/v0/rule-registrations", 200, |url| {
            register::send_with_authorization(&root, url, |_| {}, authorize)
        });
        let attestation = exchange("/v0/attestations", 201, |url| {
            publish::send_with_authorization(&root, url, authorize)
        });
        let registered = &registration["rules"][0];
        let published = &attestation["results"][0]["rule"];
        for field in ["source", "path", "id", "version"] {
            assert_eq!(published[field], registered[field], "{field} for {remote}");
        }
        assert_eq!(
            published["digest"],
            rule_digest::digest(registered.clone()).unwrap()
        );

        if remote.starts_with("https://GitHub.COM/") {
            let mut old_rule = registered.clone();
            old_rule["source"] = json!(api_key_identity);
            let mut old_metadata = report.results[0].rule_metadata.clone().unwrap();
            old_metadata.source = Some(api_key_identity.into());
            old_metadata.digest = Some(rule_digest::digest(old_rule).unwrap());
            report.results[0].rule_metadata = Some(old_metadata.clone());
            report.plan.scheduled[0].rule_metadata = Some(old_metadata);
            crate::report::write(&root, &report, 2).unwrap();
            let error = publish::send_with_authorization(&root, "http://127.0.0.1", |_, _| {
                panic!("authorization must follow report validation")
            })
            .unwrap_err();
            assert!(error.to_string().contains("run please run again"));
        }
    }
}
