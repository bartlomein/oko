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
    metrics: tempfile::TempDir,
    _cache: Option<tempfile::TempDir>,
}
impl Client {
    fn start(root: &Path, offline: bool, endpoint: Option<&str>) -> Self {
        let cache = tempfile::tempdir().unwrap();
        let mut client = Self::start_with_cache(root, offline, endpoint, cache.path());
        client._cache = Some(cache);
        client
    }
    fn start_with_cache(root: &Path, offline: bool, endpoint: Option<&str>, cache: &Path) -> Self {
        Self::start_with_env(root, offline, endpoint, cache, &[])
    }
    fn start_with_env(
        root: &Path,
        offline: bool,
        endpoint: Option<&str>,
        cache: &Path,
        env: &[(&str, &str)],
    ) -> Self {
        let metrics = tempfile::tempdir().unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oko"));
        cmd.args(["mcp", "--root"])
            .arg(root)
            .env("OKO_METRICS_FILE", metrics.path().join("metrics.jsonl"))
            .env("OKO_CACHE_DIR", cache)
            .env("OKO_NO_CACHE", "0")
            .env("TYPESAFE_API_KEY", "")
            .env_remove("TYPESAFE_DEFAULT_MODEL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        cmd.envs(env.iter().copied());
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
            metrics,
            _cache: None,
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
    /// Search lines, or startup preparation events, from `OKO_METRICS_FILE`.
    fn recorded(&self, events: bool) -> Vec<Value> {
        fs::read_to_string(self.metrics.path().join("metrics.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|line| line.get("event").is_some() == events)
            .collect()
    }
    /// Startup preparation holds the cache lock until its event is recorded.
    fn wait_for_prewarm(&self) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(event) = self.recorded(true).pop() {
                assert_eq!(event["event"], "prewarm");
                return event;
            }
            assert!(std::time::Instant::now() < deadline, "no prewarm event");
            thread::sleep(Duration::from_millis(5));
        }
    }
    /// The agent-visible result, plus the operator's `OKO_METRICS_FILE` line
    /// for a completed search under the test-only `metrics` key.
    fn search(&mut self, args: Value) -> Value {
        let before = self.recorded(false).len();
        let mut response = self.request("tools/call", json!({"name":"search","arguments":args}));
        let recorded = self.recorded(false);
        if response["result"]["isError"] == false {
            assert_eq!(recorded.len(), before + 1, "one metrics line per search");
            response["metrics"] = recorded.last().unwrap().clone();
        } else {
            assert_eq!(recorded.len(), before, "failed searches record nothing");
        }
        response
    }
}

#[test]
fn stdio_search_reuses_memory_and_disk_preparation_across_server_restarts() {
    let root = fixture();
    let cache = tempfile::tempdir().unwrap();
    let mut client = Client::start_with_cache(root.path(), true, None, cache.path());
    client.initialize();
    // Startup preparation pays for the cold build; the first search does not.
    let prewarm = client.wait_for_prewarm();
    assert_eq!(prewarm["cache"]["status"], "cold");
    assert_eq!(prewarm["cache"]["rebuiltFiles"], 1);
    let cold = client.search(json!({"question":"authentication token"}));
    let cold_packet = assert_packet_envelope(&cold);
    assert_eq!(cold_packet["timings"]["cache"]["status"], "memory");
    assert_eq!(cold_packet["timings"]["cache"]["rebuiltFiles"], 0);
    assert!(cold_packet["timings"]["cacheWaitMs"].is_u64());

    let warm = client.search(json!({"question":"authentication token"}));
    let warm_packet = assert_packet_envelope(&warm);
    assert_eq!(warm_packet["timings"]["cache"]["status"], "memory");
    assert_eq!(warm_packet["timings"]["cache"]["rebuiltFiles"], 0);
    assert_eq!(warm_packet["timings"]["cache"]["reusedFiles"], 1);
    assert_eq!(cold_packet["results"], warm_packet["results"]);
    drop(client);

    let mut client = Client::start_with_cache(root.path(), true, None, cache.path());
    client.initialize();
    let prewarm = client.wait_for_prewarm();
    assert_eq!(prewarm["cache"]["status"], "disk");
    assert_eq!(prewarm["cache"]["rebuiltFiles"], 0);
    let restarted = client.search(json!({"question":"authentication token"}));
    let restarted_packet = assert_packet_envelope(&restarted);
    assert_eq!(restarted_packet["timings"]["cache"]["status"], "memory");
    assert_eq!(cold_packet["results"], restarted_packet["results"]);
    assert_eq!(client.recorded(true).len(), 1, "one event per server");

    fs::write(
        root.path().join("auth.rs"),
        "fn revoke_authentication_token() {}\n",
    )
    .unwrap();
    let changed = client.search(json!({"question":"authentication token"}));
    let changed_packet = assert_packet_envelope(&changed);
    assert_eq!(changed_packet["timings"]["cache"]["rebuiltFiles"], 1);
    assert!(
        changed_packet["results"][0]["text"]
            .as_str()
            .unwrap()
            .contains("revoke_authentication")
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
#[test]
fn without_startup_preparation_the_first_search_does_the_work_itself() {
    let root = fixture();
    let cache = tempfile::tempdir().unwrap();
    let mut client = Client::start_with_env(
        root.path(),
        true,
        None,
        cache.path(),
        &[("OKO_NO_PREWARM", "1")],
    );
    client.initialize();
    let first = client.search(json!({"question":"authentication token"}));
    assert_eq!(
        assert_packet_envelope(&first)["timings"]["cache"]["status"],
        "cold"
    );
    assert!(client.recorded(true).is_empty());
    drop(client);

    // Without retained snapshots preparation would only be repeated.
    let mut client = Client::start_with_env(
        root.path(),
        true,
        None,
        cache.path(),
        &[("OKO_NO_CACHE", "1")],
    );
    client.initialize();
    let uncached = client.search(json!({"question":"authentication token"}));
    assert_eq!(
        assert_packet_envelope(&uncached)["timings"]["cache"]["status"],
        "disabled"
    );
    assert!(client.recorded(true).is_empty());
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
    assert_eq!(tools[0]["_meta"]["anthropic/alwaysLoad"], true);
    // Unknown fields are ignored rather than refused.
    assert_ne!(tools[0]["inputSchema"]["additionalProperties"], false);
    // Every agent turn pays for the tool definition, whether or not it searches.
    assert!(tools[0].get("outputSchema").is_none());
    // Raised from 1,950 for `symbols`, `mode` and `questions` (2026-09-24/25):
    // about 140 more tokens on every turn, for parameters that replace calls.
    let definition = serde_json::to_vec(&tools[0]).unwrap().len();
    assert!(
        definition <= 2_600,
        "tool definition grew to {definition} bytes"
    );
    // Brevity must not cost correctness: agents that read a completeness label
    // as a relevance claim stop before the evidence is sufficient.
    let description = tools[0]["description"].as_str().unwrap();
    for guidance in [
        "prefixed with its file line number",
        // A runner-up must never be mistaken for a match the ranker accepted.
        "`possible match` was rated below the relevance cutoff",
        "Labels describe only that excerpt",
        // A question spanning several locations still needs the others.
        "search again only for locations not shown",
    ] {
        assert!(description.contains(guidance), "{description}");
    }
    let result = client.search(json!({"question":"authentication token"}));
    assert_eq!(result["result"]["isError"], false, "{result}");
    let text = result["result"]["content"][0]["text"].as_str().unwrap();
    // The first line says what was searched; the fixture has two indexed files.
    assert!(text.starts_with("Index: 1 of 1 files, "), "{text}");
    assert_eq!(
        body(text),
        "auth.rs:1-1 (whole file)\n```\n1\tfn authenticate() { validate_token(); }\n```\n"
    );
    let data = &result["metrics"];
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
        result["metrics"]["results"][0]["text"]
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
    for args in [json!({}), json!({"question":"auth","intent":"bad"})] {
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
    // An unknown field is ignored.
    assert_eq!(
        client.search(json!({"question":"auth","max_results":5}))["result"]["isError"],
        false
    );
    let mut paid = Client::start(root.path(), false, None);
    paid.initialize();
    let result = paid.search(json!({"question":"auth"}));
    assert_eq!(result["result"]["isError"], true);
    assert!(
        result["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No TypeSafe key")
    );
}
#[test]
fn server_instructions_stay_brief_and_subdirectory_searches_state_their_path_base() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("server")).unwrap();
    fs::write(
        root.path().join("server/auth.rs"),
        "fn authenticate() { validate_token(); }\n",
    )
    .unwrap();
    let mut client = Client::start(root.path(), true, None);
    let response = client.request("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"oko-tests","version":"1"}}));
    client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let instructions = response["result"]["instructions"].as_str().unwrap();
    // Under tool search the instructions may be all an agent sees of Oko at
    // first, so they say what it is for; they still cost every session.
    assert!(instructions.len() <= 420, "{}", instructions.len());
    assert!(instructions.starts_with("Oko finds code in this repository"));

    let scoped = client.search(json!({"question":"authentication token","directory":"server"}));
    assert_packet_envelope(&scoped);
    assert_eq!(
        body(scoped["result"]["content"][0]["text"].as_str().unwrap()),
        "Paths are relative to server/.\n\nauth.rs:1-1 (whole file)\n```\n1\tfn authenticate() { validate_token(); }\n```\n"
    );
    let unscoped = client.search(json!({"question":"authentication token"}));
    assert!(
        body(unscoped["result"]["content"][0]["text"].as_str().unwrap())
            .starts_with("server/auth.rs:1-1 (whole file)\n")
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
            assert_eq!(result["metrics"]["results"][0]["path"], "auth.rs");
            if deep {
                assert_eq!(result["metrics"]["investigation"]["jevCalls"], 1);
                assert!(
                    body(result["result"]["content"][0]["text"].as_str().unwrap())
                        .starts_with("Deep search stopped after 1 step: "),
                    "{result}"
                );
            }
        }
        assert!(client.request("ping", json!({})).get("error").is_none());
    }
}

#[test]
fn a_slow_overloaded_or_unreachable_ranker_yields_labelled_keyword_matches() {
    use std::{io::Read, net::TcpListener};
    for (behavior, reason, class) in [
        ("slow", "timeout", "timeout"),
        ("overloaded", "unavailable", "http_status"),
        ("unreachable", "unreachable", "transport"),
    ] {
        let root = fixture();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = (behavior != "unreachable").then(|| {
            thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 65536];
                let _ = stream.read(&mut buffer).unwrap();
                if behavior == "slow" {
                    // Longer than the configured patience, far below the default timeout.
                    thread::sleep(Duration::from_millis(1500));
                } else {
                    write!(stream, "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                }
            })
        });
        let cache = tempfile::tempdir().unwrap();
        let mut client = Client::start_with_env(
            root.path(),
            false,
            Some(&endpoint),
            cache.path(),
            &[("OKO_JEV_TIMEOUT_MS", "500")],
        );
        client.initialize();
        let started = std::time::Instant::now();
        let response = client.search(json!({"question":"authentication token"}));
        assert!(
            started.elapsed() < Duration::from_millis(1400),
            "{behavior}: the search must not wait for the provider"
        );
        let packet = assert_packet_envelope(&response);
        assert_eq!(packet["ranking"], "lexical-fallback", "{behavior}");
        assert_eq!(packet["retrieval"]["lexicalFallback"], reason);
        assert_eq!(packet["retrieval"]["jevCalls"][0]["errorClass"], class);
        assert_eq!(packet["retrieval"]["jevCalls"][0]["success"], false);
        assert_eq!(packet["results"][0]["path"], "auth.rs");
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            body(text)
                .starts_with("The relevance ranker did not respond, so these are keyword matches"),
            "{text}"
        );
        assert!(text.contains("auth.rs:1-1 (whole file)"));
        assert!(!response.to_string().contains("fake-mcp-key"));
        drop(client);
        if let Some(server) = server {
            server.join().unwrap();
        }
    }
}

#[test]
fn runners_up_are_named_by_path_and_every_judgment_is_recorded() {
    let root = tempfile::tempdir().unwrap();
    for name in ["accepted", "close", "weak", "irrelevant"] {
        fs::write(
            root.path().join(format!("{name}.rs")),
            format!("pub fn parcel_dispatch_{name}() {{ deliver_parcel(); }}\n"),
        )
        .unwrap();
    }
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"parcel dispatch"}),
        |request| {
            relevance_response(request, |candidate| {
                let source = candidate["source"].as_str().unwrap();
                if source.starts_with("accepted.rs:") {
                    0.9
                } else if source.starts_with("close.rs:") {
                    0.45
                } else if source.starts_with("weak.rs:") {
                    0.25
                } else {
                    0.05
                }
            })
        },
    );
    assert_eq!(requests.len(), 1, "runners-up come from the same judgment");
    let packet = assert_packet_envelope(&response);
    let shown: Vec<_> = packet["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["path"].as_str().unwrap(), r["lowerConfidence"] == true))
        .collect();
    // The close runner-up takes a spare slot; the weak one is only named.
    let judged: Vec<_> = packet["retrieval"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["path"].as_str().unwrap(), c["score"].as_f64().unwrap()))
        .collect();
    assert_eq!(
        judged,
        [
            ("accepted.rs", 0.9),
            ("close.rs", 0.45),
            ("weak.rs", 0.25),
            ("irrelevant.rs", 0.05)
        ]
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(shown, [("accepted.rs", false), ("close.rs", true)]);
    assert!(
        text.contains("close.rs:1-1 (whole file, possible match)\n"),
        "{text}"
    );
    assert!(
        text.ends_with("\nOther candidates, not shown, best first:\nweak.rs:1-1\n"),
        "{text}"
    );
    assert!(!text.contains("irrelevant.rs"), "rated irrelevant: {text}");

    // Keyword order has no judgments; the next matches are still worth naming.
    for index in 0..8 {
        fs::write(
            root.path().join(format!("extra{index}.rs")),
            "pub fn parcel_dispatch_extra() {}\n",
        )
        .unwrap();
    }
    let mut offline = Client::start(root.path(), true, None);
    offline.initialize();
    let response = offline.search(json!({"question":"parcel dispatch"}));
    let packet = assert_packet_envelope(&response);
    assert_eq!(packet["results"].as_array().unwrap().len(), 3);
    assert!(packet["retrieval"]["candidates"][0].get("score").is_none());
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let listed = text
        .split("\nOther keyword matches, not shown:\n")
        .nth(1)
        .unwrap_or_else(|| panic!("{text}"));
    assert_eq!(listed.lines().count(), 6);
    for result in packet["results"].as_array().unwrap() {
        assert!(!listed.contains(result["path"].as_str().unwrap()), "{text}");
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
    let cache = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oko"))
        .current_dir(root.path())
        .env("OKO_CACHE_DIR", cache.path())
        .env("OKO_NO_CACHE", "0")
        .env("RIPGREP_CONFIG_PATH", config)
        .args(["ask", "outside secret", "--no-jev", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("outside_secret"));
}

/// Accept every request until the MCP search returns, so an accidental second
/// ranking call is observable rather than merely producing a connection error.
#[test]
fn parallel_searches_all_run_instead_of_one_being_turned_away() {
    use std::sync::atomic::Ordering;
    // Agents send several searches in one turn. Each must get results, and
    // their Jev requests must overlap rather than queue behind one another.
    let root = fixture();
    let (endpoint, most, done, server) = slow_provider();
    let mut client = Client::start(root.path(), false, Some(&endpoint));
    client.initialize();
    // One more than run at once, so the last call has to wait for a slot.
    let ids: Vec<u64> = (1..=5).map(|n| 100 + n).collect();
    for id in &ids {
        client.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
            "params":{"name":"search","arguments":{"question":"authentication"}}}));
    }
    let mut responses = std::collections::HashMap::new();
    while responses.len() < ids.len() {
        let response = client
            .output
            .recv_timeout(Duration::from_secs(15))
            .expect("MCP response timed out");
        if let Some(id) = response["id"].as_u64() {
            responses.insert(id, response);
        }
    }
    done.store(true, Ordering::Release);
    server.join().unwrap();
    for id in &ids {
        let response = &responses[id];
        assert_eq!(response["result"]["isError"], false, "{response}");
        assert!(
            body(response["result"]["content"][0]["text"].as_str().unwrap())
                .starts_with("auth.rs:"),
            "{response}"
        );
    }
    assert!(
        most.load(Ordering::SeqCst) > 1,
        "searches ran one at a time"
    );
}

