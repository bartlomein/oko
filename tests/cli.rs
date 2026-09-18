//! End-to-end CLI checks use fake local credentials and a loopback provider only.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

fn command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_oko"));
    command
        .current_dir(cwd)
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("TYPESAFE_DEFAULT_MODEL")
        .env("TYPESAFE_BASE_URL", "http://127.0.0.1:1");
    command
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

fn probabilities() -> Value {
    json!({"answers":{"selection":{"probabilities":{"none":0.1,"candidate_1":0.9}}}})
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
        let (output, headers, body) = mock_run(&mut cmd, |_| probabilities());
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
        let output = command(temp.path()).args(args).output().unwrap();
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
fn invalid_provider_probabilities_fail_instead_of_succeeding() {
    let temp = tempfile::tempdir().unwrap();
    input(temp.path());
    let mut cmd = command(temp.path());
    cmd.env("TYPESAFE_API_KEY", "fake-key").args([
        "rank",
        "--input",
        "items.json",
        "--json",
        "refund",
    ]);
    let (output, _, _) = mock_run(
        &mut cmd,
        |_| json!({"answers":{"selection":{"probabilities":{"none":0.1}}}}),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("invalid or missing candidate probabilities")
    );
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
        let mut probabilities = serde_json::Map::new();
        probabilities.insert("none".into(), json!(0.1));
        for candidate in candidates {
            probabilities.insert(
                candidate["candidate"].as_str().unwrap().into(),
                json!(
                    if candidate["source"].as_str().unwrap().starts_with("g.rs:") {
                        0.9
                    } else {
                        0.5
                    }
                ),
            );
        }
        json!({"answers":{"selection":{"probabilities":probabilities}}})
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
