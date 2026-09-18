//! Project-scoped Codex setup. Never writes credentials to configuration files.
use anyhow::{Context, Result, bail};
use rmcp::ServiceExt;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use toml_edit::{DocumentMut, Item, Table, value};

const USAGE: &str = "Usage: oko setup [--root DIRECTORY] [--no-jev] [--no-instructions]\n                 [--install-dir DIRECTORY]\n\nSet up Oko for Codex in the chosen project (default: current directory).\nInstalls a stable copy, checks MCP, and updates .codex/config.toml.\nAdds a managed search section to AGENTS.md unless --no-instructions is set.\n--no-jev sets up local-only search without credentials or network calls.\n--install-dir overrides the per-user application bin directory.";
const MANAGED: &str = "# Managed by oko setup";
const START: &str = "<!-- oko:search:start -->";
const END: &str = "<!-- oko:search:end -->";
const GUIDANCE: &str = "<!-- oko:search:start -->\n## Oko code search\nUse the Oko MCP search tool first when locating unfamiliar code or finding where a behavior is implemented in this project. Start with the user's wording; do not add guessed frameworks or pipeline stages. Results include source context: use that evidence directly when sufficient. Follow up only for evidence needed to answer; truncation and ambiguity flags describe limits to assess. Related definitions are lexical candidates, not a verified call graph. Use native grep for exact known identifiers or literal text. Start with normal search; use deep mode only if those results are insufficient. If Oko is unavailable or insufficient, fall back to native search.\n<!-- oko:search:end -->";

