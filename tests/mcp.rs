//! Real stdio protocol tests; no real keys or credential-store access.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Value>,
    id: u64,
}
impl Client {
    fn start(root: &Path, offline: bool, endpoint: Option<&str>) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oko"));
        cmd.args(["mcp", "--root"])
            .arg(root)
            .env("TYPESAFE_API_KEY", "")
            .env_remove("TYPESAFE_DEFAULT_MODEL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if offline {
            cmd.arg("--no-jev");
        }
        if let Some(endpoint) = endpoint {
            cmd.env("TYPESAFE_API_KEY", "fake-mcp-key")
                .env("TYPESAFE_BASE_URL", endpoint);
        }
        let mut child = cmd.spawn().unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = serde_json::from_str(&line.unwrap())
                    .expect("stdout must contain only JSON-RPC");
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input: Some(input),
            output,
            id: 0,
        }
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input.as_mut().unwrap(), "{value}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0", "id":self.id, "method":method,"params":params}));
        loop {
            let response = self
                .output
                .recv_timeout(Duration::from_secs(15))
                .expect("MCP response timed out");
            if response.get("id") == Some(&json!(self.id)) {
                return response;
            }
        }
    }
    fn initialize(&mut self) {
        let response = self.request("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"oko-tests","version":"1"}}));
        assert!(response.get("error").is_none(), "{response}");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }
    fn search(&mut self, args: Value) -> Value {
        self.request("tools/call", json!({"name":"search","arguments":args}))
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("auth.rs"),
        "fn authenticate() { validate_token(); }\n",
    )
    .unwrap();
    temp
}
#[test]
fn stdio_handshake_schema_search_and_fresh_files() {
    let root = fixture();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "search");
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(tools[0]["inputSchema"]["additionalProperties"], false);
    let result = client.search(json!({"question":"authentication token"}));
    assert_eq!(result["result"]["isError"], false, "{result}");
    let data = &result["result"]["structuredContent"];
    assert_eq!(data["ranking"], "lexical");
    assert_eq!(data["results"][0]["path"], "auth.rs");
    assert_eq!(data["results"][0]["startLine"], 1);
    fs::write(
        root.path().join("auth.rs"),
        "fn changed_authenticate() {}\n",
    )
    .unwrap();
    let result = client.search(json!({"question":"changed authenticate"}));
    assert!(
        result["result"]["structuredContent"]["results"][0]["text"]
            .as_str()
            .unwrap()
            .contains("changed_authenticate")
    );
    assert!(client.request("ping", json!({})).get("error").is_none());
}
#[test]
fn invalid_arguments_boundaries_and_missing_key_are_recoverable() {
    let root = fixture();
    let outside = fixture();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    for args in [
        json!({"question":" "}),
        json!({"question":"x".repeat(4097)}),
        json!({"question":"auth","directory":outside.path()}),
        json!({"question":"auth","directory":".."}),
        json!({"question":"auth","max_steps":1}),
        json!({"question":"auth","deep":true,"max_steps":6}),
        json!({"question":"auth","deep":true}),
    ] {
        let result = client.search(args);
        assert_eq!(result["result"]["isError"], true, "{result}");
    }
    for args in [
        json!({}),
        json!({"question":"auth","intent":"bad"}),
        json!({"question":"auth","surprise":true}),
    ] {
        let result = client.search(args);
        assert!(
            result.get("error").is_some() || result["result"]["isError"] == true,
            "{result}"
        );
    }
    assert_eq!(
        client.search(json!({"question":"auth"}))["result"]["isError"],
        false
    );
    let mut paid = Client::start(root.path(), false, None);
    paid.initialize();
    let result = paid.search(json!({"question":"auth"}));
    assert_eq!(result["result"]["isError"], true);
    assert!(
        result["result"]["structuredContent"]["error"]
            .as_str()
            .unwrap()
            .contains("No TypeSafe key")
    );
}
#[cfg(unix)]
#[test]
fn symlink_directory_cannot_escape_workspace() {
    let root = fixture();
    let outside = fixture();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    assert_eq!(
        client.search(json!({"question":"auth","directory":"escape"}))["result"]["isError"],
        true
    );
}