/// A mock provider that holds every request for 300 ms, so searches sent
/// together overlap. Returns its endpoint, the most requests it held at once,
/// the flag that stops it, and its thread.
fn slow_provider() -> (
    String,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread::JoinHandle<()>,
) {
    use std::{
        io::Read,
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Instant,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (in_flight, most, done) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let (server_in_flight, server_most, server_done) =
        (in_flight.clone(), most.clone(), done.clone());
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut handlers = Vec::new();
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
            let (in_flight, most) = (server_in_flight.clone(), server_most.clone());
            handlers.push(thread::spawn(move || {
                stream.set_nonblocking(false).unwrap();
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
                            break serde_json::from_slice(&bytes[end + 4..end + 4 + len]).unwrap();
                        }
                    }
                };
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                // Long enough for the searches to overlap, well inside Jev patience.
                thread::sleep(Duration::from_millis(300));
                in_flight.fetch_sub(1, Ordering::SeqCst);
                let body = serde_json::to_vec(&relevance_response(&request, |_| 0.9)).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }));
        }
        for handler in handlers {
            handler.join().unwrap();
        }
    });
    (endpoint, most, done, server)
}

fn search_with_counted_provider(
    root: &Path,
    args: Value,
    response: impl Fn(&Value) -> Value + Send + 'static,
) -> (Value, Vec<Value>) {
    // Most tests are about the shortlist's requests: nothing beside it is relevant.
    let (result, requests, _) = search_with_provider(root, args, response, |request| {
        Some(relevance_response(request, |_| 0.0))
    });
    (result, requests)
}

/// As above, with the requests judged beside the shortlist answered by `beside`
/// (`None` = the provider fails) and returned separately with their phase.
fn search_with_provider(
    root: &Path,
    args: Value,
    response: impl Fn(&Value) -> Value + Send + 'static,
    beside: impl Fn(&Value) -> Option<Value> + Send + 'static,
) -> (Value, Vec<Value>, Vec<(String, Value)>) {
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
        let mut beside_requests = Vec::new();
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
            let (request, headers): (Value, String) = loop {
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
                        let body = serde_json::from_slice(&bytes[end + 4..end + 4 + len]).unwrap();
                        break (body, headers);
                    }
                }
            };
            // Candidates judged beside the shortlist arrive in requests of their
            // own, in no fixed order. These tests are about the shortlist's.
            let phase = ["connected", "further"]
                .into_iter()
                .find(|phase| headers.contains(&format!("x-oko-phase: {phase}")));
            if let Some(phase) = phase {
                // They arrive in no fixed order relative to the shortlist's request.
                match beside(&request) {
                    Some(answer) => {
                        let body = serde_json::to_vec(&answer).unwrap();
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                        stream.write_all(&body).unwrap();
                    }
                    None => write!(stream, "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap(),
                }
                beside_requests.push((phase.to_owned(), request));
                continue;
            }
            let body = serde_json::to_vec(&response(&request)).unwrap();
            requests.push(request);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
        (requests, beside_requests)
    });
    let mut client = Client::start(root, false, Some(&endpoint));
    client.initialize();
    let result = client.search(args);
    done.store(true, Ordering::Release);
    let (requests, beside_requests) = server.join().unwrap();
    (result, requests, beside_requests)
}

