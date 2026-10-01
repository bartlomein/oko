//! `oko upgrade` against releases served locally: a stand-in Oko that logs how
//! it is run, packed and listed in SHA256SUMS the way releases are.
#![cfg(unix)]
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn executable() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_oko"))
}

fn target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => "x86_64-unknown-linux-gnu",
    }
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A release archive whose `oko` reports `version` and logs every other run.
fn release(work: &Path, tag: &str, version: &str, log: &Path) -> (String, Vec<u8>) {
    let bundle = format!("oko-{tag}-{}", target());
    let folder = work.join(&bundle);
    fs::create_dir_all(&folder).unwrap();
    script(
        &folder.join("oko"),
        &format!(
            "if [ \"$1\" = --version ]; then echo 'oko {version}'; exit 0; fi\nprintf '%s\\n' \"$*\" >> '{}'",
            log.display()
        ),
    );
    script(&folder.join("rg"), "echo 'ripgrep 15.2.0'");
    let name = format!("{bundle}.tar.gz");
    let status = Command::new("tar")
        .args(["-czf", &name, &bundle])
        .current_dir(work)
        .status()
        .unwrap();
    assert!(status.success());
    (name.clone(), fs::read(work.join(&name)).unwrap())
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Serves `files` by path until the test ends.
fn serve(files: HashMap<String, Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let files = Arc::new(files);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let files = files.clone();
            thread::spawn(move || {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, body) = match files.get(&path) {
                    Some(body) => ("200 OK", body.clone()),
                    None => ("404 Not Found", b"missing".to_vec()),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            });
        }
    });
    address
}

struct World {
    temp: tempfile::TempDir,
    home: PathBuf,
    stable: PathBuf,
    link: PathBuf,
    old_target: PathBuf,
    project: PathBuf,
    gone: PathBuf,
    log: PathBuf,
    state: PathBuf,
}

/// A home with the install script's `oko` link, setup's copy, and a project
/// list naming one project that exists and one that is gone.
fn world() -> World {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let home = root.join("home");
    let releases = home.join(".local/share/oko/releases/v0.0.1.abcdefgh/oko-v0.0.1-x");
    fs::create_dir_all(&releases).unwrap();
    script(&releases.join("oko"), "echo 'oko 0.0.1'");
    fs::create_dir_all(home.join(".local/bin")).unwrap();
    let link = home.join(".local/bin/oko");
    symlink(releases.join("oko"), &link).unwrap();
    let stable = root.join("stable");
    fs::create_dir_all(&stable).unwrap();
    fs::write(stable.join("oko"), "old oko").unwrap();
    fs::write(stable.join("rg"), "old rg").unwrap();
    let project = root.join("project");
    fs::create_dir_all(&project).unwrap();
    let gone = root.join("gone");
    fs::write(
        stable.join("projects.json"),
        serde_json::json!({"projects": [
            {"root": project, "clients": ["codex", "claude"], "noJev": true},
            {"root": gone, "clients": ["opencode"]},
        ]})
        .to_string(),
    )
    .unwrap();
    World {
        log: root.join("log"),
        state: root.join("update.json"),
        old_target: releases.join("oko"),
        temp,
        home,
        stable,
        link,
        project,
        gone,
    }
}

fn upgrade(world: &World, releases: &str) -> Output {
    Command::new(executable())
        .args(["upgrade", "--install-dir"])
        .arg(&world.stable)
        .env("HOME", &world.home)
        .env("OKO_RELEASES_URL", releases)
        .env("OKO_UPDATE_STATE", &world.state)
        .env_remove("OKO_INSTALL_DIR")
        .env_remove("OKO_BIN_DIR")
        .output()
        .unwrap()
}

fn files(tag: &str, archive: (String, Vec<u8>), sums: String) -> HashMap<String, Vec<u8>> {
    HashMap::from([
        (
            "/latest".to_owned(),
            serde_json::json!({"tag_name": tag})
                .to_string()
                .into_bytes(),
        ),
        (format!("/download/{tag}/SHA256SUMS"), sums.into_bytes()),
        (format!("/download/{tag}/{}", archive.0), archive.1),
    ])
}

