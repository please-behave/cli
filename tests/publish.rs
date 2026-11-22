mod support;

use please::report::{self, Status};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::Output,
    thread,
    time::Duration,
};

const KEY_NAME: &str = "PLEASE_ATTESTATION_TEST_KEY";
const KEY: &str = "offline-test-bearer";

fn git(root: &Path, args: &[&str]) {
    let result = support::command("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
}

fn cli(root: &Path, args: &[&str], key: Option<&str>) -> Output {
    let mut command = support::command(env!("CARGO_BIN_EXE_please"));
    command
        .current_dir(root)
        .args(args)
        .env("PLEASE_CHANGE_BASE", "HEAD")
        .env("NO_PROXY", "*")
        .env("no_proxy", "*");
    if let Some(key) = key {
        command.env(KEY_NAME, key);
    }
    command.output().unwrap()
}

fn publish(root: &Path, gateway_url: &str, key: Option<&str>) -> Output {
    cli(root, &["publish", "--gateway-url", gateway_url], key)
}

fn gateway_url(listener: &TcpListener) -> String {
    format!("http://{}", listener.local_addr().unwrap())
}

fn fixture(repository: Option<&str>) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    please::init::init(root.path()).unwrap();
    let path = root.path().join(".please/config.toml");
    let config = fs::read_to_string(&path).unwrap();
    let identity = repository.map_or_else(String::new, |value| format!("repository = {value:?}\n"));
    fs::write(
        path,
        format!("{config}\n[publish]\nkey_env = '{KEY_NAME}'\n{identity}"),
    )
    .unwrap();
    root
}

fn run(root: &Path, expected: i32) -> report::Report {
    let output = cli(root, &["run", "--at", "check"], None);
    assert_eq!(output.status.code(), Some(expected), "{output:?}");
    report::read(root).unwrap()
}

fn listen() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").unwrap()
}

fn exchange(listener: &TcpListener, root: &Path, status: u16) -> (Output, Value) {
    let server = listener.try_clone().unwrap();
    server.set_nonblocking(true).unwrap();
    let request = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match server.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "no publish request");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = BufReader::new(&mut stream);
        let mut line = String::new();
        request.read_line(&mut line).unwrap();
        assert_eq!(line, "POST /v0/attestations HTTP/1.1\r\n");
        let (mut length, mut authorization, mut json_content) = (None, false, false);
        loop {
            line.clear();
            assert_ne!(request.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            match name.to_ascii_lowercase().as_str() {
                "authorization" => {
                    assert_eq!(value.trim(), format!("Bearer {KEY}"));
                    authorization = true;
                }
                "content-type" => json_content = value.trim() == "application/json",
                "content-length" => length = Some(value.trim().parse::<usize>().unwrap()),
                _ => assert!(!value.contains(KEY)),
            }
        }
        assert!(authorization && json_content);
        let mut body = vec![0; length.unwrap()];
        request.read_exact(&mut body).unwrap();
        assert!(!String::from_utf8_lossy(&body).contains(KEY));
        // Even adversarial response bodies and redirect locations must stay out of diagnostics.
        let redirect = format!(
            "Location: http://{}/redirect\r\n",
            server.local_addr().unwrap()
        );
        write!(
            stream,
            "HTTP/1.1 {status} Test\r\n{redirect}Content-Length: {}\r\nConnection: close\r\n\r\n{KEY}",
            KEY.len()
        )
        .unwrap();
        serde_json::from_slice(&body).unwrap()
    });

    let output = publish(root, &gateway_url(listener), Some(KEY));
    let body = request.join().unwrap();
    assert_no_request(listener);
    (output, body)
}

fn assert_no_request(listener: &TcpListener) {
    listener.set_nonblocking(true).unwrap();
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

fn assert_output(output: &Output, code: i32, message: &str) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(KEY) && !stderr.contains(KEY));
    assert!(
        stdout.contains(message) || stderr.contains(message),
        "{output:?}"
    );
}

