//! End-to-end CLI checks use fake local credentials and a loopback provider only.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    ops::{Deref, DerefMut},
    path::Path,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

struct TestCommand {
    command: Command,
    _cache: tempfile::TempDir,
}

impl Deref for TestCommand {
    type Target = Command;
    fn deref(&self) -> &Self::Target {
        &self.command
    }
}

impl DerefMut for TestCommand {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.command
    }
}

fn command(cwd: &Path) -> TestCommand {
    let cache = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_oko"));
    command
        .current_dir(cwd)
        .env("OKO_CACHE_DIR", cache.path())
        .env("OKO_NO_CACHE", "0")
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("TYPESAFE_DEFAULT_MODEL")
        .env("TYPESAFE_BASE_URL", "http://127.0.0.1:1");
    TestCommand {
        command,
        _cache: cache,
    }
}

#[test]
fn ask_reuses_preparation_across_processes_and_refreshes_changed_source() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let source = root.path().join("archive.rs");
    fs::write(&source, "fn verify_archive_checksum() {}\n").unwrap();
    let run = || {
        success(
            command(root.path())
                .env("OKO_CACHE_DIR", cache.path())
                .args(["ask", "--no-jev", "--json", "archive checksum"])
                .output()
                .unwrap(),
        )
    };
    let cold = run();
    assert_eq!(cold["cache"]["status"], "cold");
    assert_eq!(cold["cache"]["rebuiltFiles"], 1);
    let restarted = run();
    assert_eq!(restarted["cache"]["status"], "disk");
    assert_eq!(restarted["cache"]["rebuiltFiles"], 0);
    assert_eq!(restarted["cache"]["reusedFiles"], 1);
    assert_eq!(cold["results"], restarted["results"]);
    fs::write(&source, "fn repair_archive_checksum() {}\n").unwrap();
    let changed = run();
    assert_eq!(changed["cache"]["rebuiltFiles"], 1);
    assert!(
        changed["results"][0]["text"]
            .as_str()
            .unwrap()
            .contains("repair_archive")
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn mock_run(
    command: &mut Command,
    response: impl FnOnce(&Value) -> Value + Send + 'static,
) -> (Output, String, Value) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    command.env(
        "TYPESAFE_BASE_URL",
        format!("http://{}", listener.local_addr().unwrap()),
    );
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(error) => panic!("mock did not receive request: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        let (headers, body) = loop {
            let length = stream.read(&mut buffer).unwrap();
            assert!(length > 0);
            bytes.extend_from_slice(&buffer[..length]);
            if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break (
                        headers,
                        serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length]).unwrap(),
                    );
                }
            }
        };
        let response = serde_json::to_vec(&response(&body)).unwrap();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
        stream.write_all(&response).unwrap();
        (headers, body)
    });
    let output = command.output().unwrap();
    let (headers, body) = server.join().unwrap();
    (output, headers, body)
}

fn input(cwd: &Path) {
    fs::write(
        cwd.join("items.json"),
        r#"[{"id":"ticket","text":"Refund request","source":"tickets/1"}]"#,
    )
    .unwrap();
}

