//! `oko upgrade`, and the once-a-day check behind the "Oko X is out" notice.
//!
//! The check asks GitHub for the latest release's version number, nothing
//! else, at most once a day; it is skipped in CI and with
//! `OKO_NO_UPDATE_CHECK`, and a failed check stays silent. `oko upgrade`
//! downloads the release, checks it like the installer does, replaces the
//! installer's `oko` command and setup's copy, then reruns the new setup on
//! every project that copy serves.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LATEST_URL: &str = "https://api.github.com/repos/bartlomein/oko/releases/latest";
const DOWNLOAD_URL: &str = "https://github.com/bartlomein/oko/releases/download";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const USAGE: &str = "Usage: oko upgrade [--install-dir DIRECTORY]\n\nInstalls the latest Oko release: downloads it, verifies its checksum, replaces\nthe `oko` command the installer put on your PATH and the copy `oko setup`\ninstalled (default: the per-user application bin directory, or --install-dir),\nthen refreshes every project set up with that copy. Start new agent sessions\nafterwards; open ones keep the old version until restarted.";

type Version = (u64, u64, u64);

/// `0.7.1` or `v0.7.1`; anything else is not a release version.
fn version(text: &str) -> Option<Version> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut part = || -> Option<u64> {
        let digits = parts.next()?;
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse().ok())?
    };
    let parsed = (part()?, part()?, part()?);
    parts.next().is_none().then_some(parsed)
}

fn current() -> Version {
    version(env!("CARGO_PKG_VERSION")).expect("the package version is x.y.z")
}

/// The latest-release and download addresses; tests serve releases locally.
fn urls() -> (String, String) {
    match env::var("OKO_RELEASES_URL") {
        Ok(base) => (format!("{base}/latest"), format!("{base}/download")),
        Err(_) => (LATEST_URL.to_owned(), DOWNLOAD_URL.to_owned()),
    }
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("oko/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// The latest release's tag, such as `v0.7.1`.
fn latest_release(timeout: Duration) -> Result<String> {
    let (latest, _) = urls();
    let release: Value = client(timeout)?
        .get(latest)
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()?;
    let tag = release["tag_name"]
        .as_str()
        .context("The latest release has no tag")?;
    version(tag).with_context(|| format!("The latest release tag {tag:?} is not a version"))?;
    Ok(tag.to_owned())
}

fn checks_allowed() -> bool {
    env::var_os("OKO_NO_UPDATE_CHECK").is_none() && env::var_os("CI").is_none()
}

fn state_file() -> Option<PathBuf> {
    env::var_os("OKO_UPDATE_STATE")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|path| path.join("oko").join("update.json")))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn read_state(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_state(path: &Path, latest: Option<&str>) {
    let state = json!({"checkedAt": now(), "latest": latest});
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_ok()
        && let Ok(mut file) = tempfile::NamedTempFile::new_in(parent)
        && std::io::Write::write_all(&mut file, state.to_string().as_bytes()).is_ok()
    {
        let _ = file.persist(path);
    }
}

/// Asks GitHub for the latest version when the last check is a day old.
/// Silent: a failed check is recorded as a check, and tried again tomorrow.
pub fn refresh(timeout: Duration) {
    if !checks_allowed() {
        return;
    }
    let Some(path) = state_file() else {
        return;
    };
    let state = read_state(&path);
    let checked = state["checkedAt"].as_u64().unwrap_or(0);
    if now().saturating_sub(checked) < CHECK_EVERY.as_secs() {
        return;
    }
    let latest = latest_release(timeout).ok();
    let known = state["latest"].as_str().map(str::to_owned);
    write_state(&path, latest.as_deref().or(known.as_deref()));
}

/// "Oko X is out" when the last check found a newer release than this one.
pub fn notice() -> Option<String> {
    if !checks_allowed() {
        return None;
    }
    notice_for(&read_state(&state_file()?), current())
}

fn notice_for(state: &Value, current: Version) -> Option<String> {
    let latest = version(state["latest"].as_str()?)?;
    (latest > current).then(|| {
        let show = |(a, b, c): Version| format!("{a}.{b}.{c}");
        format!(
            "Oko {} is out (you have {}). Run `oko upgrade` to update.",
            show(latest),
            show(current)
        )
    })
}

/// The release archive's target for this machine.
fn target() -> Option<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        _ => None,
    }
}

/// The `oko` command the install script manages: a link into its releases.
struct InstallerLink {
    link: PathBuf,
    releases: PathBuf,
}

fn installer_link() -> Option<InstallerLink> {
    let home = PathBuf::from(env::var_os("HOME")?);
    let install = env::var_os("OKO_INSTALL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share/oko"));
    let bin = env::var_os("OKO_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/bin"));
    let link = bin.join("oko");
    let target = fs::read_link(&link).ok()?;
    let releases = install.canonicalize().ok()?.join("releases");
    target
        .starts_with(&releases)
        .then_some(InstallerLink { link, releases })
}