fn ledger_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("ledger.rs"),
        "pub fn parse_ledger_rows(raw: &str) -> Vec<Row> {\n    raw.lines().map(row_from_line).collect()\n}\n",
    )
    .unwrap();
    // Shares no word with the question, in its text or its path: only the
    // file name pairing can find it.
    fs::write(
        root.path().join("ledger_test.rs"),
        "#[test]\nfn keeps_trailing() {\n    assert!(verify_everything());\n}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("weather.rs"),
        "pub fn forecast() -> u8 {\n    7\n}\n",
    )
    .unwrap();
    root
}

#[test]
fn connected_files_are_judged_beside_the_shortlist_and_only_listed() {
    let root = ledger_fixture();
    let (response, requests, beside) = search_with_provider(
        root.path(),
        json!({"question":"where are rows parsed from raw input"}),
        |request| relevance_response(request, |_| 0.9),
        |request| Some(relevance_response(request, |_| 0.9)),
    );
    assert_eq!(requests.len(), 1, "the shortlist is still judged once");
    let connected: Vec<_> = beside
        .iter()
        .filter(|(phase, _)| phase == "connected")
        .collect();
    assert_eq!(connected.len(), 1);
    let state = &connected[0].1["state"];
    assert!(
        state.get("relatedCriteria").is_some() && state.get("implementationCriteria").is_none()
    );
    let sources: Vec<_> = state["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["source"].as_str().unwrap().to_owned())
        .collect();
    assert!(
        sources
            .iter()
            .any(|source| source.starts_with("ledger_test.rs")),
        "{sources:?}"
    );
    assert!(
        sources
            .iter()
            .all(|source| !source.starts_with("weather.rs")),
        "{sources:?}"
    );
    // Rated as relevant as the match itself, it still takes no excerpt.
    let packet = assert_packet_envelope(&response);
    let shown: Vec<_> = packet["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].clone())
        .collect();
    assert_eq!(shown, [json!("ledger.rs")]);
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let listed = text
        .split("Other candidates, not shown, best first:")
        .nth(1)
        .unwrap_or("");
    assert!(listed.contains("ledger_test.rs"), "{text}");
    let recorded = packet["retrieval"]["candidates"].as_array().unwrap();
    let test_file = recorded
        .iter()
        .find(|c| c["path"] == "ledger_test.rs")
        .unwrap();
    assert_eq!(test_file["connected"], true);
    assert!((test_file["score"].as_f64().unwrap() - 0.9 * 0.4).abs() < 1e-9);
}

#[test]
fn a_failing_request_beside_the_shortlist_changes_nothing_shown() {
    let root = ledger_fixture();
    let (response, requests, beside) = search_with_provider(
        root.path(),
        json!({"question":"where are rows parsed from raw input"}),
        |request| relevance_response(request, |_| 0.9),
        |_| None,
    );
    assert_eq!(requests.len(), 1);
    assert!(!beside.is_empty());
    let packet = assert_packet_envelope(&response);
    assert_eq!(packet["ranking"], "jev", "no keyword fallback");
    assert_eq!(packet["results"][0]["path"], "ledger.rs");
    assert!(packet["retrieval"].get("lexicalFallback").is_none());
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

/// The answer without its coverage line, which only a session's first
/// answer (or a changed index) carries.
fn body(text: &str) -> &str {
    let rest = if text.starts_with("Index: ") {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        text
    };
    // A blank line separates the notes from the excerpts.
    rest.strip_prefix('\n').unwrap_or(rest)
}

fn assert_packet_envelope(response: &Value) -> &Value {
    assert_eq!(response["result"]["isError"], false, "{response}");
    let result = &response["result"];
    assert!(
        serde_json::to_vec(result).unwrap().len() <= 16_000,
        "the complete MCP result, including JSON escaping, is bounded"
    );
    assert!(
        result.get("structuredContent").is_none(),
        "the agent receives one copy of the evidence"
    );
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    let text = content[0]["text"].as_str().unwrap();
    for serving_detail in ["timings", "retrieval", "jevCalls", "score", "Ms\""] {
        assert!(!text.contains(serving_detail), "{serving_detail}: {text}");
    }
    let packet = &response["metrics"];
    // The server measures before rmcp drops fields that are absent on the wire.
    let measured = packet["responseBytes"].as_u64().unwrap() as usize;
    assert!((serde_json::to_vec(result).unwrap().len()..=16_000).contains(&measured));
    // Every structured excerpt reaches the agent as exact, unescaped source.
    let excerpts = packet["results"]
        .as_array()
        .unwrap()
        .iter()
        .chain(packet["related"].as_array().unwrap());
    for excerpt in excerpts {
        let mut label = if excerpt["wholeFile"] == true {
            "whole file".to_owned()
        } else if excerpt["definitions"].as_u64().unwrap() > 1 {
            format!("{} complete definitions", excerpt["definitions"])
        } else if excerpt["definitionComplete"] == true {
            "complete definition".to_owned()
        } else {
            "partial excerpt".to_owned()
        };
        assert_eq!(excerpt["truncated"] == true, label == "partial excerpt");
        if excerpt["lowerConfidence"] == true {
            label.push_str(", possible match");
        }
        if excerpt["exactName"] == true {
            label.push_str(", exact name match");
        }
        if let Some(tag) = excerpt["tag"].as_str() {
            label.push_str(", ");
            label.push_str(tag);
        }
        let omitted: Vec<(u64, u64)> = excerpt["omitted"]
            .as_array()
            .map(|ranges| {
                ranges
                    .iter()
                    .map(|r| (r[0].as_u64().unwrap(), r[1].as_u64().unwrap()))
                    .collect()
            })
            .unwrap_or_default();
        if excerpt["outline"] == true {
            label.push_str(", outline");
        } else if !omitted.is_empty() {
            label.push_str(", body abridged");
        }
        if excerpt["seen"] == true {
            // Sent whole earlier in the session: a citable stub, no body.
            let mut stub = "shown in an earlier answer".to_owned();
            if let Some(tag) = excerpt["tag"].as_str() {
                stub.push_str(", ");
                stub.push_str(tag);
            }
            let header = format!(
                "{}:{}-{} ({stub})\n",
                excerpt["path"].as_str().unwrap(),
                excerpt["startLine"],
                excerpt["endLine"]
            );
            assert!(text.contains(&header), "{header}: {text}");
            continue;
        }
        let header = format!(
            "{}:{}-{} ({label})\n",
            excerpt["path"].as_str().unwrap(),
            excerpt["startLine"],
            excerpt["endLine"]
        );
        let body = &text[text
            .find(&header)
            .unwrap_or_else(|| panic!("{header}: {text}"))
            + header.len()..];
        let fence = body.lines().next().unwrap();
        assert!(fence.len() >= 3 && fence.chars().all(|c| c == '`'));
        // The agent reads each line's number instead of counting from the header.
        let mut number = excerpt["startLine"].as_u64().unwrap();
        let mut gaps = omitted.iter().peekable();
        let mut numbered = String::new();
        for line in excerpt["text"].as_str().unwrap().split('\n') {
            while let Some((from, to)) = gaps.next_if(|(from, _)| *from <= number) {
                numbered.push_str(&format!(
                    "… {} lines omitted ({}:{from}-{to}) …\n",
                    to - from + 1,
                    excerpt["path"].as_str().unwrap()
                ));
                number = to + 1;
            }
            numbered.push_str(&format!("{number}\t{line}\n"));
            number += 1;
        }
        assert!(
            body[fence.len() + 1..].starts_with(&format!("{numbered}{fence}\n")),
            "{header}: {text}"
        );
    }
    if packet["results"].as_array().unwrap().is_empty() {
        assert!(body(text).starts_with("No relevant code found."), "{text}");
    }
    // Three excerpts for one question; a several-question call may show up to twelve.
    let asked = packet["retrieval"]["questions"]
        .as_array()
        .map_or(1, Vec::len);
    assert!(packet["results"].as_array().unwrap().len() <= (3 + 2 * (asked - 1)).min(12));
    assert!(packet["related"].as_array().unwrap().len() <= 2);
    assert!(packet["truncated"].is_boolean());
    for phase in [
        "preparationMs",
        "scanMs",
        "contextMs",
        "totalMs",
        "totalWallNs",
    ] {
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
fn implementation_intent_recovers_prose_crowded_source_in_one_request() {
    let root = tempfile::tempdir().unwrap();
    let question = "select parcel depot delivery";
    for index in 0..75 {
        fs::write(root.path().join(format!("guide{index:02}.md")), question).unwrap();
    }
    let source = format!(
        "pub fn choose(shipment: &Parcel) -> Depot {{\n    // {}\n    shipment.depot\n}}",
        "unrelated ".repeat(100),
    );
    fs::write(root.path().join("decision.rs"), &source).unwrap();
    let corpus = oko::search::workspace_chunks(root.path()).unwrap();
    let broad = oko::search::rank_lexically(&corpus, question);
    assert_eq!(broad.len(), oko::search::SHORTLIST_LIMIT);
    assert!(broad.iter().all(|chunk| chunk.path != "decision.rs"));

    for intent in [None, Some("general"), Some("explanation")] {
        let implementation = intent.is_none();
        let mut args = json!({"question":question});
        if let Some(intent) = intent {
            args["intent"] = json!(intent);
        }
        let (response, requests) =
            search_with_counted_provider(root.path(), args, move |request| {
                relevance_response(request, |candidate| {
                    let path = candidate["source"].as_str().unwrap();
                    if (implementation && path.starts_with("decision.rs:"))
                        || (!implementation && path.starts_with("guide00.md:"))
                    {
                        0.95
                    } else {
                        0.05
                    }
                })
            });
        assert_eq!(requests.len(), 1);
        let mut application_request = requests[0].clone();
        // Transport adds the model after the application request budget check.
        application_request.as_object_mut().unwrap().remove("model");
        assert!(
            serde_json::to_vec(&application_request).unwrap().len()
                <= oko::ranking::MAX_JEV_REQUEST_BYTES
        );
        let candidates = requests[0]["state"]["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), oko::ranking::MAX_ITEMS);
        let paths: Vec<_> = candidates
            .iter()
            .map(|candidate| {
                candidate["source"]
                    .as_str()
                    .unwrap()
                    .split(':')
                    .next()
                    .unwrap()
            })
            .collect();
        let packet = assert_packet_envelope(&response);
        assert_eq!(packet["ranking"], "jev");
        assert_eq!(packet["retrieval"]["omittedCandidates"], 0);
        if implementation {
            let helper = candidates
                .iter()
                .find(|candidate| {
                    candidate["source"]
                        .as_str()
                        .unwrap()
                        .starts_with("decision.rs:")
                })
                .expect("default implementation retrieval must recover the actual source");
            assert!(helper["text"].as_str().unwrap().contains("shipment.depot"));
            let winner = &packet["results"][0];
            assert_eq!(winner["path"], "decision.rs");
            assert_eq!(winner["startLine"], 1);
            assert_eq!(winner["endLine"], source.lines().count());
            assert_eq!(winner["text"], source);
        } else {
            assert_eq!(
                paths,
                broad
                    .iter()
                    .map(|chunk| chunk.path.as_str())
                    .collect::<Vec<_>>()
            );
            assert_eq!(packet["results"][0]["path"], "guide00.md");
        }
    }

    let mut offline = Client::start(root.path(), true, None);
    offline.initialize();
    let response = offline.search(json!({"question":question}));
    let packet = assert_packet_envelope(&response);
    assert_eq!(packet["ranking"], "lexical");
    let paths: Vec<_> = packet["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        broad
            .iter()
            .take(3)
            .map(|chunk| chunk.path.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn headerless_primary_keeps_late_decision_evidence_when_the_winner_fits() {
    let root = tempfile::tempdir().unwrap();
    let mut lines = vec!["    step();"; 400];
    lines[0] = "pub fn process_batch() {";
    lines[123] = "    trace(\"pending records checked before persisting records\");";
    lines[204] = "    if !pending_records.is_empty() {";
    lines[205] = "        let accepted = pending_records.into_iter().filter(|record| {";
    lines[206] = "            !index.contains(record.key)";
    lines[207] = "        }).collect();";
    lines[208] = "        persist(accepted);";
    lines[209] = "    }";
    lines[399] = "}";
    fs::write(root.path().join("worker.rs"), lines.join("\n")).unwrap();
    let corpus = oko::search::workspace_chunks(root.path()).unwrap();
    let winner = corpus
        .iter()
        .find(|chunk| chunk.text.contains("!index.contains(record.key)"))
        .unwrap();
    assert_eq!((winner.start_line, winner.end_line), (116, 235));
    assert!(!winner.text.contains("fn process_batch"));
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"where pending records are checked before persisting records"}),
        |request| {
            let target = request["state"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["source"] == "worker.rs:116-235")
                .expect("the full headerless winner must enter reranking");
            assert!(
                target["text"]
                    .as_str()
                    .unwrap()
                    .contains("!index.contains(record.key)")
            );
            relevance_response(request, |candidate| {
                if candidate["source"] == "worker.rs:116-235" {
                    0.95
                } else {
                    0.01
                }
            })
        },
    );
    assert_eq!(requests.len(), 1);
    let packet = assert_packet_envelope(&response);
    let primary = &packet["results"][0];
    assert_eq!(primary["path"], "worker.rs");
    let start = primary["startLine"].as_u64().unwrap() as usize;
    let end = primary["endLine"].as_u64().unwrap() as usize;
    assert!(start <= winner.start_line && end >= winner.end_line);
    // The 400-line function is complete but too long to show whole: its
    // signature, the ranked evidence and its end are kept, the rest marked.
    assert_eq!((start, end), (1, 400));
    let text = primary["text"].as_str().unwrap();
    assert!(text.starts_with("pub fn process_batch() {\n"));
    assert!(text.contains("!index.contains(record.key)"));
    assert!(text.contains("persist(accepted)"));
    assert!(text.ends_with("\n}"));
    assert_eq!(primary["truncated"], false);
    assert_eq!(primary["definitionComplete"], true);
    assert_eq!(primary["omitted"], json!([[31, 115], [236, 390]]));
    let rendered = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        rendered.contains("worker.rs:1-400 (complete definition, body abridged)\n"),
        "{rendered}"
    );
    assert!(
        rendered.contains("… 85 lines omitted (worker.rs:31-115) …\n116\t"),
        "{rendered}"
    );
}

#[test]
fn normal_packet_retains_thirty_previews_and_expands_a_late_winner_in_one_call() {
    let root = tempfile::tempdir().unwrap();
    let mut full_source_bytes = 0;
    for index in 0..30 {
        let mut lines = vec![
            format!("fn authenticate_{index:02}() {{"),
            "    verify_credentials();".into(),
            "    write_session();".into(),
        ];
        for line in 0..65 {
            lines.push(format!(
                "    // authentication step {line:02}: {}",
                "source evidence keeps surrounding code"
            ));
        }
        lines.push("}".into());
        let source = lines.join("\n");
        full_source_bytes += source.len();
        fs::write(root.path().join(format!("{index:02}.rs")), source).unwrap();
    }
    assert!(
        full_source_bytes > oko::ranking::MAX_JEV_REQUEST_BYTES,
        "full candidates exceed the provider request budget"
    );
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
    assert_eq!(
        result["text"], source,
        "the affordable primary implementation is complete"
    );
    assert_eq!(result["truncated"], false);
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
    assert_eq!(results.len(), 3, "irrelevant candidates are excluded");
    assert_eq!(results[0]["path"], "password.rs");
    assert_eq!(results[0]["score"], 0.97);
    assert_eq!(results[1]["path"], "token.rs");
    assert_eq!(results[1]["score"], 0.94);
    // The uncertain candidate is not accepted. With a slot to spare in a small
    // response it is shown, labelled so it cannot pass for a match.
    assert_eq!(results[2]["path"], "test.rs");
    assert_eq!(results[2]["lowerConfidence"], true);
    assert!(results[0].get("lowerConfidence").is_none());
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("test.rs:1-2 (whole file, possible match)\n")
    );
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
fn edit_requests_keep_existing_source_and_scope_in_one_ranking_call() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("login.tsx"),
        "export function Login() { return <h1>Sign in</h1>; }",
    )
    .unwrap();
    fs::write(
        root.path().join("signup.tsx"),
        "export function Signup() { return <h1>Create account</h1>; }",
    )
    .unwrap();
    let question = "Change the login heading to Welcome aboard. Leave signup unchanged.";
    // The provider is mocked: this verifies retrieval and transport, not Jev accuracy.
    let (response, requests) =
        search_with_counted_provider(root.path(), json!({"question": question}), move |request| {
            assert_eq!(request["state"]["question"], question);
            let candidates = request["state"]["candidates"].as_array().unwrap();
            let target = candidates
                .iter()
                .find(|c| c["source"].as_str().unwrap().starts_with("login.tsx:"))
                .expect("existing source must reach the provider despite absent replacement text");
            assert!(target["text"].as_str().unwrap().contains("Sign in"));
            assert!(!target["text"].as_str().unwrap().contains("Welcome aboard"));
            relevance_response(request, |candidate| {
                if candidate["source"]
                    .as_str()
                    .unwrap()
                    .starts_with("login.tsx:")
                {
                    0.95
                } else {
                    0.2
                }
            })
        });
    assert_eq!(requests.len(), 1);
    let packet = assert_packet_envelope(&response);
    let results = packet["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["path"], "login.tsx");
    assert!(results[0]["text"].as_str().unwrap().contains("Sign in"));
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
    assert!(
        serde_json::to_vec(&application_request).unwrap().len()
            <= oko::ranking::MAX_JEV_REQUEST_BYTES
    );
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
fn packet_preserves_complete_primary_implementation_before_lower_ranked_context() {
    let root = tempfile::tempdir().unwrap();
    let implementation = format!(
        "pub fn authenticate_session(expired: bool) -> bool {{\n{}\n    if expired {{\n        return false;\n    }}\n    true\n}}",
        (0..75)
            .map(|line| format!("    record_check({line});"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(implementation.lines().count() > 60);
    fs::write(root.path().join("session.rs"), &implementation).unwrap();
    for index in 0..2 {
        let lines = (0..80)
            .map(|line| {
                format!(
                    "    // authentication alternative {line}: {}",
                    "\"\\😀\t".repeat(30)
                )
            })
            .collect::<Vec<_>>();
        fs::write(
            root.path().join(format!("alternative{index}.rs")),
            format!(
                "fn authenticate_alternative_{index}() {{\n{}\n}}",
                lines.join("\n")
            ),
        )
        .unwrap();
    }
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"Where does authentication reject an expired session?"}),
        |request| {
            for path in ["session.rs:", "alternative0.rs:", "alternative1.rs:"] {
                assert!(
                    request["state"]["candidates"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|candidate| candidate["source"].as_str().unwrap().starts_with(path)),
                    "the fixture must exercise all ranked alternatives: {path}"
                );
            }
            relevance_response(request, |candidate| {
                if candidate["source"]
                    .as_str()
                    .unwrap()
                    .starts_with("session.rs:")
                {
                    0.95
                } else {
                    0.7
                }
            })
        },
    );
    assert_eq!(requests.len(), 1, "packet expansion adds no provider calls");
    let packet = assert_packet_envelope(&response);
    let primary = &packet["results"][0];
    assert_eq!(primary["path"], "session.rs");
    assert_eq!(primary["symbol"]["name"], "authenticate_session");
    assert_eq!(primary["startLine"], 1);
    assert_eq!(primary["endLine"], implementation.lines().count());
    assert_eq!(primary["text"], implementation);
    assert_eq!(primary["truncated"], false);
    assert_eq!(
        packet["truncated"], true,
        "lower-priority context was omitted"
    );
    for result in packet["results"].as_array().unwrap() {
        let source =
            fs::read_to_string(root.path().join(result["path"].as_str().unwrap())).unwrap();
        let start = result["startLine"].as_u64().unwrap() as usize;
        let end = result["endLine"].as_u64().unwrap() as usize;
        let expected = source
            .lines()
            .skip(start - 1)
            .take(end - start + 1)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            result["text"], expected,
            "returned spans must be exact source"
        );
    }
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
        assert!(!results.is_empty(), "the best match must survive fitting");
        assert!(
            results[0]["text"]
                .as_str()
                .unwrap()
                .contains("// authentication"),
            "JSON escaping must not reduce the primary result to just a function header"
        );
        assert_eq!(results[0]["truncated"], true);
        assert_eq!(packet["truncated"], true);
    }
}

#[test]
fn cached_syntax_supports_validators_with_one_provider_call_and_bounded_wire_output() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("decode.ts"), "import { TextRule, PayloadRule } from './rules.js';\nexport function decodePayload(input: string): unknown | null {\n const text = TextRule.parse(input);\n return PayloadRule.parse(JSON.parse(text));\n}\n").unwrap();
    fs::write(root.path().join("rules.ts"), "export const TextRule = text().min(1);\nexport const PayloadRule = object({ tick: number().finite(), id: text().min(1) });\n").unwrap();
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"decode payload and validate text rules"}),
        |request| {
            relevance_response(request, |candidate| {
                if candidate["source"]
                    .as_str()
                    .unwrap()
                    .starts_with("decode.ts:")
                {
                    0.95
                } else {
                    0.01
                }
            })
        },
    );
    assert_eq!(requests.len(), 1);
    let packet = assert_packet_envelope(&response);
    // The five-line file is one definition-aligned chunk; its import names
    // the question's words but does not choose what is shown, so the answer
    // is the function, complete, and the imported rules come as related.
    assert_eq!(packet["results"][0]["startLine"], 2);
    assert_eq!(packet["results"][0]["endLine"], 5);
    assert_eq!(packet["results"][0]["definitionComplete"], true);
    assert_eq!(packet["results"][0]["truncated"], false);
    let related = packet["related"].as_array().unwrap();
    assert_eq!(related.len(), 2);
    assert!(
        related
            .iter()
            .all(|value| value["path"] == "rules.ts" && value["relation"] == "resolved_definition")
    );
    assert!(
        related
            .iter()
            .any(|value| value["text"].as_str().unwrap().contains("finite"))
    );
}