#[test]
fn normal_and_deep_search_use_mock_jev_and_survive_provider_errors() {
    use std::{io::Read, net::TcpListener, time::Instant};
    for (deep, fail) in [(false, false), (true, false), (false, true)] {
        let root = fixture();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("Mock provider did not receive request: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let len: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + len {
                        assert!(headers.contains("authorization: bearer fake-mcp-key"));
                        break;
                    }
                }
            }
            let body = if fail {
                "fake-mcp-key must never be exposed".to_owned()
            } else {
                json!({"answers":{"candidate_1":{"type":"noul","noul":0.9}}}).to_string()
            };
            write!(stream,"HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",if fail {"401 Unauthorized"} else {"200 OK"},body.len(),body).unwrap();
        });
        let mut client = Client::start(root.path(), false, Some(&endpoint));
        client.initialize();
        let args = if deep {
            json!({"question":"authentication","deep":true,"max_steps":1})
        } else {
            json!({"question":"authentication"})
        };
        let result = client.search(args);
        server.join().unwrap();
        assert!(!result.to_string().contains("fake-mcp-key"));
        assert_eq!(result["result"]["isError"], fail, "{result}");
        if !fail {
            assert_packet_envelope(&result);
            assert_eq!(
                result["result"]["structuredContent"]["results"][0]["path"],
                "auth.rs"
            );
            if deep {
                assert_eq!(
                    result["result"]["structuredContent"]["investigation"]["jevCalls"],
                    1
                );
            }
        }
        assert!(client.request("ping", json!({})).get("error").is_none());
    }
}

#[test]
fn closing_client_input_exits_server() {
    let root = fixture();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    client.input.take();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = client.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "server did not stop after EOF"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn ripgrep_config_cannot_enable_outside_symlink_reads() {
    let root = fixture();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.rs"), "fn outside_secret() {}\n").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.rs"),
        root.path().join("leak.rs"),
    )
    .unwrap();
    let config = outside.path().join("ripgrep-config");
    fs::write(&config, "--follow\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oko"))
        .current_dir(root.path())
        .env("RIPGREP_CONFIG_PATH", config)
        .args(["ask", "outside secret", "--no-jev", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("outside_secret"));
}

/// Accept every request until the MCP search returns, so an accidental second
/// ranking call is observable rather than merely producing a connection error.
fn search_with_counted_provider(
    root: &Path,
    args: Value,
    response: impl Fn(&Value) -> Value + Send + 'static,
) -> (Value, Vec<Value>) {
    use std::{
        io::Read,
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Instant,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let done = Arc::new(AtomicBool::new(false));
    let server_done = done.clone();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut requests = Vec::new();
        while !server_done.load(Ordering::Acquire) {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "mock provider timed out");
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("mock provider failed: {error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let request: Value = loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0, "incomplete provider request");
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    let len: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + len {
                        assert!(headers.contains("authorization: bearer fake-mcp-key"));
                        break serde_json::from_slice(&bytes[end + 4..end + 4 + len]).unwrap();
                    }
                }
            };
            let body = serde_json::to_vec(&response(&request)).unwrap();
            requests.push(request);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    let mut client = Client::start(root, false, Some(&endpoint));
    client.initialize();
    let result = client.search(args);
    done.store(true, Ordering::Release);
    (result, server.join().unwrap())
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

fn assert_packet_envelope(response: &Value) -> &Value {
    assert_eq!(response["result"]["isError"], false, "{response}");
    let result = &response["result"];
    assert!(
        serde_json::to_vec(result).unwrap().len() <= 16_000,
        "the complete MCP result includes both structured content and escaped text"
    );
    let packet = &result["structuredContent"];
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        text, *packet,
        "text-only MCP clients receive the same evidence"
    );
    assert!(packet["results"].as_array().unwrap().len() <= 3);
    assert!(packet["related"].as_array().unwrap().len() <= 2);
    assert!(packet["truncated"].is_boolean());
    for phase in ["preparationMs", "scanMs", "contextMs", "totalMs"] {
        assert!(
            packet["timings"][phase].as_f64().is_some_and(|n| n >= 0.0),
            "missing or invalid timing {phase}: {packet}"
        );
    }
    for optional_phase in ["shortlistMs", "investigateMs"] {
        assert!(
            packet["timings"][optional_phase].is_null()
                || packet["timings"][optional_phase]
                    .as_f64()
                    .is_some_and(|n| n >= 0.0)
        );
    }
    packet
}