#[test]
// REQ-0345, REQ-0284: the destination is required at invocation and rejected in project settings.
fn requires_a_gateway_argument_and_rejects_a_configured_gateway() {
    let listener = listen();
    let url = gateway_url(&listener);
    let root = fixture(Some("fixture.example/owner/repo"));
    support::commit_all(root.path());
    run(root.path(), 0);

    assert_output(
        &cli(root.path(), &["publish"], Some(KEY)),
        2,
        "--gateway-url <URL>",
    );
    assert_no_request(&listener);

    let config_path = root.path().join(".please/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(config_path, format!("{config}\ngateway_url = '{url}'\n")).unwrap();
    let plan = cli(root.path(), &["run", "--plan", "--at", "check"], None);
    assert_output(&plan, 2, "unknown field `gateway_url`");
    assert_output(&publish(root.path(), &url, Some(KEY)), 2, "UNKNOWN");
    assert_no_request(&listener);
}

#[test]
// REQ-0345: invalid destinations are rejected without showing their values or making requests.
fn rejects_invalid_gateway_origins_without_disclosing_the_url() {
    let listener = listen();
    let address = listener.local_addr().unwrap();
    let root = fixture(Some("fixture.example/owner/repo"));
    support::commit_all(root.path());
    run(root.path(), 0);

    for url in [
        format!("http://fixture-user:fixture-password@{address}"),
        format!("http://{address}/private-path"),
        format!("http://{address}/?private-query"),
        format!("http://{address}/#private-fragment"),
        format!("ftp://{address}"),
        "http://".to_string(),
        "http://gateway.example.invalid:65536".to_string(),
    ] {
        let output = publish(root.path(), &url, Some(KEY));
        assert_output(&output, 2, "--gateway-url must be an HTTP or HTTPS origin");
        for stream in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(stream);
            assert!(!text.contains(&url), "{output:?}");
            assert!(!text.contains("fixture-password"), "{output:?}");
        }
        assert_no_request(&listener);
    }
}

#[cfg(unix)]
#[test]
// REQ-0345, REQ-0346: the newest failed run is delivered independently using only attestation fields.
fn projects_newest_failed_report_and_preserves_saved_evidence() {
    let listener = listen();
    let root = fixture(Some("fixture.example/owner/repo"));
    git(
        root.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@example.invalid:other/repo.git",
        ],
    );
    support::check(root.path(), "passing", &["check"], "exit 0\n");
    support::commit_all(root.path());
    assert_eq!(run(root.path(), 0).number, 1);

    support::check(
        root.path(),
        "failing",
        &["check"],
        "echo private-output\nexit 1\n",
    );
    support::configure_adapter(root.path());
    let review_path = support::review_path(root.path(), "advisory");
    fs::write(
        review_path,
        "id = 'advisory'\ndescription = 'Fixture'\nenabled = true\nblocking = false\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\n"
    ).unwrap();
    let mut report = run(root.path(), 1);
    assert_eq!(report.number, 2);
    assert_eq!(report.overall, Status::Fail);
    let (output, document) = exchange(&listener, root.path(), 201);
    assert_output(&output, 0, "Attestation accepted");
    assert_eq!(
        document,
        json!({
            "schema_version": 0,
            "repository": "fixture.example/owner/repo",
            "commit": report.snapshot.head,
            "execution_points": ["check"],
            "overall": "FAIL",
            "results": [
                {"id": "failing", "kind": "check", "status": "FAIL"},
                {"id": "passing", "kind": "check", "status": "PASS"},
                {"id": "advisory", "kind": "review", "status": "NOT_RUN"},
            ],
        })
    );

    // Model a retained transition report with outcomes that are cumbersome to trigger together.
    report.schema_version = 1;
    report.results[0].status = Status::Error;
    report.results[0].exit_code = Some(2);
    report.results[1].status = Status::Cancelled;
    report.results[2].status = Status::Error;
    report.results[2].explanation = Some("private-explanation".into());
    report.results[2].findings.push(report::Finding {
        message: "private-finding".into(),
        path: Some(".please/README.md".into()),
        line: None,
    });
    report.overall = report::overall(&report.results);
    let path = root.path().join(".please/reports/2.json");
    let saved = serde_json::to_vec(&report).unwrap();
    fs::write(&path, &saved).unwrap();
    for review_status in ["ERROR", "FAIL"] {
        let mut fixture: Value = serde_json::from_slice(&saved).unwrap();
        fixture["results"][2]["status"] = json!(review_status);
        let fixture = serde_json::to_vec(&fixture).unwrap();
        fs::write(&path, &fixture).unwrap();
        let (output, document) = exchange(&listener, root.path(), 201);
        assert_output(&output, 0, "Attestation accepted");
        assert_eq!(
            document,
            json!({
                "schema_version": 1,
                "repository": "fixture.example/owner/repo",
                "commit": report.snapshot.head,
                "execution_points": ["check"],
                "overall": "ERROR",
                "results": [
                    {"id": "failing", "kind": "check", "status": "ERROR"},
                    {"id": "passing", "kind": "check", "status": "NOT_RUN"},
                    {"id": "advisory", "kind": "review", "status": "WARN"},
                ],
            })
        );
        assert_eq!(fs::read(&path).unwrap(), fixture);
        assert_eq!(report::saved(root.path()).unwrap(), [1, 2]);
    }
}