#[test]
fn empty_search_recovers_unseen_candidates_once_and_preserves_constraints() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    // More files than the shortlist holds, so recovery has unseen candidates.
    for index in 0..80 {
        fs::write(
            root.path().join(format!("parcel{index:02}.rs")),
            "fn parcel_dispatch() { deliver_parcel(); }\n",
        )
        .unwrap();
    }
    let calls = AtomicUsize::new(0);
    let question = "parcel dispatch excluding canceled deliveries";
    let (response, requests) =
        search_with_counted_provider(root.path(), json!({"question":question}), move |request| {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            relevance_response(request, |candidate| {
                if attempt == 1 && candidate["id"] == "8" {
                    0.95
                } else {
                    0.1
                }
            })
        });
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request["state"]["question"], question);
        assert!(request["state"].get("implementationCriteria").is_some());
        assert!(serde_json::to_vec(request).unwrap().len() <= 32_100);
    }
    let initial = requests[0]["state"]["candidates"].as_array().unwrap();
    let recovered = &requests[1]["state"]["candidates"][8];
    assert!(
        !initial
            .iter()
            .any(|item| item["source"] == recovered["source"])
    );
    let packet = &response["metrics"];
    assert_eq!(packet["retrieval"]["attempts"], 2);
    assert_eq!(packet["retrieval"]["recovered"], true);
    assert_eq!(packet["results"].as_array().unwrap().len(), 1);
    assert!(
        recovered["source"]
            .as_str()
            .unwrap()
            .starts_with(packet["results"][0]["path"].as_str().unwrap())
    );
}