struct Options {
    root: PathBuf,
    install: PathBuf,
    offline: bool,
    instructions: bool,
}
fn options(args: &[String], cwd: &Path) -> Result<Options> {
    let (mut root, mut install, mut offline, mut instructions) = (None, None, false, true);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" if root.is_none() => {
                root = Some(args.next().context("--root needs a directory")?)
            }
            "--install-dir" if install.is_none() => {
                install = Some(args.next().context("--install-dir needs a directory")?)
            }
            "--no-jev" if !offline => offline = true,
            "--no-instructions" if instructions => instructions = false,
            _ => bail!("Invalid setup arguments.\n{USAGE}"),
        }
    }
    let root = cwd
        .join(root.map_or(".", String::as_str))
        .canonicalize()
        .context("Cannot access project directory")?;
    if !root.is_dir() {
        bail!("Project root must be a directory");
    }
    let install = match install {
        Some(path) => cwd.join(path),
        None => {
            let home = PathBuf::from(
                env::var_os("HOME").context("Cannot locate home directory; pass --install-dir")?,
            );
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Oko/bin")
            } else {
                home.join(".local/share/oko/bin")
            }
        }
    };
    Ok(Options {
        root,
        install,
        offline,
        instructions,
    })
}
fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
fn ripgrep(exe: &Path) -> Result<PathBuf> {
    let name = executable_name("rg");
    let path_env = env::var_os("PATH").unwrap_or_default();
    let candidates = env::var_os("OKO_RIPGREP")
        .map(PathBuf::from)
        .into_iter()
        .chain(exe.parent().map(|p| p.join(&name)))
        .chain(env::split_paths(&path_env).map(|p| p.join(&name)));
    for candidate in candidates {
        if let Ok(path) = candidate.canonicalize()
            && let Ok(output) = Command::new(&path).arg("--version").output()
            && output.status.success()
            && output.stdout.starts_with(b"ripgrep ")
        {
            return Ok(path);
        }
    }
    bail!(
        "ripgrep is required but was not found. Install ripgrep, then rerun setup. Packaged releases may provide rg beside oko."
    )
}
fn text(path: &Path) -> Result<Option<String>> {
    if let Ok(meta) = fs::symlink_metadata(path)
        && (meta.file_type().is_symlink() || !meta.is_file())
    {
        bail!(
            "Refusing to replace a symlink or non-file: {}",
            path.display()
        );
    }
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!(
            "Cannot read {} as UTF-8; nothing was overwritten",
            path.display()
        ),
    }
}
fn config(original: &str, exe: &Path, root: &Path, rg: &Path, offline: bool) -> Result<String> {
    let mut doc: DocumentMut = original.parse().map_err(|_| {
        anyhow::anyhow!("Existing Codex configuration is invalid TOML; nothing was overwritten")
    })?;
    if let Some(existing) = doc.get("mcp_servers").and_then(|v| v.get("oko")) {
        let owned = existing
            .as_table()
            .and_then(|t| t.decor().prefix())
            .and_then(|s| s.as_str())
            .is_some_and(|s| s.contains(MANAGED));
        if !owned {
            bail!(
                "An Oko MCP entry already exists and is not managed by setup. Rename or remove that entry before continuing."
            );
        }
    }
    if doc.get("mcp_servers").is_some_and(|v| !v.is_table()) {
        bail!("mcp_servers must be a TOML table; nothing was overwritten");
    }
    if doc.get("mcp_servers").is_none() {
        doc["mcp_servers"] = Item::Table(Table::new());
    }
    let mut server = doc["mcp_servers"]
        .get("oko")
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    server.decor_mut().set_prefix(format!("\n{MANAGED}\n"));
    server["command"] = value(exe.to_str().context("Executable path must be UTF-8")?);
    let mut args = toml_edit::Array::new();
    args.push("mcp");
    args.push("--root");
    args.push(root.to_str().context("Project path must be UTF-8")?);
    if offline {
        args.push("--no-jev");
    }
    server["args"] = value(args);
    server["cwd"] = value(root.to_str().unwrap());
    server["startup_timeout_sec"] = value(15);
    server["tool_timeout_sec"] = value(120);
    server["enabled"] = value(true);
    if server.get("env").is_some_and(|item| !item.is_table()) {
        bail!("Existing Oko env must be a TOML table; nothing was overwritten");
    }
    let mut environment = server
        .get("env")
        .and_then(Item::as_table)
        .cloned()
        .unwrap_or_default();
    environment["OKO_RIPGREP"] = value(rg.to_str().context("ripgrep path must be UTF-8")?);
    server["env"] = Item::Table(environment);
    doc["mcp_servers"]["oko"] = Item::Table(server);
    Ok(doc.to_string())
}
fn instructions(original: &str) -> Result<String> {
    let start = original
        .match_indices(START)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let end = original
        .match_indices(END)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    match (start.as_slice(), end.as_slice()) {
        ([], []) => Ok(format!(
            "{original}{}{GUIDANCE}\n",
            if original.is_empty() {
                ""
            } else if original.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            }
        )),
        ([a], [b]) if a < b => Ok(format!(
            "{}{GUIDANCE}{}",
            &original[..*a],
            &original[*b + END.len()..]
        )),
        _ => bail!("Oko instruction markers are incomplete or duplicated; nothing was overwritten"),
    }
}
fn atomic(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let parent = path.parent().context("File has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable {
            0o755
        } else {
            fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0o600)
        };
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|_| anyhow::anyhow!("Cannot replace {}", path.display()))?;
    Ok(())
}
struct Edit {
    path: PathBuf,
    before: Option<String>,
    after: String,
}
fn apply(edits: &[Edit], backups: &Path) -> Result<()> {
    // Detect intervening edits before writing; retain private backups outside the project.
    for edit in edits {
        if text(&edit.path)? != edit.before {
            bail!("A setup file changed during setup; rerun to preserve those edits");
        }
    }
    for edit in edits
        .iter()
        .filter(|e| e.before.as_deref() != Some(&e.after))
    {
        if let Some(before) = &edit.before {
            fs::create_dir_all(backups)?;
            let mut backup = tempfile::Builder::new()
                .prefix("oko-setup-")
                .tempfile_in(backups)?;
            backup.write_all(before.as_bytes())?;
            backup.as_file().sync_all()?;
            let (_, path) = backup.keep()?;
            println!("Backup of {}: {}", edit.path.display(), path.display());
        }
    }
    for (i, edit) in edits.iter().enumerate() {
        if edit.before.as_deref() == Some(&edit.after) {
            continue;
        }
        if let Err(error) = atomic(&edit.path, edit.after.as_bytes(), false) {
            for prior in edits[..i].iter().rev() {
                let restored = if let Some(before) = &prior.before {
                    atomic(&prior.path, before.as_bytes(), false)
                } else {
                    fs::remove_file(&prior.path).map_err(Into::into)
                };
                if restored.is_err() {
                    eprintln!(
                        "Could not restore {}; use the setup backup.",
                        prior.path.display()
                    );
                }
            }
            return Err(error.context(
                "Setup could not save all files; previous changes were restored where possible",
            ));
        }
    }
    Ok(())
}
fn healthcheck(exe: &Path, root: &Path, rg: &Path, offline: bool) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let mut command = tokio::process::Command::new(exe);
            command
                .args(["mcp", "--root"])
                .arg(root)
                .current_dir(root)
                .env("OKO_RIPGREP", rg)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            if offline {
                command.arg("--no-jev");
            }
            let mut child = command.spawn().context("Cannot launch installed Oko")?;
            let transport = (child.stdout.take().unwrap(), child.stdin.take().unwrap());
            let result = tokio::time::timeout(Duration::from_secs(10), async {
                let service = ().serve(transport).await.context("MCP initialization failed")?;
                let tools = service
                    .list_tools(None)
                    .await
                    .context("MCP tool discovery failed")?;
                if !tools.tools.iter().any(|t| t.name == "search") {
                    bail!("Installed Oko did not advertise search");
                }
                service.cancel().await?;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("MCP connection check timed out")?;
            let _ = child.kill().await;
            result
        })
}
pub fn run(args: &[String], cwd: &Path) -> Result<()> {
    if matches!(args, [flag] if flag == "--help" || flag == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let options = options(args, cwd)?;
    let source = env::current_exe()?.canonicalize()?;
    let rg_source = ripgrep(&source)?;
    // Refuse project configuration redirected through a symlink.
    let config_dir = options.root.join(".codex");
    if let Ok(meta) = fs::symlink_metadata(&config_dir)
        && (meta.file_type().is_symlink() || !meta.is_dir())
    {
        bail!("Project .codex must be a real directory");
    }
    fs::create_dir_all(&options.install)?;
    let install = options.install.canonicalize()?;
    let executable = install.join(executable_name("oko"));
    let rg = install.join(executable_name("rg"));
    let config_path = config_dir.join("config.toml");
    let original = text(&config_path)?;
    let updated = config(
        original.as_deref().unwrap_or(""),
        &executable,
        &options.root,
        &rg,
        options.offline,
    )?;
    let mut edits = vec![Edit {
        path: config_path,
        before: original,
        after: updated,
    }];
    if options.instructions {
        let override_path = options.root.join("AGENTS.override.md");
        let path = if override_path.exists() {
            override_path
        } else {
            options.root.join("AGENTS.md")
        };
        let original = text(&path)?;
        let updated = instructions(original.as_deref().unwrap_or(""))?;
        edits.push(Edit {
            path,
            before: original,
            after: updated,
        });
    }
    let ignore_path = options.root.join(".gitignore");
    let original = text(&ignore_path)?;
    let mut updated = original.clone().unwrap_or_default();
    if !updated.lines().any(|line| line == "/.codex/config.toml") {
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str("\n# Oko: machine-local Codex connection\n/.codex/config.toml\n");
    }
    if !updated.lines().any(|line| line == "/.env") {
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str("/.env\n");
    }
    edits.push(Edit {
        path: ignore_path,
        before: original,
        after: updated,
    });
    println!("Setting up Codex for {}", options.root.display());
    if !options.offline {
        crate::auth::setup(&options.root)?;
    }
    // This copy can survive deletion of the download or development build.
    if source != executable {
        atomic(&executable, &fs::read(&source)?, true)?;
    }
    if rg_source != rg {
        atomic(&rg, &fs::read(&rg_source)?, true)?;
    }
    healthcheck(&executable, &options.root, &rg, options.offline)?;
    apply(&edits, &install.join("setup-backups"))?;
    println!(
        "Oko installed: {}\nMCP startup and search-tool discovery verified.\nCodex project configuration saved. Open this project in Codex, trust it if prompted, and start a new session.\nUse /mcp to check the connection. No Jev request was made; key validity and Codex tool selection are not yet tested.",
        executable.display()
    );
    println!(
        "The machine-specific .codex/config.toml is added to .gitignore (already-tracked files remain tracked). Run setup for each project you want to connect."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn updating_owned_config_keeps_user_options_and_other_environment() {
        let first = config(
            "",
            Path::new("/bin/oko"),
            Path::new("/project"),
            Path::new("/bin/rg"),
            false,
        )
        .unwrap();
        let mut doc: DocumentMut = first.parse().unwrap();
        doc["mcp_servers"]["oko"]["required"] = value(true);
        doc["mcp_servers"]["oko"]["env"]["TYPESAFE_DEFAULT_MODEL"] = value("custom");
        let updated = config(
            &doc.to_string(),
            Path::new("/new/oko"),
            Path::new("/project"),
            Path::new("/new/rg"),
            true,
        )
        .unwrap();
        let doc: DocumentMut = updated.parse().unwrap();
        assert_eq!(doc["mcp_servers"]["oko"]["required"].as_bool(), Some(true));
        assert_eq!(
            doc["mcp_servers"]["oko"]["env"]["TYPESAFE_DEFAULT_MODEL"].as_str(),
            Some("custom")
        );
        assert_eq!(
            doc["mcp_servers"]["oko"]["command"].as_str(),
            Some("/new/oko")
        );
    }
    #[test]
    fn intervening_edits_are_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(&path, "user changed this").unwrap();
        let edit = Edit {
            path: path.clone(),
            before: Some("old".into()),
            after: "new".into(),
        };
        assert!(apply(&[edit], &temp.path().join("backups")).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "user changed this");
    }
}