#[test]
fn upgrade_replaces_both_copies_and_refreshes_the_listed_projects() {
    let world = world();
    let work = world.temp.path().join("release");
    let archive = release(&work, "v99.0.0", "99.0.0", &world.log);
    let sums = format!("{}  {}\n", hex(&archive.1), archive.0);
    let output = upgrade(&world, &serve(files("v99.0.0", archive, sums)));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("to 99.0.0"), "{stdout}");
    assert!(stdout.contains("Checksum verified"), "{stdout}");
    // The install script's command now runs the new release from its folder.
    let linked = fs::read_link(&world.link).unwrap();
    assert!(
        linked.ends_with(format!("oko-v99.0.0-{}/oko", target()))
            && linked
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .is_some_and(|name| name.to_string_lossy().starts_with("v99.0.0.")),
        "{}",
        linked.display()
    );
    assert!(
        world.old_target.exists(),
        "the old release stays for open sessions"
    );
    // Setup's copy is the new release, and the listed project was set up again.
    let new = fs::read(&linked).unwrap();
    assert_eq!(fs::read(world.stable.join("oko")).unwrap(), new);
    assert_eq!(
        fs::read_to_string(world.stable.join("rg")).unwrap(),
        "#!/bin/sh\necho 'ripgrep 15.2.0'\n"
    );
    let log = fs::read_to_string(&world.log).unwrap();
    assert_eq!(
        log.trim(),
        format!(
            "setup --root {} --install-dir {} --client codex,claude --no-jev --quiet",
            world.project.display(),
            world.stable.display()
        )
    );
    assert!(stdout.contains("(Codex, Claude Code): done"), "{stdout}");
    // A project whose folder is gone is forgotten.
    assert!(stdout.contains("Forgot"), "{stdout}");
    let listed = fs::read_to_string(world.stable.join("projects.json")).unwrap();
    assert!(!listed.contains(&*world.gone.to_string_lossy()), "{listed}");
    assert!(
        listed.contains(&*world.project.to_string_lossy()),
        "{listed}"
    );
    // The daily check now knows the latest release.
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&world.state).unwrap()).unwrap();
    assert_eq!(state["latest"], "v99.0.0");
}

#[test]
fn a_checksum_mismatch_changes_nothing() {
    let world = world();
    let work = world.temp.path().join("release");
    let archive = release(&work, "v99.0.0", "99.0.0", &world.log);
    let sums = format!("{}  {}\n", "0".repeat(64), archive.0);
    let output = upgrade(&world, &serve(files("v99.0.0", archive, sums)));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Checksum mismatch"));
    assert_eq!(fs::read_link(&world.link).unwrap(), world.old_target);
    assert_eq!(
        fs::read_to_string(world.stable.join("oko")).unwrap(),
        "old oko"
    );
    assert!(!world.log.exists());
    let releases = world.home.join(".local/share/oko/releases");
    assert_eq!(
        fs::read_dir(releases).unwrap().count(),
        1,
        "the staged download is removed"
    );
}

#[test]
fn a_release_reporting_another_version_is_refused() {
    let world = world();
    let work = world.temp.path().join("release");
    let archive = release(&work, "v99.0.0", "98.0.0", &world.log);
    let sums = format!("{}  {}\n", hex(&archive.1), archive.0);
    let output = upgrade(&world, &serve(files("v99.0.0", archive, sums)));
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(world.stable.join("oko")).unwrap(),
        "old oko"
    );
    assert_eq!(fs::read_link(&world.link).unwrap(), world.old_target);
}

#[test]
fn the_latest_release_needs_no_upgrade() {
    let world = world();
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let latest = HashMap::from([(
        "/latest".to_owned(),
        serde_json::json!({"tag_name": tag})
            .to_string()
            .into_bytes(),
    )]);
    let output = upgrade(&world, &serve(latest));
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("is the latest release"));
    assert_eq!(
        fs::read_to_string(world.stable.join("oko")).unwrap(),
        "old oko"
    );
    assert_eq!(fs::read_link(&world.link).unwrap(), world.old_target);
}

fn session_start(state: &Path, extra: &[(&str, &str)]) -> serde_json::Value {
    let mut command = Command::new(executable());
    command
        .args(["hook", "session-start"])
        .env("OKO_UPDATE_STATE", state)
        .env_remove("CI")
        .env_remove("OKO_NO_UPDATE_CHECK")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    for (key, value) in extra {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"session_id\":\"s\"}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn claude_code_shows_a_newer_release_at_session_start() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("update.json");
    let checked = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        &state,
        serde_json::json!({"checkedAt": checked, "latest": "v99.0.0"}).to_string(),
    )
    .unwrap();
    let started = Instant::now();
    let answer = session_start(&state, &[]);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the hook reads the last check only"
    );
    let message = answer["systemMessage"].as_str().unwrap();
    assert!(
        message.starts_with("Oko 99.0.0 is out (you have ")
            && message.ends_with("Run `oko upgrade` to update."),
        "{message}"
    );
    assert!(
        answer["hookSpecificOutput"]["additionalContext"].is_string(),
        "the agent's context is unchanged"
    );
    let answer = session_start(&state, &[("OKO_NO_UPDATE_CHECK", "1")]);
    assert!(answer.get("systemMessage").is_none(), "{answer}");
    fs::write(
        &state,
        serde_json::json!({"checkedAt": checked, "latest": format!("v{}", env!("CARGO_PKG_VERSION"))})
            .to_string(),
    )
    .unwrap();
    assert!(session_start(&state, &[]).get("systemMessage").is_none());
}
