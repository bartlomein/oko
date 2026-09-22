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
    assert_eq!(tools[0]["inputSchema"]["additionalProperties"], false);
    // Every agent turn pays for the tool definition, whether or not it searches.
    assert!(tools[0].get("outputSchema").is_none());
    let definition = serde_json::to_vec(&tools[0]).unwrap().len();
    assert!(
        definition <= 1_950,
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
        "candidates, not a complete answer",
    ] {
        assert!(description.contains(guidance), "{description}");
    }
    let result = client.search(json!({"question":"authentication token"}));
    assert_eq!(result["result"]["isError"], false, "{result}");
    assert_eq!(
        result["result"]["content"][0]["text"],
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
    ] {
        let result = client.search(args);
        assert_eq!(result["result"]["isError"], true, "{result}");
    }
    for args in [
        json!({}),
        json!({"question":"auth","intent":"bad"}),
        json!({"question":"auth","surprise":true}),
        json!({"question":"auth","deep":true}),
        json!({"question":"auth","max_steps":1}),
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
    assert!(instructions.len() <= 320, "{}", instructions.len());

    let scoped = client.search(json!({"question":"authentication token","directory":"server"}));
    assert_packet_envelope(&scoped);
    assert_eq!(
        scoped["result"]["content"][0]["text"],
        "Paths are relative to server/.\n\nauth.rs:1-1 (whole file)\n```\n1\tfn authenticate() { validate_token(); }\n```\n"
    );
    let unscoped = client.search(json!({"question":"authentication token"}));
    assert!(
        unscoped["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
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
fn search_uses_mock_jev_and_survives_provider_errors() {
    use std::{io::Read, net::TcpListener, time::Instant};
    for fail in [false, true] {
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
        let result = client.search(json!({"question":"authentication"}));
        server.join().unwrap();
        assert!(!result.to_string().contains("fake-mcp-key"));
        assert_eq!(result["result"]["isError"], fail, "{result}");
        if !fail {
            assert_packet_envelope(&result);
            assert_eq!(result["metrics"]["results"][0]["path"], "auth.rs");
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
            text.starts_with("The relevance ranker did not respond, so these are keyword matches"),
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
        let numbered = excerpt["text"]
            .as_str()
            .unwrap()
            .split('\n')
            .zip(excerpt["startLine"].as_u64().unwrap()..)
            .map(|(line, number)| format!("{number}\t{line}\n"))
            .collect::<String>();
        assert!(
            body[fence.len() + 1..].starts_with(&format!("{numbered}{fence}\n")),
            "{header}: {text}"
        );
    }
    if packet["results"].as_array().unwrap().is_empty() {
        assert!(text.starts_with("No relevant code found."), "{text}");
    }
    assert!(packet["results"].as_array().unwrap().len() <= 3);
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
    assert!(
        packet["timings"]["shortlistMs"]
            .as_f64()
            .is_some_and(|n| n >= 0.0)
    );
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
    assert_eq!(broad.len(), 30);
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
        assert!(serde_json::to_vec(&application_request).unwrap().len() <= 32_000);
        let candidates = requests[0]["state"]["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 30);
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
    assert_eq!(primary["text"], lines[start - 1..end].join("\n"));
    assert!(
        primary["text"]
            .as_str()
            .unwrap()
            .contains("!index.contains(record.key)")
    );
    assert!(
        primary["text"]
            .as_str()
            .unwrap()
            .contains("persist(accepted)")
    );
    assert_eq!(
        primary["truncated"], true,
        "the containing function is still incomplete"
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
        full_source_bytes > 32_000,
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
    for index in 0..40 {
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
    for index in 0..40 {
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
