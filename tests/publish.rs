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
const REGISTRATION_REPOSITORY: &str = "fixture.example/Owner/Repository";
const CHECK_DIRECTORY: &str = ".please/checks/check-fixture";
const REVIEW_DIRECTORY: &str = "sets/core/reviews/review-fixture";
const CHECK_RULE: &str = "id = 'check-fixture'\nversion = '1.0'\nname = 'Fixture check'\ndescription = 'Check'\nenabled = false\n[check]\ncommand = 'unused-check-command'\n";
const REVIEW_RULE: &str = "id = 'review-fixture'\nversion = '2.3'\ndescription = 'Review'\n[review]\nprompt = 'prompt.md'\n";
const KNOWLEDGE: &str = "purpose: Fixture\nproblem: Fixture problem\nvalue: Fixture value\nplacement: Fixture placement\nconditions:\n  - statement: Fixture condition\n    rationale: Fixture rationale\nextension: [true, preserved]\n";
const SET_KNOWLEDGE: &str = "purpose: Fixture\naudience: Maintainers\ncommonality: Repository checks\nrules:\n  - rule: check-fixture\n    role: Checks inputs\nadmission: [Checks repository inputs]\nexecution_points: Check\n";
const LOCAL_SET_PATH: &str = ".please/rule-sets/local.toml";
const LOCAL_SET: &str = "id = 'local'\nname = 'Local checks'\ndescription = 'The local set'\nrun_at = ['check']\nrules = ['check-fixture']\n";
const PROMPT: &str = "Inspect the project.\n";