#[test]
fn persistent_miss_stops_after_two_requests_without_lowering_threshold() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..80 {
        fs::write(
            root.path().join(format!("parcel{index:02}.rs")),
            "fn parcel_dispatch() {}\n",
        )
        .unwrap();
    }
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"parcel dispatch"}),
        |request| relevance_response(request, |_| 0.5),
    );
    assert_eq!(requests.len(), 2);
    let packet = &response["metrics"];
    assert_eq!(packet["retrieval"]["attempts"], 2);
    assert_eq!(packet["retrieval"]["recovered"], false);
    assert!(packet["results"].as_array().unwrap().is_empty());
}

#[test]
fn empty_search_skips_identical_recovery_evidence() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("parcel.rs"), "fn parcel_dispatch() {}\n").unwrap();
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"parcel dispatch"}),
        |request| relevance_response(request, |_| 0.1),
    );
    assert_eq!(requests.len(), 1);
    let packet = &response["metrics"];
    assert_eq!(packet["retrieval"]["attempts"], 1);
    assert!(packet["results"].as_array().unwrap().is_empty());
}

#[test]
fn a_named_definition_is_shown_even_when_the_ranker_rejects_everything() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("next-server.ts"),
        "import { BaseServer } from './base-server';\nexport default class NextNodeServer extends BaseServer {\n  handle() {\n    return 1;\n  }\n}\n",
    )
    .unwrap();
    // A file that repeats the words of the question wins the keyword shortlist.
    fs::write(
        root.path().join("base-server.ts"),
        "// Server Server Server: the base server every server subclass extends.\nexport class BaseServer {\n  serve() { return 'server'; }\n}\n".repeat(3),
    )
    .unwrap();
    let (response, requests) = search_with_counted_provider(
        root.path(),
        json!({"question":"NextNodeServer subclass extending base Server"}),
        |request| relevance_response(request, |_| 0.05),
    );
    assert!(!requests.is_empty());
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("next-server.ts:2-6 (complete definition, exact name match)\n"),
        "{text}"
    );
    assert!(!text.contains("No relevant code found"), "{text}");
    let packet = assert_packet_envelope(&response);
    assert_eq!(packet["results"][0]["exactName"], true);
    assert_eq!(packet["floor"]["pins"][0]["qualified"], "NextNodeServer");
    assert_eq!(packet["floor"]["identifiers"][0], "NextNodeServer");
    // The pin was judged with the shortlist, so it is listed among the candidates.
    let judged = packet["retrieval"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["path"] == "next-server.ts" && c["startLine"] == 2 && c["endLine"] == 6);
    assert!(judged, "{}", packet["retrieval"]["candidates"]);
}

#[test]
fn the_coverage_line_counts_skipped_files_and_names_one_the_question_mentions() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("auth.rs"),
        "fn authenticate() { validate_token(); }\n",
    )
    .unwrap();
    fs::write(
        root.path().join("ledger.txt"),
        vec![b'x'; oko::search::MAX_FILE_BYTES + 1],
    )
    .unwrap();
    fs::write(root.path().join("blob.bin"), b"auth\0enticate").unwrap();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let plain = client.search(json!({"question":"authentication token"}));
    let text = plain["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("Index: 1 of 3 files (2 skipped: 1 over size, 1 unreadable), "),
        "{text}"
    );
    assert!(!text.contains("skipped: ledger.txt"), "{text}");
    let mentioned = client.search(json!({"question":"authentication token in the ledger"}));
    let text = mentioned["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains(" · skipped: ledger.txt (257 KiB)\n"),
        "{text}"
    );
    let metrics = &mentioned["metrics"]["coverage"];
    assert_eq!(metrics["files"]["discovered"], 3);
    assert_eq!(metrics["files"]["indexed"], 1);
    assert_eq!(metrics["files"]["skippedUnreadable"], 1);
    assert_eq!(metrics["files"]["skippedOverSize"][0][0], "ledger.txt");
}