fn run_tar(args: &[&str], archive: &Path, extra: &[&Path]) -> Result<String> {
    let output = Command::new("tar")
        .args(args)
        .arg(archive)
        .args(extra.iter().flat_map(|path| [Path::new("-C"), path]))
        .output()
        .context("Cannot run tar")?;
    if !output.status.success() {
        bail!("Cannot read the release archive");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Unpacks a verified archive into `into`, refusing what the install script
/// refuses: paths outside the one release folder, links and special files.
fn unpack(archive: &Path, bundle: &str, into: &Path) -> Result<PathBuf> {
    for member in run_tar(&["-tzf"], archive, &[])?.lines() {
        let member = member.trim_end_matches('/');
        let inside = member == bundle || member.starts_with(&format!("{bundle}/"));
        let wrapped = format!("/{member}/");
        if !inside
            || ["/../", "/./", "//"]
                .iter()
                .any(|bad| wrapped.contains(bad))
        {
            bail!("Unsafe path in the release archive");
        }
    }
    if run_tar(&["-tvzf"], archive, &[])?
        .lines()
        .any(|line| !line.starts_with('-') && !line.starts_with('d'))
    {
        bail!("The release archive contains a link or special file");
    }
    run_tar(&["-xzf"], archive, &[into])?;
    Ok(into.join(bundle))
}

fn reports(program: &Path, expected: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == expected
        })
}

fn runs_ripgrep(program: &Path) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success() && output.stdout.starts_with(b"ripgrep "))
}

/// Downloads the release for this machine into `into` and checks it: the
/// archive against `SHA256SUMS`, then that its `oko` reports the version.
fn fetch(tag: &str, target: &str, into: &Path) -> Result<PathBuf> {
    let (_, downloads) = urls();
    let client = client(Duration::from_secs(300))?;
    let get = |file: &str| -> Result<Vec<u8>> {
        Ok(client
            .get(format!("{downloads}/{tag}/{file}"))
            .send()?
            .error_for_status()
            .with_context(|| format!("Cannot download {file}"))?
            .bytes()?
            .to_vec())
    };
    let bundle = format!("oko-{tag}-{target}");
    let archive_name = format!("{bundle}.tar.gz");
    println!("Downloading {archive_name}…");
    let sums = String::from_utf8(get("SHA256SUMS")?).context("SHA256SUMS is not text")?;
    let expected: Vec<&str> = sums
        .lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once(char::is_whitespace)?;
            (name.trim().trim_start_matches('*') == archive_name).then_some(hash)
        })
        .collect();
    let [expected] = expected[..] else {
        bail!("SHA256SUMS lists {archive_name} {} times", expected.len());
    };
    let archive = get(&archive_name)?;
    let actual: String = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("Checksum mismatch for {archive_name}; nothing was changed");
    }
    let saved = into.join(&archive_name);
    fs::write(&saved, archive)?;
    let unpacked = unpack(&saved, &bundle, into)?;
    fs::remove_file(&saved)?;
    let expected_version = format!("oko {}", tag.trim_start_matches('v'));
    if !reports(&unpacked.join("oko"), &expected_version) {
        bail!("The downloaded Oko does not run on this system or is not {tag}");
    }
    if !runs_ripgrep(&unpacked.join("rg")) {
        bail!("The downloaded ripgrep does not run on this system");
    }
    Ok(unpacked)
}

#[cfg(unix)]
fn relink(link: &Path, to: &Path) -> Result<()> {
    let folder = link.parent().context("The oko link has no folder")?;
    let staged = tempfile::Builder::new()
        .prefix(".oko-link.")
        .tempdir_in(folder)?;
    let temporary = staged.path().join("oko");
    std::os::unix::fs::symlink(to, &temporary)?;
    fs::rename(&temporary, link)?;
    Ok(())
}

#[cfg(not(unix))]
fn relink(_: &Path, _: &Path) -> Result<()> {
    bail!("oko upgrade supports macOS and Linux")
}