fn git(root: &Path, args: &[&str]) {
    let result = support::command("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
}

fn cli(root: &Path, args: &[&str], key: Option<&str>) -> Output {
    let mut command = cli_command(root, args);
    if let Some(key) = key {
        command.env(KEY_NAME, key);
    }
    command.output().unwrap()
}

fn cli_command(root: &Path, args: &[&str]) -> std::process::Command {
    let mut command = support::command(env!("CARGO_BIN_EXE_please"));
    command
        .current_dir(root)
        .args(args)
        .env("PLEASE_CHANGE_BASE", "HEAD")
        .env("NO_PROXY", "*")
        .env("no_proxy", "*")
        .env_remove("ACTIONS_ID_TOKEN_REQUEST_URL")
        .env_remove("ACTIONS_ID_TOKEN_REQUEST_TOKEN");
    command
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
    exchange_delivery(
        listener,
        root,
        status,
        &["publish"],
        "/v0/attestations",
        |_| KEY.as_bytes().to_vec(),
    )
}

fn exchange_delivery(
    listener: &TcpListener,
    root: &Path,
    status: u16,
    command: &[&str],
    endpoint: &'static str,
    reply: impl FnOnce(&Value) -> Vec<u8> + Send + 'static,
) -> (Output, Value) {
    let server = listener.try_clone().unwrap();
    server.set_nonblocking(true).unwrap();
    let request = thread::spawn(move || answer_delivery(&server, status, endpoint, reply));
    let url = gateway_url(listener);
    let mut args = command.to_vec();
    args.extend(["--gateway-url", &url]);
    let output = cli(root, &args, Some(KEY));
    let body = request.join().unwrap();
    assert_no_request(listener);
    (output, body)
}

fn answer_delivery(
    server: &TcpListener,
    status: u16,
    endpoint: &str,
    reply: impl FnOnce(&Value) -> Vec<u8>,
) -> Value {
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
    assert_eq!(line, format!("POST {endpoint} HTTP/1.1\r\n"));
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
    let document = serde_json::from_slice(&body).unwrap();
    let response_body = reply(&document);
    // Even adversarial response bodies and redirect locations must stay out of diagnostics.
    let redirect = format!(
        "Location: http://{}/redirect\r\n",
        server.local_addr().unwrap()
    );
    write!(
        stream,
        "HTTP/1.1 {status} Test\r\n{redirect}Content-Length: {}\r\nConnection: close\r\n\r\n",
        response_body.len()
    )
    .unwrap();
    stream.write_all(&response_body).unwrap();
    document
}

fn assert_no_request(listener: &TcpListener) {
    listener.set_nonblocking(true).unwrap();
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

#[test]
// REQ-0364, REQ-0365: delivery uses HEAD or the exact supplied object ID and checks the receipt.
fn records_release_facts_without_a_saved_report() {
    let listener = listen();
    let root = fixture(None);
    git(
        root.path(),
        &["remote", "add", "origin", "git@Example.org:Owner/Repo.git"],
    );
    support::commit_all(root.path());
    let head = support::command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap();
    let explicit = "Ab".repeat(32);
    let version = " Train 7/β ";

    for (commit, status, receipt, code, message) in [
        (None, 201, "created", 0, "Release recorded."),
        (None, 200, "unchanged", 0, "Release already known."),
        (
            Some(explicit.as_str()),
            409,
            "conflict",
            2,
            "conflicts with a recorded commit",
        ),
    ] {
        let mut args = vec!["record", "release", "--version", version];
        if let Some(commit) = commit {
            args.extend(["--commit", commit]);
        }
        let (output, document) = exchange_delivery(
            &listener,
            root.path(),
            status,
            &args,
            "/v0/facts/releases",
            move |_| serde_json::to_vec(&json!({"outcome": receipt})).unwrap(),
        );
        assert_output(&output, code, message);
        assert_eq!(
            document,
            json!({
                "repository": "example.org/Owner/Repo",
                "version": version,
                "commit": commit.unwrap_or_else(|| head.trim()),
            })
        );
    }
    assert!(report::saved(root.path()).unwrap().is_empty());
}

#[test]
// REQ-0364, REQ-0365: validate local prerequisites and distinguish receipt and transport failures.
fn release_errors_use_publish_exit_codes() {
    let listener = listen();
    let url = gateway_url(&listener);
    let root = fixture(Some("example.org/Configured/Repo"));
    let args = [
        "record",
        "release",
        "--version",
        "Build",
        "--gateway-url",
        &url,
    ];
    for invalid in ["HEAD", "0123456789", &"g".repeat(40)] {
        let mut with_commit = args.to_vec();
        with_commit.extend(["--commit", invalid]);
        assert_output(
            &cli(root.path(), &with_commit, Some(KEY)),
            2,
            "--commit must be a full Git object ID",
        );
    }
    assert_output(
        &cli(root.path(), &args, Some(KEY)),
        2,
        "requires a current Git commit",
    );
    assert_no_request(&listener);

    let commit = "ab".repeat(20);
    let mut args = args.to_vec();
    args.extend(["--commit", &commit]);
    assert_output(
        &cli(root.path(), &args, None),
        2,
        "API key environment variable",
    );
    assert_no_request(&listener);

    for (status, receipt, message) in [
        (201, r#"{"outcome":"unchanged"}"#, "invalid receipt"),
        (409, KEY, "invalid receipt"),
        (302, KEY, "HTTP 302"),
        (503, KEY, "HTTP 503"),
    ] {
        let (output, document) = exchange_delivery(
            &listener,
            root.path(),
            status,
            &[
                "record",
                "release",
                "--version",
                "Build",
                "--commit",
                &commit,
            ],
            "/v0/facts/releases",
            move |_| receipt.as_bytes().to_vec(),
        );
        assert_output(&output, 2, message);
        assert_eq!(document["repository"], "example.org/Configured/Repo");
    }

    drop(listener);
    assert_output(
        &cli(root.path(), &args, Some(KEY)),
        2,
        "could not reach the release fact gateway",
    );
    support::write(root.path(), ".please/config.toml", "project = 'Fixture'\n");
    assert_output(
        &cli(root.path(), &args, Some(KEY)),
        2,
        "configure [publish]",
    );
}

#[test]
// REQ-0349, REQ-0358: gateway commands select OIDC without an API key and require HTTPS.
fn oidc_delivery_requires_workflow_permission_and_a_secure_token_endpoint() {
    const URL_ENV: &str = "ACTIONS_ID_TOKEN_REQUEST_URL";
    const TOKEN_ENV: &str = "ACTIONS_ID_TOKEN_REQUEST_TOKEN";

    let listener = listen();
    let url = gateway_url(&listener);
    let root = registration_fixture();
    let project = root.path();
    let config_path = project.join(".please/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        config_path,
        config.replace(&format!("key_env = '{KEY_NAME}'"), "auth = 'github-oidc'"),
    )
    .unwrap();
    support::commit_all(project);
    run(project, 0);

    for command in [
        &["publish"][..],
        &["register"],
        &["record", "release", "--version", "workflow-release"],
    ] {
        let mut args = command.to_vec();
        args.extend(["--gateway-url", &url]);
        for (provided, missing) in [(TOKEN_ENV, URL_ENV), (URL_ENV, TOKEN_ENV)] {
            let output = cli_command(project, &args)
                .env(provided, &url)
                .output()
                .unwrap();
            assert_output(&output, 2, "id-token: write");
            assert_output(&output, 2, missing);
        }
        let output = cli_command(project, &args)
            .env(URL_ENV, format!("{url}/{KEY}"))
            .env(TOKEN_ENV, KEY)
            .output()
            .unwrap();
        assert_output(&output, 2, "GitHub OIDC request URL must be an HTTPS URL");
        assert_no_request(&listener);
    }
}

fn registration_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let project = root.path();
    git(project, &["init", "--quiet"]);
    git(
        project,
        &[
            "remote",
            "add",
            "origin",
            "git@different.example.invalid:owner/repo.git",
        ],
    );
    support::write(
        project,
        ".please/config.toml",
        &format!(
            "project = 'Fixture'\n[publish]\nkey_env = '{KEY_NAME}'\nrepository = '{REGISTRATION_REPOSITORY}'\n"
        ),
    );
    for (directory, definition) in [
        (CHECK_DIRECTORY, CHECK_RULE),
        (REVIEW_DIRECTORY, REVIEW_RULE),
    ] {
        support::write(project, &format!("{directory}/rule.toml"), definition);
        support::write(project, &format!("{directory}/rule.yaml"), KNOWLEDGE);
    }
    support::write(project, &format!("{REVIEW_DIRECTORY}/prompt.md"), PROMPT);
    support::commit_all(project);
    root
}

fn registration_digest(root: &Path, directory: &str) -> String {
    let git = format!("https://{REGISTRATION_REPOSITORY}.git");
    please::rule_digest::from_directory(
        please::rule_digest::RuleSource::Imported { git: &git },
        root,
        Path::new(directory),
    )
    .unwrap()
    .unwrap()
}

fn registration_exchange(
    listener: &TcpListener,
    root: &Path,
    status: u16,
    response: Value,
) -> (Output, Value) {
    exchange_delivery(
        listener,
        root,
        status,
        &["register"],
        "/v0/rule-registrations",
        move |_| serde_json::to_vec(&response).unwrap(),
    )
}

#[test]
// REQ-0360, REQ-0361: submit rules first, reuse their digests and report each set's outcome.
fn registers_local_and_shareable_rule_sets_after_their_rules() {
    let root = registration_fixture();
    let project = root.path();
    support::write(
        project,
        LOCAL_SET_PATH,
        &format!("{LOCAL_SET}version = '3.1'\n"),
    );
    support::write(project, ".please/rule-sets/ruleset.yaml", SET_KNOWLEDGE);
    support::write(
        project,
        "ruleset.toml",
        &format!(
            "id = 'shared'\nversion = '1.4'\ndescription = 'Shared rules'\nrun_at = ['check']\nrules = ['{REVIEW_DIRECTORY}', {{ id = './{CHECK_DIRECTORY}', run_at = ['pre-push'] }}, '{CHECK_DIRECTORY}']\n"
        ),
    );
    support::write(
        project,
        "ruleset.yaml",
        &format!("{SET_KNOWLEDGE}  # comments stay local\n"),
    );
    support::commit_all(project);

    let check = registration_digest(project, CHECK_DIRECTORY);
    let review = registration_digest(project, REVIEW_DIRECTORY);
    let set_digest = format!("v1:sha256:{}", "a".repeat(64));
    for (outcomes, code, messages) in [
        (
            json!([
                {"outcome": "created", "digest": set_digest},
                {"outcome": "unchanged", "digest": set_digest},
            ]),
            0,
            vec![
                "created Rule Set local 3.1",
                "unchanged Rule Set shared 1.4",
            ],
        ),
        (
            json!([
                {"outcome": "conflict", "digest": set_digest},
                {"outcome": "invalid_members", "failures": [
                    {"member_index": 0, "reason": "digest_mismatch"},
                    {"member_index": 1, "reason": "unregistered"},
                ]},
            ]),
            2,
            vec![
                "conflict Rule Set local 3.1",
                "invalid_members Rule Set shared 1.4",
                &format!(
                    "member review-fixture 2.3 ({REVIEW_DIRECTORY}): rule digest does not match"
                ),
                &format!(
                    "member check-fixture 1.0 ({CHECK_DIRECTORY}): rule version is unregistered"
                ),
            ],
        ),
        (
            json!([
                {"outcome": "created", "digest": set_digest},
                {"outcome": "invalid_members", "failures": [
                    {"member_index": 2, "reason": "digest_mismatch"},
                ]},
            ]),
            2,
            vec!["registration gateway returned an invalid receipt"],
        ),
    ] {
        let listener = listen();
        let server = listener.try_clone().unwrap();
        server.set_nonblocking(true).unwrap();
        let rule_outcomes = json!({"outcomes": [
            {"outcome": "created", "digest": check},
            {"outcome": "created", "digest": review},
        ]});
        let worker = thread::spawn(move || {
            let rules = answer_delivery(&server, 200, "/v0/rule-registrations", |_| {
                serde_json::to_vec(&rule_outcomes).unwrap()
            });
            let sets = answer_delivery(&server, 200, "/v0/rule-set-registrations", |_| {
                serde_json::to_vec(&json!({"outcomes": outcomes})).unwrap()
            });
            (rules, sets)
        });
        let output = cli(
            project,
            &["register", "--gateway-url", &gateway_url(&listener)],
            Some(KEY),
        );
        let (rules, sets) = worker.join().unwrap();
        assert_no_request(&listener);
        for message in messages {
            assert_output(&output, code, message);
        }
        let knowledge = json!({
            "purpose": "Fixture", "audience": "Maintainers", "commonality": "Repository checks",
            "rules": [{"rule": "check-fixture", "role": "Checks inputs"}],
            "admission": ["Checks repository inputs"], "execution_points": "Check",
        });
        let check_member = json!({
            "source": REGISTRATION_REPOSITORY, "path": CHECK_DIRECTORY, "id": "check-fixture",
            "version": "1.0", "digest": check, "enabled": false,
        });
        let review_member = json!({
            "source": REGISTRATION_REPOSITORY, "path": REVIEW_DIRECTORY, "id": "review-fixture",
            "version": "2.3", "digest": review, "enabled": true,
        });
        assert_eq!(
            sets,
            json!({"commit": rules["commit"], "sets": [
                {"source": REGISTRATION_REPOSITORY, "path": ".please/rule-sets", "id": "local",
                 "version": "3.1", "name": "Local checks", "description": "The local set",
                 "knowledge": knowledge, "members": [check_member]},
                {"source": REGISTRATION_REPOSITORY, "path": ".", "id": "shared",
                 "version": "1.4", "description": "Shared rules", "knowledge": knowledge,
                 "members": [review_member, check_member]},
            ]})
        );
    }
}

#[test]
// REQ-0360: missing set inputs fail preparation even when rules could be registered.
fn rejects_missing_rule_set_version_and_knowledge_before_sending() {
    let listener = listen();
    let root = registration_fixture();
    let project = root.path();
    let url = gateway_url(&listener);
    let invoke = || cli(project, &["register", "--gateway-url", &url], Some(KEY));
    support::write(project, LOCAL_SET_PATH, LOCAL_SET);
    support::commit_all(project);
    assert_output(&invoke(), 2, "Rule Set local is missing version");
    assert_no_request(&listener);

    support::write(
        project,
        LOCAL_SET_PATH,
        &format!("{LOCAL_SET}version = '3.1'\n"),
    );
    support::commit_all(project);
    assert_output(
        &invoke(),
        2,
        "Rule Set local requires a committed ruleset.yaml",
    );
    assert_no_request(&listener);

    support::write(project, ".please/rule-sets/ruleset.yaml", SET_KNOWLEDGE);
    support::commit_all(project);
    support::write(
        project,
        ".please/rule-sets/ruleset.yaml",
        "purpose: Uncommitted\n",
    );
    assert_output(&invoke(), 2, "ruleset.yaml differs from the current commit");
    assert_no_request(&listener);
}

#[test]
// REQ-0355, REQ-0356, REQ-0357: explicitly send one committed batch and process independent outcomes.
fn registers_committed_checks_and_reviews_and_reports_repeated_and_conflicting_versions() {
    let listener = listen();
    let root = registration_fixture();
    let project = root.path();
    let commit = support::command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project)
        .output()
        .unwrap();
    assert!(commit.status.success());
    let commit = String::from_utf8(commit.stdout)
        .unwrap()
        .trim_end()
        .to_owned();
    // An unrelated working change does not alter the registered content.
    support::write(project, "unrelated", "uncommitted\n");

    let check_digest = registration_digest(project, CHECK_DIRECTORY);
    let review_digest = registration_digest(project, REVIEW_DIRECTORY);
    let knowledge = json!({
        "purpose": "Fixture",
        "problem": "Fixture problem",
        "value": "Fixture value",
        "placement": "Fixture placement",
        "conditions": [{"statement": "Fixture condition", "rationale": "Fixture rationale"}],
        "extension": [true, "preserved"]
    });
    let expected = json!({
        "commit": commit,
        "rules": [
            {"source": REGISTRATION_REPOSITORY, "path": CHECK_DIRECTORY, "id": "check-fixture",
             "version": "1.0", "kind": "check", "name": "Fixture check",
             "description": "Check", "knowledge": knowledge},
            {"source": REGISTRATION_REPOSITORY, "path": REVIEW_DIRECTORY, "id": "review-fixture",
             "version": "2.3", "kind": "review", "description": "Review",
             "blocking": true, "prompt": PROMPT, "knowledge": knowledge}
        ]
    });
    for state in ["created", "unchanged"] {
        let (output, body) = registration_exchange(
            &listener,
            project,
            200,
            json!({"outcomes": [
                {"outcome": state, "digest": check_digest},
                {"outcome": state, "digest": review_digest}
            ]}),
        );
        assert_output(&output, 0, &format!("{state} check-fixture 1.0"));
        assert_output(&output, 0, &format!("{state} review-fixture 2.3"));
        assert_eq!(body, expected);
    }

    support::write(
        project,
        &format!("{CHECK_DIRECTORY}/rule.toml"),
        &CHECK_RULE.replace("'Check'", "'Updated check'"),
    );
    support::commit_all(project);
    let changed = registration_digest(project, CHECK_DIRECTORY);
    assert_ne!(changed, check_digest);
    let (output, body) = registration_exchange(
        &listener,
        project,
        200,
        json!({"outcomes": [
            {"outcome": "conflict", "digest": changed},
            {"outcome": "unchanged", "digest": review_digest}
        ]}),
    );
    assert_ne!(body["commit"], expected["commit"]);
    assert_eq!(body["rules"][0]["description"], "Updated check");
    assert_eq!(body["rules"][1], expected["rules"][1]);
    assert_output(&output, 2, "content changed without a new version");
    assert_output(&output, 2, "unchanged review-fixture 2.3");
    assert!(!project.join(".please/reports").exists());
    assert!(!project.join(".git/hooks/pre-push").exists());
}

#[test]
// REQ-0356: any missing metadata or dirty rule input blocks the complete request.
fn rejects_missing_metadata_and_changes_before_sending() {
    let listener = listen();
    let url = gateway_url(&listener);
    let root = registration_fixture();
    let project = root.path();
    let invoke = || cli(project, &["register", "--gateway-url", &url], Some(KEY));
    support::write(
        project,
        &format!("{REVIEW_DIRECTORY}/prompt.md"),
        "Uncommitted prompt\n",
    );
    assert_output(&invoke(), 2, "prompt.md differs from the current commit");
    assert_no_request(&listener);
    support::write(project, &format!("{REVIEW_DIRECTORY}/prompt.md"), PROMPT);
    support::write(
        project,
        &format!("{REVIEW_DIRECTORY}/script.sh"),
        "untracked\n",
    );
    assert_output(&invoke(), 2, "script.sh differs from the current commit");
    assert_no_request(&listener);
    fs::remove_file(project.join(REVIEW_DIRECTORY).join("script.sh")).unwrap();

    support::write(
        project,
        &format!("{REVIEW_DIRECTORY}/rule.toml"),
        &REVIEW_RULE.replace("version = '2.3'\n", ""),
    );
    support::commit_all(project);
    assert_output(&invoke(), 2, "rule review-fixture is missing version");
    assert_no_request(&listener);
    support::write(
        project,
        &format!("{REVIEW_DIRECTORY}/rule.toml"),
        REVIEW_RULE,
    );
    fs::remove_file(project.join(REVIEW_DIRECTORY).join("rule.yaml")).unwrap();
    support::commit_all(project);
    assert_output(
        &invoke(),
        2,
        "rule review-fixture requires a committed rule.yaml",
    );
    assert_no_request(&listener);
}

#[test]
// REQ-0355, REQ-0357: delivery failures and receipt failures are distinct and redact external data.
fn registration_errors_do_not_expose_credentials_or_follow_redirects() {
    let listener = listen();
    let root = registration_fixture();
    let project = root.path();
    let url = gateway_url(&listener);
    assert_output(&cli(project, &["register"], Some(KEY)), 2, "--gateway-url");
    let invalid = format!("{url}/{KEY}");
    assert_output(
        &cli(project, &["register", "--gateway-url", &invalid], Some(KEY)),
        2,
        "must be an HTTP or HTTPS origin",
    );
    assert_output(
        &cli(project, &["register", "--gateway-url", &url], None),
        2,
        "API key environment variable is unset",
    );
    assert_no_request(&listener);
    for (status, message) in [
        (400, "rejected the registration document"),
        (401, "rejected the API credentials"),
        (307, "registration gateway error (HTTP 307)"),
    ] {
        let (output, _) = registration_exchange(&listener, project, status, json!(KEY));
        assert_output(&output, 2, message);
    }
    let review_digest = registration_digest(project, REVIEW_DIRECTORY);
    let (output, _) = registration_exchange(
        &listener,
        project,
        200,
        json!({"outcomes": [
            {"outcome": "created", "digest": KEY},
            {"outcome": "created", "digest": review_digest}
        ]}),
    );
    assert_output(&output, 2, "returned digest does not match");
    let (output, _) = registration_exchange(&listener, project, 200, json!({"outcomes": []}));
    assert_output(&output, 2, "returned an invalid receipt");

    drop(listener);
    assert_output(
        &cli(project, &["register", "--gateway-url", &url], Some(KEY)),
        2,
        "could not reach the registration gateway",
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
// REQ-0345, REQ-0346, REQ-0363: deliver the newest failed run with no Rule Set field when no sets were bound.
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
                {"id": "failing", "kind": "check", "status": "FAIL", "rule": {
                    "source": "example.invalid/other/repo", "path": ".please/checks/failing", "id": "failing"
                }},
                {"id": "passing", "kind": "check", "status": "PASS", "rule": {
                    "source": "example.invalid/other/repo", "path": ".please/checks/passing", "id": "passing"
                }},
                {"id": "advisory", "kind": "review", "status": "NOT_RUN", "rule": {
                    "source": "example.invalid/other/repo", "path": ".please/reviews/advisory", "id": "advisory"
                }},
            ],
        })
    );

    // Model a retained transition report with outcomes that are cumbersome to trigger together.
    report.schema_version = 1;
    for rule in &mut report.plan.scheduled {
        rule.rule_metadata = None;
    }
    for result in &mut report.results {
        result.rule_metadata = None;
    }
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