fn relevance_response(request: &Value, score: impl Fn(&Value) -> f64) -> Value {
    let answers = request["state"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| {
            let label = candidate["candidate"].as_str().unwrap();
            assert_eq!(request["questions"][label]["type"], "noul");
            (
                label.to_owned(),
                json!({"type":"noul", "noul":score(candidate)}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    assert_eq!(
        request["questions"].as_object().unwrap().len(),
        answers.len()
    );
    json!({"answers":answers})
}

#[test]
fn rank_reads_current_directory_env_and_shell_has_precedence() {
    let temp = tempfile::tempdir().unwrap();
    input(temp.path());
    fs::write(temp.path().join(".env"), "# Local fake credentials\nexport TYPESAFE_API_KEY = \" file-key \"\nTYPESAFE_DEFAULT_MODEL=should-not-override-process\n").unwrap();
    for key in [None, Some(" shell-key ")] {
        let mut cmd = command(temp.path());
        cmd.args(["rank", "--input", "items.json", "--json", "refund"]);
        if let Some(key) = key {
            cmd.env("TYPESAFE_API_KEY", key);
        }
        let (output, headers, body) =
            mock_run(&mut cmd, |request| relevance_response(request, |_| 0.9));
        assert!(headers.contains(&format!(
            "authorization: bearer {}\r\n",
            if key.is_some() {
                "shell-key"
            } else {
                "file-key"
            }
        )));
        assert_eq!(body["model"], "jev-latest");
        let json = success(output);
        assert_eq!(json["ranking"], "jev");
        assert_eq!(json["omittedCount"], 0);
        assert_eq!(
            json["results"][0],
            json!({"id":"ticket","text":"Refund request","source":"tickets/1","score":0.9})
        );
    }
    let output = command(temp.path())
        .env("TYPESAFE_API_KEY", "")
        .args(["rank", "--input", "items.json", "refund"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("TYPESAFE_API_KEY is required"));
    assert!(output.stdout.is_empty());
}

#[test]
fn command_errors_and_explicit_offline_mode() {
    let temp = tempfile::tempdir().unwrap();
    input(temp.path());
    for args in [
        vec!["rank", "--input", "items.json", "refund"],
        vec!["ask", "--unknown", "question"],
        vec!["rank", "question"],
    ] {
        let output = command(temp.path())
            .env("TYPESAFE_API_KEY", "")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).starts_with("Error:"));
        assert!(output.stdout.is_empty());
    }
    let result = success(
        command(temp.path())
            .args([
                "rank",
                "--input",
                "items.json",
                "--json",
                "--no-jev",
                "refund",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(result["ranking"], "input");
    assert_eq!(result["results"][0]["score"], 0.0);
}

#[test]
fn invalid_provider_scores_fail_instead_of_succeeding() {
    let temp = tempfile::tempdir().unwrap();
    input(temp.path());
    for answer in [
        json!({}),
        json!({"type":"noul"}),
        json!({"type":"noul", "noul":"0.9"}),
        json!({"type":"noul", "noul":-0.1}),
        json!({"type":"noul", "noul":1.1}),
    ] {
        let mut cmd = command(temp.path());
        cmd.env("TYPESAFE_API_KEY", "fake-key").args([
            "rank",
            "--input",
            "items.json",
            "--json",
            "refund",
        ]);
        let (output, _, _) = mock_run(&mut cmd, move |_| json!({"answers":{"candidate_1":answer}}));
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("candidate_1"));
    }
}

#[test]
fn ask_maps_provider_ids_to_code_and_breaks_ties_deterministically() {
    let temp = tempfile::tempdir().unwrap();
    for letter in 'a'..='g' {
        fs::write(
            temp.path().join(format!("{letter}.rs")),
            "fn authentication() {}\n",
        )
        .unwrap();
    }
    let mut cmd = command(temp.path());
    cmd.env("TYPESAFE_API_KEY", "fake-key")
        .args(["ask", "--json", "authentication"]);
    let (output, _, body) = mock_run(&mut cmd, |request| {
        let candidates = request["state"]["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 7);
        relevance_response(request, |candidate| {
            if candidate["source"].as_str().unwrap().starts_with("g.rs:") {
                0.9
            } else {
                0.8
            }
        })
    });
    assert_eq!(body["state"]["candidates"][0]["id"], "0");
    let result = success(output);
    assert_eq!(result["ranking"], "jev");
    let results = result["results"].as_array().unwrap();
    assert_eq!(
        results
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["g.rs", "a.rs", "b.rs", "c.rs", "d.rs"]
    );
    assert_eq!(results[0]["score"], 0.9);
    for item in results {
        assert_eq!(item["startLine"], 1);
        assert_eq!(item["endLine"], 1);
        assert_eq!(item["text"], "fn authentication() {}");
        assert!(item.get("id").is_none());
    }
}

#[test]
fn intent_defaults_and_overrides_use_one_ranking_request() {
    let temp = tempfile::tempdir().unwrap();
    input(temp.path());
    fs::write(temp.path().join("auth.rs"), "fn refund() {}\n").unwrap();
    for (mode, intent, expected) in [
        ("ask", None, "implement all or part of the behavior"),
        ("rank", None, "answer"),
        ("ask", Some("general"), "answer"),
        ("ask", Some("explanation"), "explain"),
        (
            "rank",
            Some("implementation"),
            "implement all or part of the behavior",
        ),
        ("rank", Some("explanation"), "explain"),
    ] {
        let mut cmd = command(temp.path());
        cmd.env("TYPESAFE_API_KEY", "fake-key")
            .args([mode, "refund", "--json"]);
        if mode == "rank" {
            cmd.args(["--input", "items.json"]);
        }
        if let Some(intent) = intent {
            cmd.args(["--intent", intent]);
        }
        let (output, _, body) = mock_run(&mut cmd, |request| relevance_response(request, |_| 0.9));
        assert!(!success(output)["results"].as_array().unwrap().is_empty());
        let instructions = body["questions"].to_string();
        assert!(instructions.contains(expected), "{instructions}");
        assert_eq!(body["state"]["question"], "refund");
    }
}

#[test]
fn invalid_intents_fail_and_offline_intents_need_no_network() {
    let temp = tempfile::tempdir().unwrap();
    input(temp.path());
    for flags in [
        vec!["--intent"],
        vec!["--intent", "unknown"],
        vec!["--intent", "general", "--intent", "general"],
    ] {
        let output = command(temp.path())
            .args(["rank", "refund", "--input", "items.json"])
            .args(flags)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("intent"));
    }
    for intent in ["general", "explanation", "implementation"] {
        let output = command(temp.path())
            .args([
                "rank",
                "refund",
                "--input",
                "items.json",
                "--no-jev",
                "--json",
                "--intent",
                intent,
            ])
            .output()
            .unwrap();
        let result = success(output);
        assert_eq!(result["ranking"], "input");
        assert_eq!(result["results"][0]["id"], "ticket");
        assert_eq!(result["results"][0]["score"].as_f64(), Some(0.0));
    }
}

#[test]
fn deep_mode_uses_existing_jev_key_and_reports_budget_stop() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("auth.rs"),
        "fn authenticate() { validate_token(); }\n",
    )
    .unwrap();
    fs::write(temp.path().join(".env"), "TYPESAFE_API_KEY=fake-deep-key\n").unwrap();
    let mut cmd = command(temp.path());
    cmd.args([
        "ask",
        "authenticate token",
        "--deep",
        "--max-steps",
        "1",
        "--json",
    ]);
    let (output, headers, request) =
        mock_run(&mut cmd, |request| relevance_response(request, |_| 0.9));
    assert!(headers.contains("authorization: bearer fake-deep-key"));
    assert_eq!(request["questions"]["candidate_1"]["type"], "noul");
    let value = success(output);
    assert_eq!(value["ranking"], "jev");
    assert_eq!(value["results"][0]["path"], "auth.rs");
    assert_eq!(value["investigation"]["steps"], 1);
    assert_eq!(value["investigation"]["jevCalls"], 1);
    assert_eq!(value["investigation"]["complete"], false);
    assert_eq!(value["investigation"]["stopReason"], "step_limit");
}

#[test]
fn deep_flags_reject_incompatible_or_ambiguous_limits() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["ask", "q", "--max-steps", "1"],
        vec!["ask", "q", "--deep", "--no-jev"],
        vec!["ask", "q", "--deep", "--max-steps", "0"],
        vec!["ask", "q", "--deep", "--max-steps", "-1"],
        vec!["ask", "q", "--deep", "--max-steps", "1", "--max-steps", "2"],
        vec!["rank", "q", "--input", "items.json", "--deep"],
    ] {
        assert!(
            !command(temp.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn auth_status_obeys_overrides_without_disclosing_keys() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(".env"), "TYPESAFE_API_KEY=file-secret\n").unwrap();
    for (shell, source, configured) in [
        (None, "current directory .env", true),
        (Some("shell-secret"), "environment", true),
        (Some(""), "environment", false),
    ] {
        let mut cmd = command(temp.path());
        if let Some(value) = shell {
            cmd.env("TYPESAFE_API_KEY", value);
        }
        let output = cmd.args(["auth", "status"]).output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains(source));
        assert_eq!(text.contains("not configured"), !configured);
        assert!(!text.contains("file-secret"));
        assert!(!text.contains("shell-secret"));
        assert!(output.stderr.is_empty());
    }
    fs::write(temp.path().join(".env"), "TYPESAFE_API_KEY=\n").unwrap();
    let output = command(temp.path())
        .args(["auth", "status"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("empty; lower-priority keys are not used")
    );
}

#[test]
fn auth_rejects_key_arguments_and_noninteractive_login() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["auth", "login", "fake-sensitive-key"],
        vec!["auth", "fake-sensitive-key"],
    ] {
        let output = command(temp.path()).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("fake-sensitive-key"));
        assert!(output.stdout.is_empty());
    }
    let output = command(temp.path())
        .args(["auth", "login"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interactive terminal"));
    let output = command(temp.path())
        .args(["auth", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("oko auth logout"));
}

#[test]
fn environment_and_offline_mode_bypass_unreadable_env_file() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(".env")).unwrap();
    input(temp.path());
    let output = command(temp.path())
        .env("TYPESAFE_API_KEY", "fake-secret")
        .args(["auth", "status"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("environment"));
    let output = command(temp.path())
        .args(["auth", "status"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Cannot read"));
    let output = command(temp.path())
        .args([
            "rank",
            "refund",
            "--input",
            "items.json",
            "--no-jev",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(success(output)["ranking"], "input");
}
