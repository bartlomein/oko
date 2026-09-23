//! Project-scoped setup for Codex, Claude Code and OpenCode. Never writes
//! credentials to configuration files.
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

const USAGE: &str = "Usage: oko setup [--client codex|claude|opencode|all] [--root DIRECTORY]\n                 [--no-jev] [--no-instructions] [--install-dir DIRECTORY]\n\nSet up Oko in the chosen project (default: current directory) for one or more\ncoding tools (default: codex; separate several with commas, or use all).\nInstalls a stable copy, checks MCP, then connects each tool:\n  codex     updates .codex/config.toml\n  claude    runs `claude mcp add-json --scope local` (needs the claude command)\n  opencode  updates opencode.json\nAdds a managed search section to the instructions each tool reads (AGENTS.md,\nand CLAUDE.md for Claude Code) unless --no-instructions is set.\n--no-jev sets up local-only search without credentials or network calls.\n--install-dir overrides the per-user application bin directory.";
const MANAGED: &str = "# Managed by oko setup";
const START: &str = "<!-- oko:search:start -->";
const END: &str = "<!-- oko:search:end -->";
// One file, so the benchmark measures the text users receive. It follows the
// pattern OpenAI documents for its coding models: when to use the tool, what
// its output is, a stop rule, and good and bad examples. A sentence in an MCP
// tool description did not change their behaviour; project instructions are
// where these agents look.
const GUIDANCE: &str = concat!(
    "<!-- oko:search:start -->\n",
    include_str!("guidance.md"),
    "<!-- oko:search:end -->"
);