#[test]
fn callers_and_tests_questions_get_listings_instead_of_ranked_excerpts() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("src")).unwrap();
    fs::create_dir_all(root.path().join("tests")).unwrap();
    fs::write(
        root.path().join("src/auth.py"),
        "def authenticate(token):\n    return validate(token)\n\ndef login(request):\n    # authenticate first\n    return authenticate(request.token)\n",
    )
    .unwrap();
    fs::write(
        root.path().join("src/api.py"),
        "from .auth import authenticate\n\nclass Api:\n    def handle(self, request):\n        return authenticate(request)\n",
    )
    .unwrap();
    fs::write(
        root.path().join("tests/test_auth.py"),
        "from src.auth import authenticate\n\ndef test_authenticate():\n    assert authenticate('x')\n",
    )
    .unwrap();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let response = client.search(json!({"question":"who calls authenticate"}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(
        text.starts_with("Callers of authenticate — 2 calls, 1 import; 1 in comments, 2 in tests hidden\nDefined at src/auth.py:1\n"),
        "{text}"
    );
    assert!(
        text.contains("\nsrc/auth.py\n  6\tcall\tlogin\treturn authenticate(request.token)\n"),
        "{text}"
    );
    assert!(text.contains("\nsrc/api.py\n  1\timport\t-\tfrom .auth import authenticate\n  5\tcall\tApi.handle\treturn authenticate(request)\n"), "{text}");
    assert!(!text.contains("No relevant code found"), "{text}");
    assert_eq!(response["metrics"]["retrieval"]["usages"]["calls"], 2);
    // The explicit intent works without the phrasing.
    let explicit = client.search(json!({"question":"authenticate","intent":"callers"}));
    assert!(
        body(explicit["result"]["content"][0]["text"].as_str().unwrap())
            .starts_with("Callers of authenticate — 2 calls")
    );
    // "Definition and callers": the listing accompanies the ranked code.
    let mixed = client.search(json!({"question":"authenticate function definition and callers"}));
    let text = body(mixed["result"]["content"][0]["text"].as_str().unwrap());
    assert!(
        text.starts_with("Callers of authenticate — 2 calls"),
        "{text}"
    );
    assert!(text.contains("src/auth.py:1-"), "{text}");
    // Tests for X: paired by name and by mention, ahead of the ranked code.
    let tests = client.search(json!({"question":"tests for authenticate"}));
    let text = body(tests["result"]["content"][0]["text"].as_str().unwrap());
    assert!(text.starts_with("Tests for authenticate:\n  tests/test_auth.py:1 — named after it, mentions it, high: L1, test_authenticate (L4)\n"), "{text}");
    assert!(
        text.contains("src/auth.py:1-"),
        "the definition still follows: {text}"
    );
}

#[test]
fn symbols_and_modes_answer_by_name_and_widely_used_names_get_a_dependents_line() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("app/models")).unwrap();
    fs::create_dir_all(root.path().join("spec")).unwrap();
    fs::write(
        root.path().join("app/models/upload.rb"),
        "class Upload < ActiveRecord::Base\n  def url\n    1\n  end\nend\n",
    )
    .unwrap();
    for i in 0..5 {
        fs::write(
            root.path().join(format!("app/models/thing{i}.rb")),
            format!("class Thing{i}\n  belongs_to :file, class_name: 'Upload'\n  def pick; Upload.find(1); end\nend\n"),
        )
        .unwrap();
    }
    fs::write(
        root.path().join("spec/upload_spec.rb"),
        "describe Upload do\n  it { Upload.new }\nend\n",
    )
    .unwrap();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    // Names alone: whole definitions, in order, no ranking; a Codex-style string list too.
    for arguments in [
        json!({"symbols":"Upload, Thing2.pick"}),
        json!({"symbols":"Upload Thing2.pick"}),
    ] {
        let response = client.search(arguments);
        assert_eq!(response["result"]["isError"], false, "{response}");
        let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
        assert!(
            text.contains("app/models/upload.rb:1-5 (whole file, exact name match)\n"),
            "{text}"
        );
        assert!(
            text.contains("app/models/thing2.rb:3-3 (complete definition, exact name match)\n"),
            "{text}"
        );
        let paths: Vec<_> = response["metrics"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(paths, ["app/models/upload.rb", "app/models/thing2.rb"]);
    }
    // A missing name is reported, the found ones still answer.
    let response = client.search(json!({"symbols":"Upload, Nope"}));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("`Nope`: no definition in the index.\n"),
        "{text}"
    );
    // A widely used definition carries its dependents in one line; a new
    // session, since this one already had the line.
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let response = client.search(json!({"question":"Upload model class definition"}));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("`Upload` is used by 5 files (10 uses): app/models/thing0.rb:2 (2), "),
        "{text}"
    );
    assert!(
        text.contains("; 1 test files. Ask \"who uses Upload\" for every file with path:line and the enclosing definition.\n"),
        "{text}"
    );
    // Asked again in the same session, the line is not repeated.
    let response = client.search(json!({"question":"Upload model class definition"}));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("is used by"), "{text}");
    // mode: enumerate lists the files; mode: usages the lines. Later in the
    // session there is no coverage line, and no blank line above the listing.
    let response = client.search(json!({"symbols":"Upload","mode":"enumerate"}));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("Files using Upload — 5 files, 10 uses in code; 1 test files (2 uses). "),
        "{text}"
    );
    assert!(
        text.contains("\napp/models (5 files)\n  app/models/thing0.rb:2\tThing0\tbelongs_to :file, class_name: 'Upload'\n  app/models/thing0.rb:3\tThing0.pick\tdef pick; Upload.find(1); end\n"),
        "{text}"
    );
    let response = client.search(json!({"question":"Upload","mode":"usages"}));
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(
        text.starts_with("Callers of Upload — 10 references; 2 in tests hidden\n"),
        "{text}"
    );
    // In a batch, a callers question gets its listing, as single questions
    // do, and the listing replaces the one-line summary for that definition.
    // A fresh session, so the line is absent because of the listing.
    let mut fresh = Client::start(root.path(), true, None);
    fresh.initialize();
    let response = fresh.search(json!({"questions":["Upload model definition","who uses Upload"]}));
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(!text.contains("is used by"), "{text}");
    assert!(text.contains("Q2: Callers of Upload — "), "{text}");
    // A listing for another definition leaves the line in place.
    let mut fresh = Client::start(root.path(), true, None);
    fresh.initialize();
    let response = fresh.search(json!({"questions":["Upload model definition","who uses Thing0"]}));
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(text.contains("`Upload` is used by 5 files"), "{text}");
    assert!(text.contains("Q2: "), "{text}");
    // mode: unused needs no name and lists what nothing uses.
    let response = client.search(json!({"mode":"unused","question":"dead code"}));
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(
        text.starts_with("Unused in production code under the workspace — "),
        "{text}"
    );
    assert!(text.contains("Confirm before deleting."), "{text}");
    // Neither a question nor symbols is an error, as is mode without a name.
    let response = client.search(json!({"symbols":" "}));
    assert_eq!(response["result"]["isError"], true, "{response}");
    let response = client.search(json!({"question":"how are files stored","mode":"enumerate"}));
    assert_eq!(response["result"]["isError"], true, "{response}");
}

#[test]
fn a_batch_listing_covers_every_definition_of_its_name() {
    // Uses are counted by name, so a listing for `Upload.url` already holds
    // the files the one-line summary of `Thing.url` would name.
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("app/models")).unwrap();
    fs::write(
        root.path().join("app/models/upload.rb"),
        "class Upload\n  def url\n    1\n  end\nend\n",
    )
    .unwrap();
    fs::write(
        root.path().join("app/models/thing.rb"),
        "class Thing\n  def url\n    2\n  end\nend\n",
    )
    .unwrap();
    for i in 0..5 {
        fs::write(
            root.path().join(format!("app/models/user{i}.rb")),
            format!("class User{i}\n  def link(x); x.url; end\nend\n"),
        )
        .unwrap();
    }
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let response =
        client.search(json!({"questions":["Thing.url definition","who calls Upload.url"]}));
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(text.contains("Q2: Callers of Upload.url"), "{text}");
    assert!(!text.contains("is used by"), "{text}");
}

#[test]
fn an_impact_listing_is_a_stub_when_repeated_and_whole_when_asked_by_name() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("app/models")).unwrap();
    fs::write(
        root.path().join("app/models/upload.rb"),
        "class Upload < ActiveRecord::Base\n  def url\n    1\n  end\nend\n",
    )
    .unwrap();
    for i in 0..6 {
        fs::write(
            root.path().join(format!("app/models/thing{i}.rb")),
            format!("class Thing{i}\n  def pick; Upload.find(1); end\nend\n"),
        )
        .unwrap();
    }
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let impact = json!({"question":"what depends on Upload"});
    let first = client.search(impact.clone());
    let first = first["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(first.contains("Files using Upload — 6 files"), "{first}");
    // The same impact question again: one line naming the earlier answer.
    let second = client.search(impact);
    let second = second["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        second.contains("Files using Upload: listed in an earlier answer (6 files). Not repeated;"),
        "{second}"
    );
    assert!(!second.contains("thing3.rb:2"), "{second}");
    // Asked for by name, the listing is whole again.
    let named = client.search(json!({"symbols":"Upload","mode":"enumerate"}));
    let named = named["result"]["content"][0]["text"].as_str().unwrap();
    assert!(named.contains("app/models/thing3.rb:2\t"), "{named}");
}