#[test]
fn normal_packet_retains_thirty_previews_and_expands_a_late_winner_in_one_call() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..30 {
        let mut lines = vec![
            format!("fn authenticate_{index:02}() {{"),
            "    verify_credentials();".into(),
            "    write_session();".into(),
        ];
        for line in 0..65 {
            lines.push(format!(
                "    // authentication step {line:02}: {}",
                "source evidence retains the original surrounding implementation ".repeat(2)
            ));
        }
        lines.push("}".into());
        fs::write(root.path().join(format!("{index:02}.rs")), lines.join("\n")).unwrap();
    }
    fs::write(
        root.path().join("support.rs"),
        "fn verify_credentials() { compare_digest(); }\nfn write_session() { persist_cookie(); }\n",
    )
    .unwrap();
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"authentication"}),
        |request| {
            let candidates = request["state"]["candidates"].as_array().unwrap();
            assert_eq!(
                candidates.len(),
                30,
                "full chunks would exceed the request budget"
            );
            let winner = candidates.last().unwrap()["candidate"].as_str().unwrap();
            relevance_response(request, |candidate| {
                if candidate["candidate"].as_str().unwrap() == winner {
                    0.9
                } else {
                    0.05
                }
            })
        },
    );
    assert_eq!(
        requests.len(),
        1,
        "context expansion must not call Jev again"
    );
    let packet = assert_packet_envelope(&response);
    assert_eq!(packet["retrieval"]["shortlistedCandidates"], 30);
    assert_eq!(packet["retrieval"]["rankedCandidates"], 30);
    assert_eq!(packet["retrieval"]["omittedCandidates"], 0);
    let winner = &requests[0]["state"]["candidates"][29];
    let expected_path = winner["source"]
        .as_str()
        .unwrap()
        .split(':')
        .next()
        .unwrap();
    let result = &packet["results"][0];
    assert_eq!(result["path"], expected_path);
    let source = fs::read_to_string(root.path().join(expected_path)).unwrap();
    let start = result["startLine"].as_u64().unwrap() as usize;
    let end = result["endLine"].as_u64().unwrap() as usize;
    assert_eq!(
        result["text"].as_str().unwrap(),
        source
            .lines()
            .skip(start - 1)
            .take(end - start + 1)
            .collect::<Vec<_>>()
            .join("\n"),
        "the answer restores source, not numbered/discontinuous ranking previews"
    );
    let related = packet["related"].as_array().unwrap();
    assert_eq!(
        related.len(),
        2,
        "both referenced local helpers should be available"
    );
    assert!(
        related
            .iter()
            .all(|definition| definition["path"] == "support.rs")
    );
    let evidence = related
        .iter()
        .map(|definition| definition["text"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(evidence.contains("fn verify_credentials()"));
    assert!(evidence.contains("fn write_session()"));
    assert!(!response.to_string().contains("fake-mcp-key"));
}

#[test]
fn independent_scores_keep_multiple_implementations_in_one_provider_call() {
    let root = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "password.rs",
            "pub fn authenticate_password() { verify_password_hash(); }",
        ),
        (
            "token.rs",
            "pub fn authenticate_token() { validate_signature(); }",
        ),
        (
            "test.rs",
            "#[test]\nfn authentication_contract() { assert!(true); }",
        ),
        ("docs.md", "Authentication overview and configuration."),
    ] {
        fs::write(root.path().join(path), source).unwrap();
    }
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"Where is authentication implemented?"}),
        |request| {
            relevance_response(request, |candidate| {
                let source = candidate["source"].as_str().unwrap();
                if source.starts_with("password.rs:") {
                    0.97
                } else if source.starts_with("token.rs:") {
                    0.94
                } else if source.starts_with("test.rs:") {
                    0.5
                } else {
                    0.1
                }
            })
        },
    );
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["questions"].as_object().unwrap().len(), 4);
    let packet = assert_packet_envelope(&response);
    let results = packet["results"].as_array().unwrap();
    assert_eq!(
        results.len(),
        2,
        "uncertain and irrelevant candidates are excluded"
    );
    assert_eq!(results[0]["path"], "password.rs");
    assert_eq!(results[0]["score"], 0.97);
    assert_eq!(results[1]["path"], "token.rs");
    assert_eq!(results[1]["score"], 0.94);
    assert!(
        results[0]["text"]
            .as_str()
            .unwrap()
            .contains("verify_password_hash")
    );
    assert!(
        results[1]["text"]
            .as_str()
            .unwrap()
            .contains("validate_signature")
    );
}