#[cfg(unix)]
#[test]
// REQ-0354, REQ-0346, REQ-0363: publish saved rule metadata and versioned local and imported set bindings.
fn publishes_versioned_import_metadata_with_the_original_run_ids() {
    use please::rule_digest::{self, RuleSource};

    const SELECTOR: &str = "https://Rules.example.invalid:8443/Owner/Rules.git";
    const DIRECTORY: &str = "sets/core/checks/source-directory";
    let root = fixture(None);
    let remote = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--quiet", "--initial-branch=main"]);
    support::write(
        remote.path(),
        "sets/core/ruleset.toml",
        &format!(
            "id = 'core'\nversion = '3.4'\ndescription = 'Fixture'\nrules = ['{DIRECTORY}']\nrun_at = ['validate']\n"
        ),
    );
    support::write(
        remote.path(),
        &format!("{DIRECTORY}/rule.toml"),
        "id = 'imported'\nversion = '2.17'\ndescription = 'Fixture'\n[check]\ncommand = 'git --version'\n",
    );
    support::write(
        remote.path(),
        &format!("{DIRECTORY}/rule.yaml"),
        "purpose: 'Fixture'\n",
    );
    support::commit_all(remote.path());
    let digest = rule_digest::from_directory(
        RuleSource::Imported { git: SELECTOR },
        remote.path(),
        Path::new(DIRECTORY),
    )
    .unwrap()
    .unwrap();

    let project = root.path();
    git(
        project,
        &[
            "remote",
            "add",
            "origin",
            "git@Project.example.invalid:Owner/Project.git",
        ],
    );
    support::check(project, "unversioned", &["verify"], "exit 0\n");
    support::write(
        project,
        LOCAL_SET_PATH,
        "id = 'local'\nversion = '1.2'\ndescription = 'Fixture'\nrun_at = ['pre-push']\nrules = [{ id = 'unversioned', run_at = ['verify'] }]\n",
    );
    support::write(
        project,
        ".please/rule-sets/other.toml",
        "id = 'other'\nversion = '1.0'\ndescription = 'Fixture'\nrun_at = ['pre-push']\nrules = ['unversioned']\n",
    );
    support::write(
        project,
        ".please/checks/no-knowledge/rule.toml",
        "id = 'no-knowledge'\nversion = '1.0'\ndescription = 'Fixture'\nrun_at = ['verify']\n[check]\ncommand = 'git --version'\n",
    );
    let file_selector = format!("file://{}", remote.path().display());
    let config_path = project.join(".please/config.toml");
    let mut config = fs::read_to_string(&config_path)
        .unwrap()
        .replace("\nimports = []\n", "\n")
        .replace("\ncustom = []\n", "\ncustom = ['verify']\n");
    for selector in [SELECTOR, &file_selector] {
        support::trust_location(selector);
        config.push_str(&format!(
            "\n[[imports]]\ngit = {selector:?}\npath = 'sets/core'\nref = 'main'\nexecution_points = {{ validate = 'verify' }}\n"
        ));
    }
    fs::write(config_path, config).unwrap();
    support::commit_all(project);

    let invoke = |args: &[&str]| {
        cli_command(project, args)
            .env("XDG_CACHE_HOME", cache.path())
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", format!("url.{file_selector}.insteadOf"))
            .env("GIT_CONFIG_VALUE_0", SELECTOR)
            .output()
            .unwrap()
    };
    let sync = invoke(&["sync", "--yes"]);
    assert!(sync.status.success(), "{sync:?}");
    let preview = invoke(&["run", "--at", "verify", "--plan", "--json"]);
    assert!(preview.status.success(), "{preview:?}");
    let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
    support::assert_plan_contract(&plan);
    let output = invoke(&["run", "--at", "verify"]);
    assert!(output.status.success(), "{output:?}");
    let mut saved = report::read(project).unwrap();
    assert_eq!(
        plan["scheduled"],
        serde_json::to_value(&saved.plan.scheduled).unwrap()
    );

    let published = json!({
        "source": "rules.example.invalid/Owner/Rules",
        "path": DIRECTORY,
        "id": "imported",
        "version": "2.17",
        "digest": digest,
    });
    let mut expected_results = Vec::new();
    for item in &saved.results {
        let metadata = serde_json::to_value(item.rule_metadata.as_ref().unwrap()).unwrap();
        let mut result = json!({"id": item.id, "kind": "check", "status": "PASS"});
        match item.id.as_str() {
            "unversioned" | "no-knowledge" => {
                let identity = json!({
                    "source": "project.example.invalid/Owner/Project",
                    "path": format!(".please/checks/{}", item.id),
                    "id": item.id,
                });
                let mut expected = identity.clone();
                if item.id == "no-knowledge" {
                    expected["version"] = json!("1.0");
                }
                assert_eq!(metadata, expected);
                result["rule"] = identity;
            }
            id => {
                assert!(
                    id.starts_with("git:") && id.ends_with(":sets/core:1:imported"),
                    "{id}"
                );
                if metadata["source"].is_null() {
                    assert_eq!(
                        metadata,
                        json!({"path": DIRECTORY, "id": "imported", "version": "2.17"})
                    );
                } else {
                    assert_eq!(metadata, published);
                    result["rule"] = published.clone();
                }
            }
        }
        expected_results.push(result);
    }
    assert_eq!(expected_results.len(), 4);
    assert_eq!(
        expected_results
            .iter()
            .filter(|result| result["rule"] == published)
            .count(),
        1
    );
    saved.results.push(report::Item::simple(
        "please-tree-stability",
        "integrity",
        Status::Error,
        "Fixture integrity error".into(),
    ));
    saved.overall = Status::Error;
    expected_results.push(json!({
        "id": "please-tree-stability", "kind": "integrity", "status": "ERROR",
    }));
    let document = serde_json::to_value(&saved).unwrap();
    support::assert_report_contract(&document);
    fs::write(
        project.join(".please/reports/1.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();

    // Publication uses the persisted rule source even if the Git origin changed.
    git(
        project,
        &[
            "remote",
            "set-url",
            "origin",
            "git@new.example.invalid:Owner/Project.git",
        ],
    );
    let (output, attestation) = exchange(&listen(), project, 201);
    assert_output(&output, 0, "Attestation accepted");
    assert_eq!(
        attestation["repository"],
        "new.example.invalid/Owner/Project"
    );
    assert_eq!(attestation["results"], json!(expected_results));
    assert_eq!(
        attestation["rule_sets"],
        json!([
            {
                "source": "project.example.invalid/Owner/Project",
                "path": ".please/rule-sets",
                "id": "local",
                "version": "1.2",
                "execution_points": ["verify"],
            },
            {
                "source": "rules.example.invalid/Owner/Rules",
                "path": "sets/core",
                "id": "core",
                "version": "3.4",
                "execution_points": ["verify"],
            },
        ])
    );
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