#[test]
fn more_than_eight_questions_with_symbols_and_mode_answer_the_first_eight() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("app/models")).unwrap();
    fs::write(
        root.path().join("app/models/upload.rb"),
        "class Upload < ActiveRecord::Base\n  def url\n    1\n  end\nend\n",
    )
    .unwrap();
    for i in 0..5 {
        fs::write(
            root.path().join(format!("app/models/thing{i}.rb")),
            format!("class Thing{i}\n  def pick; Upload.find(1); end\nend\n"),
        )
        .unwrap();
    }
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let questions: Vec<String> = (1..=10)
        .map(|i| format!("how does thing {i} pick an upload"))
        .collect();
    let response = client.search(json!({"questions":questions,"symbols":"Upload","mode":"usages"}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Answered the first 8 of 10 questions; send the rest in another call.\n"),
        "{text}"
    );
    assert!(text.contains("Callers of Upload — "), "{text}");
    assert_eq!(
        response["metrics"]["retrieval"]["questions"]
            .as_array()
            .map(Vec::len),
        Some(8),
        "{response}"
    );
}

#[test]
fn several_questions_share_one_call_with_labelled_excerpts_and_one_set_of_side_requests() {
    let root = tempfile::tempdir().unwrap();
    for (name, body) in [
        (
            "auth",
            "pub fn authenticate_token(token: &str) -> bool {\n    validate_signature(token)\n}\n",
        ),
        (
            "routes",
            "pub fn dispatch_route(path: &str) -> Handler {\n    lookup_route_table(path)\n}\n",
        ),
        (
            "cache",
            "pub fn evict_cache_entry(key: &str) {\n    drop_entry(key)\n}\n",
        ),
    ] {
        fs::write(root.path().join(format!("{name}.rs")), body).unwrap();
    }
    let (response, requests, beside) = search_with_provider(
        root.path(),
        json!({"questions":["where is the auth token validated", "how is a route dispatched", "how are cache entries evicted"]}),
        |request| {
            relevance_response(request, |candidate| {
                let source = candidate["source"].as_str().unwrap();
                // The user's question is somewhere in the request text.
                let asked = request.to_string();
                let want = if asked.contains("auth token") {
                    "auth.rs"
                } else if asked.contains("route dispatched") {
                    "routes.rs"
                } else {
                    "cache.rs"
                };
                if source.starts_with(want) { 0.9 } else { 0.05 }
            })
        },
        |_| None,
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    // One shortlist request per question; only the leading question sends the
    // requests judged beside it.
    assert_eq!(requests.len(), 3, "{}", requests.len());
    assert!(
        beside.len() <= 2,
        "{:?}",
        beside.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(text.contains("auth.rs:1-3 (whole file, Q1)\n"), "{text}");
    assert!(text.contains("routes.rs:1-3 (whole file, Q2)\n"), "{text}");
    assert!(text.contains("cache.rs:1-3 (whole file, Q3)\n"), "{text}");
    assert!(
        text.find("Q1").unwrap() < text.find("Q2").unwrap(),
        "the first question leads: {text}"
    );
    let packet = assert_packet_envelope(&response);
    assert_eq!(
        packet["retrieval"]["questions"].as_array().unwrap().len(),
        3
    );
    assert_eq!(packet["retrieval"]["questions"][1]["tag"], "Q2");
    // 16,000 for one question and 5,000 for each further one.
    assert_eq!(packet["responseLimitBytes"], 16_000 + 2 * 5_000);
    // One question in `questions` is a plain question; questions with deep
    // is still an error.
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let response = client.search(json!({"questions":["authenticate token"]}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(text.contains("auth.rs:1-3 (whole file)\n"), "{text}");
    let response = client.search(json!({"questions":["a b", "c d"],"deep":true}));
    assert_eq!(response["result"]["isError"], true, "{response}");
    // More than eight: the first eight are answered, and the answer says so.
    let ten: Vec<String> = (0..10).map(|i| format!("authenticate token {i}")).collect();
    let response = client.search(json!({"questions": ten}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Answered the first 8 of 10 questions; send the rest in another call."),
        "{text}"
    );
    // `symbols` beside several questions: the named definition comes too.
    let response = client.search(
        json!({"questions":["authenticate token", "dispatch route"],"symbols":"evict_cache_entry"}),
    );
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(
        text.contains("cache.rs:1-3 (whole file, exact name match, symbols)\n"),
        "{text}"
    );
    // Lexical mode answers several questions too, with labels.
    let response = client.search(json!({"questions":["authenticate token", "dispatch route"]}));
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(
        text.contains("auth.rs:1-3 (whole file, Q1)\n")
            && text.contains("routes.rs:1-3 (whole file, Q2)\n"),
        "{text}"
    );
}

#[test]
fn a_repeated_long_excerpt_becomes_a_citable_stub_unless_asked_for_by_name() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mut body = String::from("pub fn reconcile_inventory_records(store: &Store) -> usize {\n");
    for i in 0..20 {
        body.push_str(&format!(
            "    let batch_{i} = store.inventory_batch({i});\n"
        ));
    }
    body.push_str("    0\n}\n");
    fs::write(root.path().join("inventory.rs"), &body).unwrap();
    fs::write(root.path().join("other.rs"), "pub fn unrelated() {}\n").unwrap();
    let mut client = Client::start_with_cache(root.path(), true, None, cache.path());
    client.initialize();
    let question = json!({"question":"how are inventory records reconciled"});
    let first = client.search(question.clone());
    let first = first["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(first.contains("inventory_batch(19)"), "{first}");
    // The same excerpt again: location and first line, not the body.
    let second = client.search(question.clone());
    let second = second["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        second.contains("inventory.rs:1-23 (shown in an earlier answer)\n1\tpub fn reconcile_inventory_records(store: &Store) -> usize {\nNot repeated. To see it again, search with symbols: \"reconcile_inventory_records\".\n"),
        "{second}"
    );
    assert!(!second.contains("inventory_batch(19)"), "{second}");
    // The coverage line leads the session's first answer only.
    assert!(first.starts_with("Index: "), "{first}");
    assert!(!second.starts_with("Index: "), "{second}");
    // Asked for by name, it is always whole.
    let named = client.search(json!({"symbols":"reconcile_inventory_records"}));
    let named = named["result"]["content"][0]["text"].as_str().unwrap();
    assert!(named.contains("inventory_batch(19)"), "{named}");
    // A new server remembers nothing.
    let mut fresh = Client::start_with_cache(root.path(), true, None, cache.path());
    fresh.initialize();
    let again = fresh.search(question);
    let again = again["result"]["content"][0]["text"].as_str().unwrap();
    assert!(again.contains("inventory_batch(19)"), "{again}");
}

#[test]
fn a_question_answered_by_a_wide_listing_brings_one_excerpt_at_most() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("app/models")).unwrap();
    fs::write(
        root.path().join("app/models/upload.rb"),
        "class Upload < ActiveRecord::Base\n  def url\n    1\n  end\nend\n",
    )
    .unwrap();
    for i in 0..15 {
        fs::write(
            root.path().join(format!("app/models/user{i}.rb")),
            format!("class User{i}\n  def avatar_upload\n    Upload.find({i})\n  end\nend\n"),
        )
        .unwrap();
    }
    fs::write(
        root.path().join("app/models/cooking.rb"),
        "class Cooking\n  def cook_post_markdown(raw)\n    raw.strip\n  end\nend\n",
    )
    .unwrap();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let response =
        client.search(json!({"questions":["how is post markdown cooked", "who uses Upload"]}));
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = body(response["result"]["content"][0]["text"].as_str().unwrap());
    assert!(text.contains("Q2: Files using Upload — 15 files"), "{text}");
    // Excerpt headers end in a parenthesised note that names their questions.
    let q2_excerpts = text
        .lines()
        .filter_map(|line| line.strip_suffix(')')?.rsplit_once('('))
        .filter(|(_, note)| note.contains("Q2"))
        .count();
    assert!(q2_excerpts <= 1, "{q2_excerpts}: {text}");
    assert!(
        text.contains("app/models/cooking.rb:2-4 (complete definition, Q1)"),
        "{text}"
    );
    assert_eq!(response["metrics"]["retrieval"]["slimmedForListing"], 1);
}

#[test]
fn a_repeated_batch_listing_states_its_file_count_and_unused_sees_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("app/models")).unwrap();
    fs::create_dir_all(root.path().join("lib")).unwrap();
    fs::write(
        root.path().join("app/models/upload.rb"),
        "class Upload < ActiveRecord::Base\n  def url\n    1\n  end\nend\n",
    )
    .unwrap();
    // Fifteen files, each with two methods using Upload: 30 rows, 15 files.
    for i in 0..15 {
        fs::write(
            root.path().join(format!("app/models/user{i}.rb")),
            format!(
                "class User{i}\n  def avatar\n    Upload.find({i})\n  end\n  def banner\n    Upload.last\n  end\nend\n"
            ),
        )
        .unwrap();
    }
    // A helper in lib/ used only from app/.
    fs::write(
        root.path().join("lib/formatting.rb"),
        "module Formatting\n  def self.shorten_upload_name(name)\n    name[0, 10]\n  end\nend\n",
    )
    .unwrap();
    fs::write(
        root.path().join("app/models/label.rb"),
        "class Label\n  def text\n    Formatting.shorten_upload_name('x')\n  end\nend\n",
    )
    .unwrap();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    let batch = json!({"questions":["what depends on Upload", "how are labels rendered"]});
    let first = client.search(batch.clone());
    let first = body(first["result"]["content"][0]["text"].as_str().unwrap()).to_owned();
    assert!(
        first.contains("Q1: Files using Upload — 15 files"),
        "{first}"
    );
    let second = client.search(batch);
    let second = body(second["result"]["content"][0]["text"].as_str().unwrap()).to_owned();
    assert!(
        second.contains("Files using Upload: listed in an earlier answer (15 files)."),
        "{second}"
    );
    // `mode: unused` in a batch scoped to lib/ counts uses from app/ too.
    let response = client.search(json!({
        "questions":["unused helpers", "formatting module"],
        "mode":"unused",
        "directory":"lib"
    }));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Unused in production code under lib"),
        "{text}"
    );
    assert!(!text.contains("shorten_upload_name —"), "{text}");
}

