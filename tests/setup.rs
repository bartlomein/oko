//! Setup runs only against disposable projects and explicit temporary install directories.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use toml_edit::DocumentMut;
fn setup(exe: &Path, root: &Path, install: &Path, extra: &[&str]) -> Output {
    Command::new(exe)
        .arg("setup")
        .arg("--root")
        .arg(root)
        .arg("--install-dir")
        .arg(install)
        .args(extra)
        .env_remove("TYPESAFE_API_KEY")
        .env("TYPESAFE_BASE_URL", "http://127.0.0.1:1")
        .output()
        .unwrap()
}
fn executable() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_oko"))
}
fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn setup_installs_verifies_preserves_and_is_repeatable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project with spaces");
    let install = temp.path().join("stable bin");
    fs::create_dir_all(root.join(".codex")).unwrap();
    let original = "# Keep this comment\nmodel = \"custom-model\"\n\n[mcp_servers.other]\ncommand = \"untouched\"\n";
    fs::write(root.join(".codex/config.toml"), original).unwrap();
    fs::write(
        root.join("AGENTS.md"),
        "# Existing instructions\nPreserve this text.\n",
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "target/\n").unwrap();
    let output = setup(executable(), &root, &install, &["--no-jev"]);
    assert_ok(&output);
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("MCP startup and search-tool discovery verified")
    );
    let config = fs::read_to_string(root.join(".codex/config.toml")).unwrap();
    assert!(config.starts_with("# Keep this comment"));
    let doc: DocumentMut = config.parse().unwrap();
    assert_eq!(doc["model"].as_str(), Some("custom-model"));
    assert_eq!(
        doc["mcp_servers"]["other"]["command"].as_str(),
        Some("untouched")
    );
    let server = &doc["mcp_servers"]["oko"];
    let binary = Path::new(server["command"].as_str().unwrap());
    assert!(binary.starts_with(install.canonicalize().unwrap()));
    assert!(binary.is_file());
    assert!(Path::new(server["env"]["OKO_RIPGREP"].as_str().unwrap()).is_file());
    assert_eq!(server["tool_timeout_sec"].as_integer(), Some(120));
    assert_eq!(
        server["args"].as_array().unwrap().get(3).unwrap().as_str(),
        Some("--no-jev")
    );
    fs::write(root.join("auth.rs"), "fn authenticate() {}\n").unwrap();
    let search = Command::new(binary)
        .current_dir(&root)
        .env("PATH", "")
        .env(
            "OKO_RIPGREP",
            server["env"]["OKO_RIPGREP"].as_str().unwrap(),
        )
        .args(["ask", "authentication", "--no-jev", "--json"])
        .output()
        .unwrap();
    assert_ok(&search);
    assert!(String::from_utf8_lossy(&search.stdout).contains("auth.rs"));
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(agents.starts_with("# Existing instructions\nPreserve this text.\n"));
    assert_eq!(agents.matches("<!-- oko:search:start -->").count(), 1);
    assert!(
        fs::read_to_string(root.join(".gitignore"))
            .unwrap()
            .starts_with("target/\n")
    );
    let backup_count = fs::read_dir(install.join("setup-backups")).unwrap().count();
    assert_eq!(backup_count, 3);
    assert_ok(&setup(binary, &root, &install, &["--no-jev"]));
    assert_eq!(
        fs::read_to_string(root.join(".codex/config.toml")).unwrap(),
        config
    );
    assert_eq!(fs::read_to_string(root.join("AGENTS.md")).unwrap(), agents);
    assert_eq!(
        fs::read_dir(install.join("setup-backups")).unwrap().count(),
        backup_count
    );
}
#[test]
fn downloaded_copy_can_be_deleted_after_setup() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join(".gitignore"), "/.codex/config.toml").unwrap();
    let download = temp.path().join("downloaded-oko");
    fs::copy(executable(), &download).unwrap();
    let install = temp.path().join("installed");
    assert_ok(&setup(
        &download,
        &root,
        &install,
        &["--no-jev", "--no-instructions"],
    ));
    fs::remove_file(download).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        "/.codex/config.toml\n/.env\n"
    );
    assert!(!root.join("AGENTS.md").exists());
    let doc: DocumentMut = fs::read_to_string(root.join(".codex/config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let binary = Path::new(doc["mcp_servers"]["oko"]["command"].as_str().unwrap());
    assert_ok(
        &Command::new(binary)
            .args(["mcp", "--help"])
            .output()
            .unwrap(),
    );
}
#[test]
fn project_env_key_is_not_written_to_configuration_or_backups() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join(".env"), "TYPESAFE_API_KEY=fake-setup-secret\n").unwrap();
    let install = temp.path().join("installed");
    let output = setup(executable(), &root, &install, &[]);
    assert_ok(&output);
    for path in [
        root.join(".codex/config.toml"),
        root.join("AGENTS.md"),
        root.join(".gitignore"),
    ] {
        assert!(
            !fs::read_to_string(path)
                .unwrap()
                .contains("fake-setup-secret")
        );
    }
    assert!(!String::from_utf8_lossy(&output.stdout).contains("fake-setup-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("fake-setup-secret"));
    assert!(!install.join("setup-backups").exists());
}
#[test]
fn conflicting_or_malformed_config_is_not_overwritten() {
    for original in [
        "[invalid",
        "[mcp_servers.oko]\ncommand = 'someone-elses-server'\n",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir_all(root.join(".codex")).unwrap();
        fs::write(root.join(".codex/config.toml"), original).unwrap();
        let output = setup(executable(), &root, &temp.path().join("bin"), &["--no-jev"]);
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(root.join(".codex/config.toml")).unwrap(),
            original
        );
        assert!(!root.join("AGENTS.md").exists());
    }
}
#[test]
fn malformed_guidance_and_empty_credentials_leave_config_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("AGENTS.md"),
        "<!-- oko:search:start -->\nunfinished",
    )
    .unwrap();
    let install = temp.path().join("bin");
    assert!(
        !setup(executable(), &root, &install, &["--no-jev"])
            .status
            .success()
    );
    assert!(!root.join(".codex/config.toml").exists());
    fs::remove_file(root.join("AGENTS.md")).unwrap();
    fs::write(root.join(".env"), "TYPESAFE_API_KEY=\n").unwrap();
    let output = setup(executable(), &root, &install, &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("empty TYPESAFE_API_KEY"));
    assert!(!root.join(".codex/config.toml").exists());
}
#[cfg(unix)]
#[test]
fn symlinked_config_directory_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let outside = temp.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join(".codex")).unwrap();
    assert!(
        !setup(executable(), &root, &temp.path().join("bin"), &["--no-jev"])
            .status
            .success()
    );
    assert!(!outside.join("config.toml").exists());
}
#[test]
fn opencode_setup_merges_the_connection_and_is_repeatable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let install = temp.path().join("bin");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("opencode.json"),
        r#"{"model":"custom","mcp":{"other":{"type":"local","command":["untouched"]}}}"#,
    )
    .unwrap();
    let output = setup(
        executable(),
        &root,
        &install,
        &["--no-jev", "--client", "opencode"],
    );
    assert_ok(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Keep that change out of shared"));
    let saved = fs::read_to_string(root.join("opencode.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(doc["model"], "custom");
    assert_eq!(doc["mcp"]["other"]["command"][0], "untouched");
    let server = &doc["mcp"]["oko"];
    assert_eq!(server["type"], "local");
    assert_eq!(server["enabled"], true);
    let command = server["command"].as_array().unwrap();
    assert!(Path::new(command[0].as_str().unwrap()).is_file());
    assert_eq!(command[1], "mcp");
    assert_eq!(command[3], root.canonicalize().unwrap().to_str().unwrap());
    assert_eq!(command[4], "--no-jev");
    assert!(Path::new(server["environment"]["OKO_RIPGREP"].as_str().unwrap()).is_file());
    assert!(
        fs::read_to_string(root.join("AGENTS.md"))
            .unwrap()
            .contains("<!-- oko:search:start -->")
    );
    assert!(!root.join(".codex").exists());
    assert!(!root.join("CLAUDE.md").exists());
    // A file that was already there may be shared, so it is not ignored.
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        "/.env\n"
    );
    assert_ok(&setup(
        executable(),
        &root,
        &install,
        &["--no-jev", "--client", "opencode"],
    ));
    assert_eq!(
        fs::read_to_string(root.join("opencode.json")).unwrap(),
        saved
    );

    let fresh = temp.path().join("fresh");
    fs::create_dir(&fresh).unwrap();
    assert_ok(&setup(
        executable(),
        &fresh,
        &install,
        &["--no-jev", "--client", "opencode"],
    ));
    assert!(
        fs::read_to_string(fresh.join(".gitignore"))
            .unwrap()
            .lines()
            .any(|line| line == "/opencode.json")
    );
}
#[test]
fn opencode_configuration_setup_cannot_own_is_not_overwritten() {
    for (name, original) in [
        (
            "opencode.json",
            "{\n  // a comment\n  \"model\": \"custom\"\n}\n",
        ),
        (
            "opencode.json",
            r#"{"mcp":{"oko":{"type":"local","command":["someone-elses-server"]}}}"#,
        ),
        ("opencode.json", r#"{"mcp":{"servers":{}}}"#),
        ("opencode.jsonc", "{}"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join(name), original).unwrap();
        let output = setup(
            executable(),
            &root,
            &temp.path().join("bin"),
            &["--no-jev", "--client", "opencode"],
        );
        assert!(!output.status.success(), "{name}: {original}");
        assert_eq!(fs::read_to_string(root.join(name)).unwrap(), original);
        assert!(!root.join("AGENTS.md").exists());
        assert!(!root.join(".gitignore").exists());
    }
}
/// A stand-in for the claude command: logs each call, and answers `mcp get`
/// with the described connection when one is supplied.
#[cfg(unix)]
fn fake_claude(dir: &Path, described: Option<&str>) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-claude");
    let log = dir.join("claude.log");
    let get = match described {
        Some(text) => format!("printf '%s\\n' '{text}'; exit 0"),
        None => "echo 'No MCP server named \"oko\".' >&2; exit 1".into(),
    };
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s|' \"$@\" >> '{}'\necho >> '{}'\nif [ \"$1 $2\" = 'mcp get' ]; then {get}; fi\nexit 0\n",
            log.display(),
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}
#[cfg(unix)]
fn setup_claude(root: &Path, install: &Path, claude: &Path) -> Output {
    Command::new(executable())
        .args(["setup", "--no-jev", "--client", "claude", "--root"])
        .arg(root)
        .arg("--install-dir")
        .arg(install)
        .env("OKO_CLAUDE", claude)
        .output()
        .unwrap()
}
#[cfg(unix)]
#[test]
fn claude_setup_registers_a_local_connection_and_writes_its_instructions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project with spaces");
    let install = temp.path().join("bin");
    fs::create_dir(&root).unwrap();
    let claude = fake_claude(temp.path(), None);
    assert_ok(&setup_claude(&root, &install, &claude));
    let log = fs::read_to_string(temp.path().join("claude.log")).unwrap();
    let added = log
        .lines()
        .find(|line| line.starts_with("mcp|add-json|"))
        .unwrap();
    let installed = install.canonicalize().unwrap();
    let server: serde_json::Value = serde_json::from_str(
        added
            .strip_prefix("mcp|add-json|--scope|local|oko|")
            .and_then(|rest| rest.strip_suffix('|'))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        server,
        serde_json::json!({
            "type": "stdio",
            "command": installed.join("oko"),
            "args": ["mcp", "--root", root.canonicalize().unwrap(), "--no-jev"],
            "env": {"OKO_RIPGREP": installed.join("rg")},
            // Loaded up front, so agents and their explorers see the tool itself.
            "alwaysLoad": true,
        })
    );
    assert!(!log.contains("mcp|remove|"));
    assert!(
        fs::read_to_string(root.join("CLAUDE.md"))
            .unwrap()
            .contains("<!-- oko:search:start -->")
    );
    assert!(!root.join("AGENTS.md").exists());
    assert!(!root.join(".codex").exists());
    assert!(!root.join("opencode.json").exists());
}
#[cfg(unix)]
#[test]
fn claude_setup_replaces_its_own_connection_and_refuses_another() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    // CLAUDE.md that imports AGENTS.md: the guidance belongs in AGENTS.md.
    fs::write(root.join("CLAUDE.md"), "@AGENTS.md\n").unwrap();
    let claude = fake_claude(
        temp.path(),
        Some("oko:\n  Type: stdio\n  Command: /old/place/oko\n  Args: mcp --root /project"),
    );
    assert_ok(&setup_claude(&root, &temp.path().join("bin"), &claude));
    let log = fs::read_to_string(temp.path().join("claude.log")).unwrap();
    let removed = log.find("mcp|remove|--scope|local|oko|").unwrap();
    assert!(removed < log.find("mcp|add-json|").unwrap());
    assert_eq!(
        fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );
    assert!(root.join("AGENTS.md").exists());

    let other = tempfile::tempdir().unwrap();
    let root = other.path().join("project");
    fs::create_dir(&root).unwrap();
    let claude = fake_claude(
        other.path(),
        Some("oko:\n  Type: stdio\n  Command: npx\n  Args: someone-elses-server"),
    );
    let output = setup_claude(&root, &other.path().join("bin"), &claude);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("claude mcp remove oko"));
    assert!(
        !fs::read_to_string(other.path().join("claude.log"))
            .unwrap()
            .contains("mcp|add-json|")
    );
    assert!(!root.join("CLAUDE.md").exists());
}
#[cfg(unix)]
#[test]
fn a_missing_claude_command_or_unknown_client_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let output = setup_claude(&root, &temp.path().join("bin"), &temp.path().join("absent"));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("OKO_CLAUDE"));
    let output = setup(
        executable(),
        &root,
        &temp.path().join("bin"),
        &["--no-jev", "--client", "cursor"],
    );
    assert!(!output.status.success());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
}
#[cfg(unix)]
#[test]
fn all_clients_share_one_agents_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let claude = fake_claude(temp.path(), None);
    let output = Command::new(executable())
        .args(["setup", "--no-jev", "--client", "all", "--root"])
        .arg(&root)
        .arg("--install-dir")
        .arg(temp.path().join("bin"))
        .env("OKO_CLAUDE", &claude)
        .output()
        .unwrap();
    assert_ok(&output);
    for file in [
        ".codex/config.toml",
        "opencode.json",
        "AGENTS.md",
        "CLAUDE.md",
    ] {
        assert!(root.join(file).is_file(), "{file}");
    }
    let ignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    for entry in ["/.codex/config.toml", "/opencode.json", "/.env"] {
        assert!(ignore.lines().any(|line| line == entry), "{entry}");
    }
}
#[cfg(unix)]
#[test]
fn a_claude_md_linked_to_agents_md_gets_one_section_and_stays_a_link() {
    // Next.js and Discourse link CLAUDE.md to their agents file.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("AGENTS.md"), "# Project notes\n").unwrap();
    std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();
    let claude = fake_claude(temp.path(), None);
    let output = Command::new(executable())
        .args(["setup", "--no-jev", "--client", "claude,codex", "--root"])
        .arg(&root)
        .arg("--install-dir")
        .arg(temp.path().join("bin"))
        .env("OKO_CLAUDE", &claude)
        .output()
        .unwrap();
    assert_ok(&output);
    assert!(
        fs::symlink_metadata(root.join("CLAUDE.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert!(agents.starts_with("# Project notes\n"));
    assert_eq!(agents.matches("<!-- oko:search:start -->").count(), 1);
    // Setup is repeatable through the link, too.
    assert_ok(&setup_claude(&root, &temp.path().join("bin"), &claude));
    let again = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert_eq!(again.matches("<!-- oko:search:start -->").count(), 1);
}
#[cfg(unix)]
#[test]
fn a_claude_md_linked_outside_the_project_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let outside = temp.path().join("elsewhere.md");
    fs::write(&outside, "someone else's notes\n").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("CLAUDE.md")).unwrap();
    let claude = fake_claude(temp.path(), None);
    let output = setup_claude(&root, &temp.path().join("bin"), &claude);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("inside the project"));
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "someone else's notes\n"
    );
    // Dangling links are refused the same way.
    fs::remove_file(root.join("CLAUDE.md")).unwrap();
    std::os::unix::fs::symlink("missing.md", root.join("CLAUDE.md")).unwrap();
    assert!(
        !setup_claude(&root, &temp.path().join("bin"), &claude)
            .status
            .success()
    );
}
