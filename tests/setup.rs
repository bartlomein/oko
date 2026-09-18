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