fn home_relative(path: &Path) -> String {
    match env::var_os("HOME").map(PathBuf::from) {
        Some(home) if path.starts_with(&home) => {
            format!("~/{}", path.strip_prefix(&home).unwrap_or(path).display())
        }
        _ => path.display().to_string(),
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let install = match args {
        [] => None,
        [flag, directory] if flag == "--install-dir" => Some(PathBuf::from(directory)),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return Ok(());
        }
        _ => bail!("Invalid upgrade arguments.\n{USAGE}"),
    };
    let install = match install {
        Some(path) => path,
        None => crate::setup::default_install_dir()?,
    };
    let current = current();
    let tag = latest_release(Duration::from_secs(15))
        .context("Cannot reach GitHub to find the latest release")?;
    let latest = version(&tag).expect("checked by latest_release");
    if let Some(path) = state_file() {
        write_state(&path, Some(&tag));
    }
    let show = |(a, b, c): Version| format!("{a}.{b}.{c}");
    if latest <= current {
        println!("Oko {} is the latest release.", show(current));
        return Ok(());
    }
    let target =
        target().context("Oko releases are built for macOS and Linux (x64 and ARM64) only")?;
    let link = installer_link();
    let setup_copy = install.join("oko");
    let has_setup_copy = setup_copy.is_file();
    if link.is_none() && !has_setup_copy {
        bail!(
            "Found no Oko to upgrade: neither the install script's `oko` command nor a copy from `oko setup` in {}. Install with the install script, or update Oko the way you installed it.",
            home_relative(&install)
        );
    }
    println!("Upgrading Oko {} to {}.", show(current), show(latest));
    // In the installer's releases folder the download is the new install;
    // elsewhere it is staged and removed afterwards.
    let staging = match &link {
        Some(link) => tempfile::Builder::new()
            .prefix(&format!("{tag}."))
            .tempdir_in(&link.releases)?,
        None => tempfile::tempdir()?,
    };
    let bundle = fetch(&tag, target, staging.path())?;
    println!("Checksum verified.");
    let new_oko = bundle.join("oko");
    let new_rg = bundle.join("rg");
    if let Some(link) = &link {
        relink(&link.link, &new_oko)?;
        let _ = staging.keep();
        println!("Updated the `oko` command: {}", home_relative(&link.link));
    } else {
        println!(
            "Your `oko` command was not installed by the install script; update it the way you installed it."
        );
    }
    if !has_setup_copy {
        return finish(latest);
    }
    crate::setup::atomic(&setup_copy, &fs::read(&new_oko)?, true)?;
    crate::setup::atomic(&install.join("rg"), &fs::read(&new_rg)?, true)?;
    println!(
        "Updated the copy every project uses: {}",
        home_relative(&setup_copy)
    );
    refresh_projects(&install, &setup_copy, latest)?;
    finish(latest)
}

/// Reruns the new version's setup on every project the copy serves, so new
/// hooks and guidance reach them; a project that is gone is forgotten.
fn refresh_projects(install: &Path, setup_copy: &Path, latest: Version) -> Result<()> {
    let listed = install.join(crate::setup::PROJECTS_FILE).is_file();
    let projects = crate::setup::projects(install);
    let (present, gone): (Vec<_>, Vec<_>) = projects
        .into_iter()
        .partition(|project| project.root.is_dir());
    if !gone.is_empty() {
        crate::setup::forget_projects(install, |project| !project.root.is_dir())?;
        for project in &gone {
            println!(
                "Forgot {}: the folder is gone.",
                home_relative(&project.root)
            );
        }
    }
    if present.is_empty() {
        if !listed {
            println!(
                "Projects set up before this version already use the new copy. Rerun `oko setup` in each of them once, so `oko upgrade` can refresh their hooks and guidance from now on."
            );
        }
        return Ok(());
    }
    if cfg!(target_os = "macos") && present.iter().any(|project| !project.no_jev) {
        println!(
            "macOS may ask once to let the new Oko read your TypeSafe key: choose Always Allow."
        );
    }
    println!("Refreshing {} project(s):", present.len());
    let mut failed = 0;
    for project in &present {
        let args: Vec<OsString> = project.setup_args(install);
        let ok = Command::new(setup_copy)
            .arg("setup")
            .args(&args)
            .arg("--quiet")
            .status()
            .is_ok_and(|status| status.success());
        failed += usize::from(!ok);
        println!(
            "  {} ({}): {}",
            home_relative(&project.root),
            project.client_names(),
            if ok { "done" } else { "failed, see above" }
        );
    }
    if failed > 0 {
        let (a, b, c) = latest;
        println!(
            "Rerun `oko setup` in the {failed} project(s) that failed; they already use Oko {a}.{b}.{c}, but may lack its newest hooks or guidance."
        );
    }
    Ok(())
}

fn finish((a, b, c): Version) -> Result<()> {
    println!(
        "Oko {a}.{b}.{c} is installed. Start new agent sessions to use it; open ones keep the old version until restarted."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_three_numbers() {
        assert_eq!(version("v0.7.1"), Some((0, 7, 1)));
        assert_eq!(version("0.10.0"), Some((0, 10, 0)));
        for bad in [
            "v0.7",
            "v0.7.1.2",
            "v0.7.1-rc1",
            "latest",
            "v0..1",
            "v0.7.x",
            "../v0.7.1",
        ] {
            assert_eq!(version(bad), None, "{bad}");
        }
        assert!(version("0.10.0") > version("0.9.9"));
    }

    #[test]
    fn notice_only_for_a_newer_release() {
        let state = |latest: &str| json!({"checkedAt": 1, "latest": latest});
        assert_eq!(
            notice_for(&state("v0.7.1"), (0, 7, 0)).as_deref(),
            Some("Oko 0.7.1 is out (you have 0.7.0). Run `oko upgrade` to update.")
        );
        assert_eq!(notice_for(&state("v0.7.0"), (0, 7, 0)), None);
        assert_eq!(notice_for(&state("v0.6.9"), (0, 7, 0)), None);
        assert_eq!(notice_for(&json!({}), (0, 7, 0)), None);
        assert_eq!(notice_for(&json!({"latest": null}), (0, 7, 0)), None);
    }

    #[test]
    fn every_release_target_is_known() {
        // The four archives each release publishes.
        if cfg!(any(target_os = "macos", target_os = "linux"))
            && cfg!(any(target_arch = "aarch64", target_arch = "x86_64"))
        {
            assert!(target().is_some());
        }
    }
}