#[test]
// REQ-0348: normalize the fetch URL for origin and refuse local or missing origins without an override.
fn derives_repository_from_multiple_git_remote_forms() {
    let listener = listen();
    let root = fixture(None);
    let url = gateway_url(&listener);
    support::commit_all(root.path());
    run(root.path(), 0);
    for (remote, identity) in [
        ("git@GitHub.COM:Owner/Repo.git", "github.com/Owner/Repo"),
        (
            "ssh://git@github.com:2222/Owner/Repo.git/",
            "github.com/Owner/Repo",
        ),
        (
            "https://user:password@github.com:8443/Owner/Repo.git",
            "github.com/Owner/Repo",
        ),
        (
            "git://github.com:9418/Owner/Repo.git",
            "github.com/Owner/Repo",
        ),
        (
            "ssh://git@[2001:db8::1]:2222/Owner/Repo.git",
            "[2001:db8::1]/Owner/Repo",
        ),
        (
            "git@[2001:db8::1]:Owner/Repo.git",
            "[2001:db8::1]/Owner/Repo",
        ),
    ] {
        git(
            root.path(),
            &["config", "--local", "remote.origin.url", remote],
        );
        let (output, body) = exchange(&listener, root.path(), 201);
        assert_output(&output, 0, "Attestation accepted");
        assert_eq!(body["repository"], identity);
    }
    for local in [
        "/tmp/repository.git",
        "file:///tmp/repository.git",
        "../repository.git",
        "C:/repository.git",
    ] {
        git(
            root.path(),
            &["config", "--local", "remote.origin.url", local],
        );
        assert_output(
            &publish(root.path(), &url, Some(KEY)),
            2,
            "cannot determine repository identity",
        );
        assert_no_request(&listener);
    }
    git(root.path(), &["remote", "remove", "origin"]);
    assert_output(
        &publish(root.path(), &url, Some(KEY)),
        2,
        "cannot determine repository identity",
    );
    assert_no_request(&listener);
}

#[test]
// REQ-0347, REQ-0349: saved HEAD, current files and a usable environment key precede any request.
fn rejects_unavailable_or_stale_evidence_and_credentials_before_delivery() {
    let listener = listen();
    let root = fixture(Some("fixture.example/owner/repo"));
    let url = gateway_url(&listener);
    let publish = |key| publish(root.path(), &url, key);
    assert_output(&publish(Some(KEY)), 2, "no saved report");
    run(root.path(), 0);
    assert_output(&publish(Some(KEY)), 2, "has no commit");
    support::commit_all(root.path());
    run(root.path(), 0);

    assert_output(&publish(None), 2, "environment variable is unset");
    assert_output(&publish(Some("")), 2, "environment variable is unset");
    let invalid_key = format!("{KEY}\ninvalid");
    assert_output(&publish(Some(&invalid_key)), 2, "API key is invalid");

    support::write(root.path(), "untracked.txt", "changed");
    assert_output(&cli(root.path(), &["status"], None), 1, "STALE");
    assert_output(&publish(Some(KEY)), 2, "STALE; run please run again");
    fs::remove_file(root.path().join("untracked.txt")).unwrap();

    support::commit_all(root.path());
    assert_output(&cli(root.path(), &["status"], None), 1, "STALE");
    assert_output(&publish(Some(KEY)), 2, "STALE; run please run again");

    support::write(root.path(), ".please/config.toml", "[invalid");
    assert_output(&cli(root.path(), &["status"], None), 2, "UNKNOWN");
    assert_output(&publish(Some(KEY)), 2, "UNKNOWN; run please run again");
    assert_no_request(&listener);
}

#[test]
// REQ-0349, REQ-0350: distinguish delivery failures without echoing a credential or response body.
fn reports_delivery_failures_without_exposing_credentials() {
    let listener = listen();
    let root = fixture(Some("fixture.example/owner/repo"));
    support::commit_all(root.path());
    run(root.path(), 0);
    let path = root.path().join(".please/reports/1.json");
    let saved = fs::read(&path).unwrap();
    for (status, message) in [
        (401, "rejected the API credentials"),
        (403, "rejected the API credentials"),
        (429, "rate limit exceeded"),
        (400, "rejected the attestation document"),
        (413, "rejected the attestation document"),
        (408, "gateway error (HTTP 408)"),
        (502, "gateway error (HTTP 502)"),
        (503, "gateway error (HTTP 503)"),
        (504, "gateway error (HTTP 504)"),
        (307, "gateway error (HTTP 307)"),
    ] {
        let (output, _) = exchange(&listener, root.path(), status);
        assert_output(&output, 2, message);
        assert_eq!(fs::read(&path).unwrap(), saved);
    }
    let url = gateway_url(&listener);
    drop(listener);
    let output = publish(root.path(), &url, Some(KEY));
    assert_output(&output, 2, "could not reach the attestation gateway");
    assert_eq!(fs::read(&path).unwrap(), saved);
}