#[test]
fn searches_running_together_never_get_stubs() {
    use std::sync::atomic::Ordering;
    // Subagents share one server and not their context: an excerpt one of
    // them saw must reach another in full, including the first search of a
    // parallel group, which starts before the others arrive.
    let root = tempfile::tempdir().unwrap();
    let mut body_text = String::from("pub fn authenticate_session(token: &str) -> bool {\n");
    for i in 0..20 {
        body_text.push_str(&format!("    let step_{i} = check(token, {i});\n"));
    }
    body_text.push_str("    true\n}\n");
    fs::write(root.path().join("auth.rs"), &body_text).unwrap();
    let (endpoint, most, done, server) = slow_provider();
    let mut client = Client::start(root.path(), false, Some(&endpoint));
    client.initialize();
    let question = json!({"question":"how is a session authenticated"});
    let first = client.search(question.clone());
    assert!(
        first["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("step_19"),
        "{first}"
    );
    let ids: Vec<u64> = (1..=3).map(|n| 200 + n).collect();
    for id in &ids {
        client.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
            "params":{"name":"search","arguments":question.clone()}}));
    }
    let mut responses = std::collections::HashMap::new();
    while responses.len() < ids.len() {
        let response = client
            .output
            .recv_timeout(Duration::from_secs(15))
            .expect("MCP response timed out");
        if let Some(id) = response["id"].as_u64() {
            responses.insert(id, response);
        }
    }
    done.store(true, Ordering::Release);
    server.join().unwrap();
    assert!(
        most.load(Ordering::SeqCst) > 1,
        "searches ran one at a time"
    );
    for id in &ids {
        let text = responses[id]["result"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(!text.contains("shown in an earlier answer"), "{id}: {text}");
        assert!(text.contains("step_19"), "{id}: {text}");
    }
}

#[test]
fn a_long_question_for_unused_code_and_callers_gets_both_listings() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("lib.rs"),
        "pub fn parse_header(raw: &str) -> usize {\n    raw.len()\n}\n\nfn orphan_helper() -> u8 {\n    1\n}\n",
    )
    .unwrap();
    fs::write(
        root.path().join("main.rs"),
        "fn main() {\n    let n = parse_header(\"x\");\n    println!(\"{n}\");\n}\n",
    )
    .unwrap();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    // Over 120 characters: both listings accompany the ranked code.
    let question = "Before the cleanup PR I need two things from this crate: which helpers are never called anywhere, and every caller of parse_header with its line";
    let response = client.search(json!({"question": question}));
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Unused in production code under the workspace"),
        "{text}"
    );
    assert!(text.contains("orphan_helper"), "{text}");
    assert!(text.contains("Callers of parse_header"), "{text}");
}

/// A repository where one long function answers the upload question.
fn upload_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let mut body = String::from("pub fn retry_upload(job: &Job) -> Result<(), Error> {\n");
    for attempt in 0..14 {
        body.push_str(&format!(
            "    if send(job).is_ok() {{ return Ok(()); }} // attempt {attempt} after a timeout\n"
        ));
    }
    body.push_str("    Err(Error::GaveUp)\n}\n");
    fs::write(root.path().join("uploads.rs"), body).unwrap();
    fs::write(
        root.path().join("weather.rs"),
        "pub fn forecast() -> u8 {\n    7\n}\n",
    )
    .unwrap();
    root
}

/// A provider that serves every request of a session, scoring candidates
/// with `score`; returns its endpoint and each request's phase.
fn session_provider(
    score: impl Fn(&Value) -> f64 + Send + Sync + 'static,
) -> (
    String,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::{
        io::Read,
        net::TcpListener,
        sync::{Arc, Mutex, atomic::AtomicBool, atomic::Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let phases = Arc::new(Mutex::new(Vec::new()));
    let done = Arc::new(AtomicBool::new(false));
    let (seen, stop) = (phases.clone(), done.clone());
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(_) => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
            };
            stream.set_nonblocking(false).unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let (request, headers) = loop {
                let n = stream.read(&mut buffer).unwrap();
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
                        let body: Value =
                            serde_json::from_slice(&bytes[end + 4..end + 4 + len]).unwrap();
                        break (body, headers);
                    }
                }
            };
            let phase = headers
                .lines()
                .find_map(|line| line.strip_prefix("x-oko-phase: "))
                .unwrap_or("")
                .to_owned();
            seen.lock().unwrap().push(phase);
            let body = serde_json::to_vec(&relevance_response(&request, &score)).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (endpoint, phases, done)
}

fn hook_context(response: &Value) -> Option<String> {
    assert_eq!(response["result"]["isError"], false, "{response}");
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let hook: Value = serde_json::from_str(text).expect("a prefetch answers in hook JSON");
    if hook == json!({}) {
        return None;
    }
    assert_eq!(
        hook["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    Some(
        hook["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}

#[test]
fn a_code_prompt_is_answered_before_the_first_turn_and_other_prompts_are_not() {
    let root = upload_fixture();
    let (endpoint, phases, done) = session_provider(|candidate| {
        if candidate["text"]
            .as_str()
            .unwrap_or("")
            .contains("retry_upload")
        {
            0.9
        } else {
            0.1
        }
    });
    let mut client = Client::start(root.path(), false, Some(&endpoint));
    client.initialize();
    let listed = client.request("tools/list", json!({}));
    let properties = &listed["result"]["tools"][0]["inputSchema"]["properties"];
    assert!(properties.get("question").is_some());
    assert!(
        properties.get("prefetch").is_none(),
        "the hook's argument stays hidden"
    );

    for prompt in ["thanks!", "commit this and push", "/compact"] {
        let response = client.search(json!({"question": prompt, "prefetch": "s1"}));
        assert_eq!(hook_context(&response), None, "{prompt}");
        assert_eq!(response["metrics"]["prefetch"]["decision"], "skip");
    }
    assert!(phases.lock().unwrap().is_empty(), "no Jev call for chores");

    let prompt =
        "Where does the uploader retry a failed upload after a timeout?\n\nAnswer in JSON.";
    let response = client.search(json!({"question": prompt, "prefetch": "s1"}));
    let context = hook_context(&response).expect("a code prompt is answered");
    assert!(
        context.starts_with("Oko answer for this prompt."),
        "{context}"
    );
    assert!(context.contains("uploads.rs:1-"), "{context}");
    assert!(context.contains("pub fn retry_upload"), "{context}");
    assert!(!context.contains("forecast"), "{context}");
    assert!(context.chars().count() <= 8_000);
    assert_eq!(response["metrics"]["prefetch"]["decision"], "inject");
    assert_eq!(
        response["metrics"]["question"],
        "Where does the uploader retry a failed upload after a timeout?"
    );
    // One request: nothing is judged beside the shortlist.
    assert_eq!(*phases.lock().unwrap(), vec!["normal"]);

    // A resent prompt is not searched again.
    let again = client.search(json!({"question": prompt, "prefetch": "s1"}));
    assert_eq!(hook_context(&again), None);
    assert_eq!(again["metrics"]["prefetch"]["reason"], "repeat");

    // The agent's own search knows the code was sent.
    let search =
        client.search(json!({"question": "Where does the uploader retry a failed upload?"}));
    let text = search["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("uploads.rs:1-17 (shown in an earlier answer)"),
        "{text}"
    );

    // A new session starts from nothing: the same prompt is answered again.
    let fresh = client.search(json!({"question": prompt, "prefetch": "s2"}));
    assert!(
        hook_context(&fresh)
            .unwrap()
            .contains("pub fn retry_upload")
    );
    done.store(true, std::sync::atomic::Ordering::Release);
}

#[test]
fn without_jev_a_prefetch_shows_only_definitions_the_prompt_names() {
    let root = upload_fixture();
    let mut client = Client::start(root.path(), true, None);
    client.initialize();
    // Keyword order is no judgement of relevance.
    let words = client.search(json!({
        "question": "Where does the uploader retry a failed upload after a timeout?",
        "prefetch": "s1"
    }));
    assert_eq!(hook_context(&words), None);
    assert_eq!(words["metrics"]["prefetch"]["reason"], "not code");
    // A file-like name matching only a common leaf (`config`) is no name.
    for module in ["a", "b", "c", "d"] {
        fs::write(
            root.path().join(format!("{module}.rs")),
            "pub const config: u8 = 1;\n",
        )
        .unwrap();
    }
    let file = client.search(json!({
        "question": "why does the server reload when app.config changes?",
        "prefetch": "s1"
    }));
    assert_eq!(hook_context(&file), None);
    assert_eq!(file["metrics"]["prefetch"]["reason"], "not code");
    let named =
        client.search(json!({"question": "Explain how retry_upload gives up", "prefetch": "s1"}));
    let context = hook_context(&named).expect("a named definition is shown");
    assert!(context.contains("pub fn retry_upload"), "{context}");
    assert!(!context.contains("forecast"), "{context}");
}
