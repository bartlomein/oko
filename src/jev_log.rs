//! A local log of Jev requests, so `oko stats` can say what searching has
//! cost. Each line holds the time, workspace, phase, outcome and token counts
//! of one request: never the question, code or key. One file per month in the
//! user cache directory; `OKO_JEV_LOG_DIR` moves it, `OKO_NO_JEV_LOG=1` stops it.
use crate::ranking::JevCallStats;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// TypeSafe's list price for Jev: input tokens only, output is free.
pub const USD_PER_MILLION_INPUT: f64 = 0.042;

const USAGE: &str = "Usage: oko stats [--days N] [--json]\n\nSummarises the Jev requests this machine made in the last N days (default 30):\ncalls, failures, input tokens and their cost at TypeSafe's list price, by day,\nphase and workspace. The log holds no questions, code or keys.\nOKO_JEV_LOG_DIR moves it; OKO_NO_JEV_LOG=1 turns logging off.";

static WORKSPACE: OnceLock<String> = OnceLock::new();

/// The workspace this process searches, for the log's per-workspace totals.
/// The first call wins: a process serves one root.
pub fn set_workspace(root: &Path) {
    let _ = WORKSPACE.set(root.display().to_string());
}

/// One logged request.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// UTC, RFC 3339 to the second.
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub phase: String,
    pub model: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

fn disabled() -> bool {
    std::env::var("OKO_NO_JEV_LOG").is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Where the log lives, or `None` when there is no cache directory.
pub fn directory() -> Option<PathBuf> {
    std::env::var_os("OKO_JEV_LOG_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|path| path.join("oko").join("jev-log")))
}

/// Log one request. Logging must never fail a search: errors go to stderr.
pub fn record(call: &JevCallStats, model: &str) {
    if disabled() {
        return;
    }
    let Some(dir) = directory() else {
        return;
    };
    let entry = Entry {
        at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        workspace: WORKSPACE.get().cloned(),
        phase: call.phase.clone(),
        model: model.to_owned(),
        ok: call.success,
        status: call.http_status,
        error: call.error_class.clone(),
        ms: call.duration_ns / 1_000_000,
        input_tokens: call.usage.as_ref().and_then(|usage| usage.input_tokens),
        output_tokens: call.usage.as_ref().and_then(|usage| usage.output_tokens),
    };
    if let Err(error) = append(&dir, &entry) {
        eprintln!(
            "oko: cannot write the Jev log in {}: {error}",
            dir.display()
        );
    }
}

fn append(dir: &Path, entry: &Entry) -> Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.jsonl", &entry.at[..7]));
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    // One write per line keeps concurrent servers from interleaving.
    file.write_all(format!("{}\n", serde_json::to_string(entry)?).as_bytes())?;
    Ok(())
}

/// Entries from `since` (a `YYYY-MM-DD` UTC date) on, oldest file first.
/// Unreadable lines are skipped: the log is advisory.
pub fn read(dir: &Path, since: &str) -> Result<Vec<Entry>> {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_suffix(".jsonl"))
                    // A month file can hold entries from `since` on.
                    .is_some_and(|month| month.len() == 7 && month >= &since[..7])
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("Cannot read the Jev log."),
    };
    files.sort();
    let mut entries = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Cannot read {}.", path.display()))?;
        entries.extend(
            text.lines()
                .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
                .filter(|entry| entry.at.get(..10).is_some_and(|day| day >= since)),
        );
    }
    Ok(entries)
}

/// Calls, failures and tokens for one group of entries.
#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub calls: u64,
    pub failed: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub usd: f64,
}

impl Totals {
    fn add(&mut self, entry: &Entry) {
        self.calls += 1;
        self.failed += u64::from(!entry.ok);
        self.input_tokens += entry.input_tokens.unwrap_or(0);
        self.output_tokens += entry.output_tokens.unwrap_or(0);
        self.usd = self.input_tokens as f64 * USD_PER_MILLION_INPUT / 1e6;
    }
}

#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub since: String,
    pub usd_per_million_input_tokens: f64,
    pub total: Totals,
    pub days: BTreeMap<String, Totals>,
    pub phases: BTreeMap<String, Totals>,
    pub workspaces: BTreeMap<String, Totals>,
}

pub fn summarize(entries: &[Entry], since: &str) -> Summary {
    let mut summary = Summary {
        since: since.to_owned(),
        usd_per_million_input_tokens: USD_PER_MILLION_INPUT,
        ..Summary::default()
    };
    for entry in entries {
        summary.total.add(entry);
        summary
            .days
            .entry(entry.at[..10].to_owned())
            .or_default()
            .add(entry);
        summary
            .phases
            .entry(entry.phase.clone())
            .or_default()
            .add(entry);
        let workspace = entry
            .workspace
            .clone()
            .unwrap_or_else(|| "(unknown)".into());
        summary.workspaces.entry(workspace).or_default().add(entry);
    }
    summary
}

fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.0}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// Totals, then `label`: a day, phase or workspace path.
fn line(t: &Totals, label: &str) -> String {
    let failed = if t.failed > 0 {
        format!(" ({} failed)", t.failed)
    } else {
        String::new()
    };
    format!(
        "  {:>6} calls  {:>7} tokens  ${:.4}  {label}{failed}",
        t.calls,
        tokens(t.input_tokens),
        t.usd
    )
}

pub fn render(summary: &Summary, dir: &Path) -> String {
    let mut out = String::new();
    let t = &summary.total;
    if t.calls == 0 {
        out.push_str(&format!("No Jev requests since {}.\n", summary.since));
    } else {
        out.push_str(&format!(
            "Jev since {}: {} calls, {} input tokens, ${:.4} at ${} per million input tokens{}.\n",
            summary.since,
            t.calls,
            tokens(t.input_tokens),
            t.usd,
            summary.usd_per_million_input_tokens,
            if t.failed > 0 {
                format!(", {} failed", t.failed)
            } else {
                String::new()
            }
        ));
        for (title, groups) in [
            ("By day", &summary.days),
            ("By phase", &summary.phases),
            ("By workspace", &summary.workspaces),
        ] {
            out.push_str(&format!("\n{title}:\n"));
            for (label, totals) in groups {
                out.push_str(&line(totals, label));
                out.push('\n');
            }
        }
    }
    out.push_str(&format!(
        "\nLog: {} (no questions or code; OKO_NO_JEV_LOG=1 turns it off)\n",
        dir.display()
    ));
    out
}

/// `oko stats [--days N] [--json]`.
pub fn run(args: &[String]) -> Result<()> {
    let mut days: i64 = 30;
    let mut as_json = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => as_json = true,
            "--days" => {
                days = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| *value >= 1)
                    .context(format!(
                        "--days needs a whole number of at least 1.\n\n{USAGE}"
                    ))?;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("Unknown argument {other}.\n\n{USAGE}"),
        }
    }
    let dir = directory().context("No user cache directory for the Jev log.")?;
    let since = (chrono::Utc::now() - chrono::Duration::days(days - 1))
        .format("%Y-%m-%d")
        .to_string();
    let summary = summarize(&read(&dir, &since)?, &since);
    if as_json {
        println!("{}", json!(summary));
    } else {
        print!("{}", render(&summary, &dir));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: &str, phase: &str, ok: bool, input: Option<u64>) -> Entry {
        Entry {
            at: at.into(),
            workspace: Some("/w".into()),
            phase: phase.into(),
            model: "jev-latest".into(),
            ok,
            status: Some(if ok { 200 } else { 500 }),
            input_tokens: input,
            ..Entry::default()
        }
    }

    #[test]
    fn appends_monthly_files_and_reads_from_a_day_on() {
        let temp = tempfile::tempdir().unwrap();
        for e in [
            entry("2026-09-30T23:59:59Z", "normal", true, Some(10)),
            entry("2026-10-01T00:00:00Z", "normal", true, Some(20)),
            entry("2026-10-02T08:00:00Z", "further", false, None),
        ] {
            append(temp.path(), &e).unwrap();
        }
        fs::write(temp.path().join("notes.txt"), "ignored").unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(temp.path().join("2026-10.jsonl"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        assert!(temp.path().join("2026-09.jsonl").exists());
        let all = read(temp.path(), "2026-09-01").unwrap();
        assert_eq!(all.len(), 3);
        let october = read(temp.path(), "2026-10-01").unwrap();
        assert_eq!(
            october.iter().map(|e| e.input_tokens).collect::<Vec<_>>(),
            [Some(20), None]
        );
        assert!(
            read(&temp.path().join("missing"), "2026-01-01")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn totals_count_failures_and_price_input_tokens_only() {
        let entries = [
            entry("2026-10-01T10:00:00Z", "normal", true, Some(500_000)),
            entry("2026-10-01T10:00:01Z", "connected", true, Some(250_000)),
            entry("2026-10-02T09:00:00Z", "normal", false, None),
        ];
        let summary = summarize(&entries, "2026-10-01");
        assert_eq!(summary.total.calls, 3);
        assert_eq!(summary.total.failed, 1);
        assert_eq!(summary.total.input_tokens, 750_000);
        assert!((summary.total.usd - 0.0315).abs() < 1e-9);
        assert_eq!(summary.days["2026-10-01"].calls, 2);
        assert_eq!(summary.phases["normal"].calls, 2);
        assert_eq!(summary.workspaces["/w"].calls, 3);
        let text = render(&summary, Path::new("/log"));
        assert!(text.starts_with("Jev since 2026-10-01: 3 calls, 750k input tokens, $0.0315"));
        assert!(text.contains("1 failed"));
        assert!(
            render(&summarize(&[], "2026-10-01"), Path::new("/log"))
                .starts_with("No Jev requests since 2026-10-01.")
        );
    }

    #[test]
    fn entries_hold_no_question_or_code() {
        let line =
            serde_json::to_value(entry("2026-10-01T10:00:00Z", "normal", true, Some(1))).unwrap();
        let mut keys: Vec<_> = line.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "at",
                "inputTokens",
                "model",
                "ms",
                "ok",
                "phase",
                "status",
                "workspace"
            ]
        );
    }
}