#[test]
fn independent_scores_can_return_no_match_without_a_none_choice() {
    let root = fixture();
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"authentication"}),
        |request| relevance_response(request, |_| 0.5),
    );
    assert_eq!(requests.len(), 1);
    assert!(requests[0]["questions"].get("selection").is_none());
    assert!(requests[0]["questions"].get("none").is_none());
    let packet = assert_packet_envelope(&response);
    assert!(packet["results"].as_array().unwrap().is_empty());
    assert!(packet["related"].as_array().unwrap().is_empty());
    assert_eq!(packet["retrieval"]["rankedCandidates"], 1);
    assert_eq!(packet["retrieval"]["omittedCandidates"], 0);
}

#[test]
fn normal_search_preserves_annotations_and_returns_a_long_signature_body() {
    let root = tempfile::tempdir().unwrap();
    let mut implementation = vec!["pub fn authenticate_session(".to_owned()];
    implementation.extend((0..16).map(|index| format!("    argument_{index}: &str,")));
    implementation.extend([
        ") {".into(),
        "    validate_session_credentials();".into(),
        "    persist_authenticated_session();".into(),
        "}".into(),
    ]);
    let implementation = implementation.join("\n");
    fs::write(root.path().join("session.rs"), &implementation).unwrap();
    for (path, header, footer) in [
        (
            "contract.rs",
            "#[test]\nfn authentication_contract() {",
            "}",
        ),
        (
            "handler.py",
            "@router.post(\"/session\")\ndef authenticate_session(request):",
            "    return session",
        ),
    ] {
        let body = (0..50)
            .map(|index| {
                format!(
                    "    authentication_session_step_{index}({});",
                    "authentication_session_argument, ".repeat(5)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            root.path().join(path),
            format!("{header}\n{body}\n{footer}"),
        )
        .unwrap();
    }
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"Where is session authentication implemented?"}),
        |request| {
            let candidates = request["state"]["candidates"].as_array().unwrap();
            for (path, evidence) in [
                ("contract.rs:", "#[test]"),
                ("handler.py:", "@router.post(\"/session\")"),
            ] {
                assert!(
                    candidates.iter().any(|candidate| {
                        candidate["source"].as_str().unwrap().starts_with(path)
                            && candidate["text"].as_str().unwrap().contains(evidence)
                    }),
                    "the provider must see attached source context for {path}"
                );
            }
            assert!(candidates.iter().any(|candidate| {
                candidate["source"]
                    .as_str()
                    .unwrap()
                    .starts_with("session.rs:")
            }));
            relevance_response(request, |candidate| {
                if candidate["source"]
                    .as_str()
                    .unwrap()
                    .starts_with("session.rs:")
                {
                    0.95
                } else {
                    0.01
                }
            })
        },
    );
    assert_eq!(requests.len(), 1, "normal search uses one provider call");
    let mut application_request = requests[0].clone();
    application_request.as_object_mut().unwrap().remove("model");
    assert!(serde_json::to_vec(&application_request).unwrap().len() <= 32_000);
    let packet = assert_packet_envelope(&response);
    assert_eq!(packet["retrieval"]["omittedCandidates"], 0);
    let result = &packet["results"][0];
    assert_eq!(result["path"], "session.rs");
    assert_eq!(result["symbol"]["name"], "authenticate_session");
    assert_eq!(result["text"], implementation);
    assert_eq!(result["truncated"], false);
    assert!(!response.to_string().contains("fake-mcp-key"));
}