#[derive(Clone, Copy, PartialEq)]
enum Client {
    Codex,
    Claude,
    OpenCode,
}
impl Client {
    const ALL: [Client; 3] = [Client::Codex, Client::Claude, Client::OpenCode];
    fn name(self) -> &'static str {
        match self {
            Client::Codex => "Codex",
            Client::Claude => "Claude Code",
            Client::OpenCode => "OpenCode",
        }
    }
}
fn clients(list: &str) -> Result<Vec<Client>> {
    let mut chosen = Vec::new();
    for name in list.split(',') {
        let named = match name {
            "codex" => &Client::ALL[..1],
            "claude" => &Client::ALL[1..2],
            "opencode" => &Client::ALL[2..],
            "all" => &Client::ALL[..],
            _ => bail!("Unknown --client {name:?}.\n{USAGE}"),
        };
        for client in named {
            if !chosen.contains(client) {
                chosen.push(*client);
            }
        }
    }
    Ok(chosen)
}
struct Options {
    clients: Vec<Client>,
    root: PathBuf,
    install: PathBuf,
    offline: bool,
    instructions: bool,
}
fn options(args: &[String], cwd: &Path) -> Result<Options> {
    let (mut root, mut install, mut offline, mut instructions) = (None, None, false, true);
    let mut chosen = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" if root.is_none() => {
                root = Some(args.next().context("--root needs a directory")?)
            }
            "--install-dir" if install.is_none() => {
                install = Some(args.next().context("--install-dir needs a directory")?)
            }
            "--client" if chosen.is_none() => {
                chosen = Some(clients(args.next().context("--client needs a name")?)?)
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
        clients: chosen.unwrap_or_else(|| vec![Client::Codex]),
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
fn server_args(root: &Path, offline: bool) -> Result<Vec<&str>> {
    let mut args = vec![
        "mcp",
        "--root",
        root.to_str().context("Project path must be UTF-8")?,
    ];
    if offline {
        args.push("--no-jev");
    }
    Ok(args)
}
// JSON has no comments to carry an ownership marker, so an entry counts as
// setup's own when it launches an `oko` executable as an MCP server.
fn launches_oko(command: &str, first_arg: Option<&str>) -> bool {
    Path::new(command).file_stem().and_then(|s| s.to_str()) == Some("oko")
        && first_arg == Some("mcp")
}
fn opencode(original: &str, exe: &Path, root: &Path, rg: &Path, offline: bool) -> Result<String> {
    use serde_json::{Map, Value, json};
    let before: Value = if original.trim().is_empty() {
        json!({"$schema": "https://opencode.ai/config.json"})
    } else {
        serde_json::from_str(original).map_err(|_| {
            anyhow::anyhow!(
                "opencode.json is not plain JSON (comments are not supported by setup); nothing was overwritten. Add the entry from docs/clients.md by hand."
            )
        })?
    };
    let mut doc = before.clone();
    let top = doc
        .as_object_mut()
        .context("opencode.json must hold a JSON object; nothing was overwritten")?;
    let mcp = top
        .entry("mcp")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("opencode.json mcp must be an object; nothing was overwritten")?;
    if mcp.get("servers").is_some_and(Value::is_object) {
        bail!(
            "opencode.json uses the v2 mcp.servers layout, which setup does not edit; nothing was overwritten. Add the entry from docs/clients.md by hand."
        );
    }
    if let Some(existing) = mcp.get("oko") {
        let command = existing.get("command").and_then(Value::as_array);
        let part = |i: usize| command.and_then(|c| c.get(i)).and_then(Value::as_str);
        if !part(0).is_some_and(|program| launches_oko(program, part(1))) {
            bail!(
                "An Oko MCP entry already exists in opencode.json and was not created by setup. Rename or remove that entry before continuing."
            );
        }
    }
    let server = mcp
        .entry("oko")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("Existing Oko entry must be an object; nothing was overwritten")?;
    let mut command = vec![exe.to_str().context("Executable path must be UTF-8")?];
    command.extend(server_args(root, offline)?);
    server.insert("type".into(), json!("local"));
    server.insert("command".into(), json!(command));
    server.insert("enabled".into(), json!(true));
    server
        .entry("environment")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("Existing Oko environment must be an object; nothing was overwritten")?
        .insert(
            "OKO_RIPGREP".into(),
            json!(rg.to_str().context("ripgrep path must be UTF-8")?),
        );
    // Leave the user's formatting alone when nothing changes.
    if doc == before && !original.trim().is_empty() {
        return Ok(original.into());
    }
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}
/// Claude Code keeps per-project, per-user connections in its own settings
/// file; its command line is the supported way to change them.
struct Claude {
    program: PathBuf,
    root: PathBuf,
}
impl Claude {
    fn find(root: &Path) -> Result<Self> {
        let program = env::var_os("OKO_CLAUDE")
            .map(PathBuf::from)
            .unwrap_or_else(|| executable_name("claude").into());
        let claude = Self {
            program,
            root: root.into(),
        };
        if claude.run(&["--version"]).is_err() {
            bail!(
                "The claude command was not found. Install Claude Code or set OKO_CLAUDE to its path, then rerun setup."
            );
        }
        Ok(claude)
    }
    fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new(&self.program)
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .context("Cannot run the claude command")?;
        if !output.status.success() {
            bail!(
                "{}",
                String::from_utf8_lossy(&output.stderr).trim().to_owned()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    /// Whether a connection named oko exists; an error if it is someone else's.
    fn existing(&self) -> Result<bool> {
        let Ok(described) = self.run(&["mcp", "get", "oko"]) else {
            return Ok(false);
        };
        let field = |name: &str| {
            described
                .lines()
                .find_map(|line| line.trim().strip_prefix(name))
                .map(str::trim)
        };
        let first_arg = field("Args:").and_then(|args| args.split(' ').next());
        if !field("Command:").is_some_and(|command| launches_oko(command, first_arg)) {
            bail!(
                "Claude Code already has an MCP connection named oko that setup did not create. Remove it with `claude mcp remove oko`, then rerun setup."
            );
        }
        Ok(true)
    }
    fn connect(&self, replace: bool, exe: &Path, rg: &Path, offline: bool) -> Result<()> {
        if replace {
            self.run(&["mcp", "remove", "--scope", "local", "oko"])
                .context("Cannot replace the existing Claude Code connection; remove it with `claude mcp remove oko`, then rerun setup")?;
        }
        // Claude Code defers MCP tools behind a tool search: an agent sees only
        // the bare name until it loads the schema, and explorers never do.
        // `alwaysLoad` needs add-json; `mcp add` has no flag for it.
        let server = serde_json::json!({
            "type": "stdio",
            "command": exe.to_str().context("Executable path must be UTF-8")?,
            "args": server_args(&self.root, offline)?,
            "env": {"OKO_RIPGREP": rg.to_str().context("ripgrep path must be UTF-8")?},
            "alwaysLoad": true,
        })
        .to_string();
        let args = [
            "mcp",
            "add-json",
            "--scope",
            "local",
            "oko",
            server.as_str(),
        ];
        self.run(&args)
            .context("Claude Code did not accept the connection")?;
        Ok(())
    }
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
    let wants = |client| options.clients.contains(&client);
    // Refuse project configuration redirected through a symlink.
    let config_dir = options.root.join(".codex");
    if wants(Client::Codex)
        && let Ok(meta) = fs::symlink_metadata(&config_dir)
        && (meta.file_type().is_symlink() || !meta.is_dir())
    {
        bail!("Project .codex must be a real directory");
    }
    fs::create_dir_all(&options.install)?;
    let install = options.install.canonicalize()?;
    let executable = install.join(executable_name("oko"));
    let rg = install.join(executable_name("rg"));
    let mut edits = Vec::new();
    let mut ignored = Vec::new();
    if wants(Client::Codex) {
        let path = config_dir.join("config.toml");
        let before = text(&path)?;
        let after = config(
            before.as_deref().unwrap_or(""),
            &executable,
            &options.root,
            &rg,
            options.offline,
        )?;
        edits.push(Edit {
            path,
            before,
            after,
        });
        ignored.push((
            "/.codex/config.toml",
            "# Oko: machine-local Codex connection",
        ));
    }
    let mut shared_opencode = false;
    if wants(Client::OpenCode) {
        if options.root.join("opencode.jsonc").exists() {
            bail!(
                "This project uses opencode.jsonc, which setup does not edit. Add the entry from docs/clients.md by hand, or run setup for the other tools with --client."
            );
        }
        let path = options.root.join("opencode.json");
        let before = text(&path)?;
        let after = opencode(
            before.as_deref().unwrap_or(""),
            &executable,
            &options.root,
            &rg,
            options.offline,
        )?;
        // A file that was already here may be shared; only a new one is ignored.
        if before.is_none() {
            ignored.push(("/opencode.json", "# Oko: machine-local OpenCode connection"));
        } else {
            shared_opencode = before.as_deref() != Some(&after);
        }
        edits.push(Edit {
            path,
            before,
            after,
        });
    }
    let claude = if wants(Client::Claude) {
        let claude = Claude::find(&options.root)?;
        let replace = claude.existing()?;
        Some((claude, replace))
    } else {
        None
    };
    if options.instructions {
        // Codex and OpenCode read AGENTS.md; Claude Code reads CLAUDE.md, which
        // may itself import AGENTS.md.
        let agents = options.root.join("AGENTS.md");
        let mut paths = Vec::new();
        for client in &options.clients {
            let path = match client {
                Client::Codex if options.root.join("AGENTS.override.md").exists() => {
                    options.root.join("AGENTS.override.md")
                }
                Client::Claude => {
                    let own = options.root.join("CLAUDE.md");
                    if text(&own)?.is_some_and(|text| text.contains("@AGENTS.md")) {
                        agents.clone()
                    } else {
                        own
                    }
                }
                _ => agents.clone(),
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        for path in paths {
            let before = text(&path)?;
            let after = instructions(before.as_deref().unwrap_or(""))?;
            edits.push(Edit {
                path,
                before,
                after,
            });
        }
    }
    let ignore_path = options.root.join(".gitignore");
    let original = text(&ignore_path)?;
    let mut updated = original.clone().unwrap_or_default();
    ignored.push(("/.env", ""));
    for (entry, comment) in ignored {
        if updated.lines().any(|line| line == entry) {
            continue;
        }
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        if !comment.is_empty() {
            updated.push_str(&format!("\n{comment}\n"));
        }
        updated.push_str(&format!("{entry}\n"));
    }
    edits.push(Edit {
        path: ignore_path,
        before: original,
        after: updated,
    });
    let names = options
        .clients
        .iter()
        .map(|client| client.name())
        .collect::<Vec<_>>()
        .join(", ");
    println!("Setting up {names} for {}", options.root.display());
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
    // Before the files: a refusal here leaves the project untouched.
    if let Some((claude, replace)) = &claude {
        claude.connect(*replace, &executable, &rg, options.offline)?;
    }
    apply(&edits, &install.join("setup-backups"))?;
    println!(
        "Oko installed: {}\nMCP startup and search-tool discovery verified.",
        executable.display()
    );
    for client in &options.clients {
        println!(
            "{}",
            match client {
                Client::Codex =>
                    "Codex project configuration saved. Open this project in Codex, trust it if prompted, and start a new session. Use /mcp to check the connection.",
                Client::Claude =>
                    "Claude Code connection added for this project and user. Start a new session and use /mcp to check it.",
                Client::OpenCode =>
                    "OpenCode project configuration saved. Start a new session and run `opencode mcp list` to check the connection.",
            }
        );
    }
    println!("No Jev request was made; key validity and tool selection are not yet tested.");
    if shared_opencode {
        println!(
            "opencode.json already existed, so it was not added to .gitignore; it now holds paths for this machine. Keep that change out of shared commits."
        );
    }
    println!(
        "Machine-specific configuration that setup created is added to .gitignore (already-tracked files remain tracked). Run setup for each project you want to connect."
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