#[test]
fn qualified_external_calls_do_not_pull_unrelated_definitions_into_the_packet() {
    let root = tempfile::tempdir().unwrap();
    let auth_source = [
        "pub fn start_oauth() {",
        "    let state = create_oauth_state();",
        "    let query = urlencoding::encode(&state);",
        "    open_browser(&query);",
        "}",
    ]
    .join("\n");
    fs::write(root.path().join("auth.rs"), &auth_source).unwrap();
    fs::write(
        root.path().join("support.rs"),
        "fn create_oauth_state() { random_token(); }\n",
    )
    .unwrap();
    for path in ["graphics.rs", "renderer.rs"] {
        fs::write(
            root.path().join(path),
            "fn encode(frame: &[u8]) { submit_gpu_commands(frame); }\n",
        )
        .unwrap();
    }

    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"Where is OAuth authentication handled?"}),
        |request| {
            assert!(
                request["state"]["candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|candidate| {
                        candidate["source"]
                            .as_str()
                            .unwrap()
                            .starts_with("auth.rs:")
                    })
            );
            relevance_response(request, |candidate| {
                if candidate["source"]
                    .as_str()
                    .unwrap()
                    .starts_with("auth.rs:")
                {
                    0.95
                } else {
                    0.01
                }
            })
        },
    );
    assert_eq!(requests.len(), 1, "related lookup stays local");
    let packet = assert_packet_envelope(&response);
    let results = packet["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["path"], "auth.rs");
    assert_eq!(results[0]["startLine"], 1);
    assert_eq!(results[0]["endLine"], 5);
    assert_eq!(
        results[0]["text"], auth_source,
        "qualification filtering must not rewrite the primary source"
    );
    let related = packet["related"].as_array().unwrap();
    assert_eq!(
        related.len(),
        1,
        "an external encode call must not match either local graphics definition: {related:?}"
    );
    assert_eq!(related[0]["path"], "support.rs");
    assert_eq!(related[0]["symbol"]["name"], "create_oauth_state");
    assert_eq!(
        related[0]["text"],
        "fn create_oauth_state() { random_token(); }"
    );
    assert_eq!(related[0]["ambiguous"], false);
    assert_eq!(related[0]["candidateCount"], 1);
    assert_eq!(
        related[0]["referencedFrom"],
        json!([{"path":"auth.rs","line":2}])
    );
}

#[test]
fn packet_budget_counts_utf8_json_escaping_question_and_compatibility_content() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..6 {
        let lines = (0..80)
            .map(|line| format!("    // authentication {line}: {}", "\"\\😀\t".repeat(20)))
            .collect::<Vec<_>>();
        fs::write(
            root.path().join(format!("auth{index}.rs")),
            format!("fn authentication_{index}() {{\n{}\n}}", lines.join("\n")),
        )
        .unwrap();
    }
    let mut question = "authentication ".to_owned();
    while question.len() + "\"\\😀\t".len() <= 4096 {
        question.push_str("\"\\😀\t");
    }
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let response = client.search(json!({"question":question}));
    let packet = assert_packet_envelope(&response);
    assert_eq!(packet["ranking"], "lexical");
    assert!(packet["truncated"].as_bool().unwrap());
    assert!(!packet["results"].as_array().unwrap().is_empty());
    assert!(client.request("ping", json!({})).get("error").is_none());
}

#[test]
fn escaped_packet_fitting_preserves_useful_source_instead_of_over_shrinking() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..6 {
        let lines = (0..80)
            .map(|line| format!("    // authentication {line}: {}", "\\".repeat(200)))
            .collect::<Vec<_>>();
        fs::write(
            root.path().join(format!("auth{index}.rs")),
            format!("fn authentication_{index}() {{\n{}\n}}", lines.join("\n")),
        )
        .unwrap();
    }
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    for question in [
        "authentication".to_owned(),
        format!("authentication {}", "\\".repeat(4000)),
    ] {
        let response = client.search(json!({"question":question}));
        let packet = assert_packet_envelope(&response);
        let results = packet["results"].as_array().unwrap();
        assert_eq!(
            results.len(),
            3,
            "all three matches fit with compact context"
        );
        assert!(
            results.iter().all(|result| result["text"]
                .as_str()
                .unwrap()
                .contains("// authentication")),
            "JSON escaping must not reduce every result to just a function header"
        );
    }
}
