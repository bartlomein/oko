//! Native smoke benchmarks. Saved results exclude source snippets and provider bodies.
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Options {
    items: bool,
    repo: Option<PathBuf>,
    repeats: usize,
    baseline: Option<PathBuf>,
    no_jev: bool,
}
fn options(args: &[String], cwd: &Path) -> Result<Options> {
    let items = args.first().is_some_and(|s| s == "benchmark-items");
    let mut parsed = Options {
        items,
        repo: None,
        repeats: 1,
        baseline: None,
        no_jev: false,
    };
    let mut repeats_seen = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--no-jev" => parsed.no_jev = true,
            flag @ ("--repo" | "--repeats" | "--baseline") => {
                index += 1;
                let value = args
                    .get(index)
                    .filter(|s| !s.is_empty() && !s.starts_with('-'))
                    .with_context(|| format!("{flag} requires a value"))?;
                match flag {
                    "--repo" if !items && parsed.repo.is_none() => {
                        parsed.repo = Some(cwd.join(value))
                    }
                    "--baseline" if !items && parsed.baseline.is_none() => {
                        parsed.baseline = Some(cwd.join(value))
                    }
                    "--repeats" if !repeats_seen => {
                        parsed.repeats = value
                            .parse()
                            .context("--repeats must be an integer from 1 to 10")?;
                        repeats_seen = true;
                        if !(1..=10).contains(&parsed.repeats) {
                            bail!("--repeats must be an integer from 1 to 10");
                        }
                    }
                    _ => bail!("Unsupported or repeated option: {flag}"),
                }
            }
            flag => bail!("Unknown benchmark option: {flag}"),
        }
        index += 1;
    }
    if !items && parsed.repo.is_none() {
        bail!("oko benchmark requires --repo /path/to/repository");
    }
    Ok(parsed)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Location {
    path: String,
    start_line: usize,
    end_line: usize,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Range {
    start_line: usize,
    end_line: usize,
}
#[derive(Debug, Deserialize)]
struct Target {
    #[serde(flatten)]
    location: Location,
    sha256: String,
    function: Option<Range>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    id: String,
    question: String,
    #[serde(default)]
    expected: Vec<Target>,
    #[serde(default)]
    expected_ids: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct Fixture {
    cases: Vec<Case>,
    #[serde(default)]
    items: Vec<Value>,
}
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Scores {
    top1: bool,
    top5: bool,
    function_top1: bool,
    function_top5: bool,
}
fn score_locations(results: &[Location], expected: &[Target]) -> Scores {
    let hits = |functions: bool| {
        results
            .iter()
            .take(5)
            .map(|result| {
                expected.iter().any(|target| {
                    let (start, end) = if functions {
                        match &target.function {
                            Some(range) => (range.start_line, range.end_line),
                            None => return false,
                        }
                    } else {
                        (target.location.start_line, target.location.end_line)
                    };
                    result.path == target.location.path
                        && result.start_line <= end
                        && result.end_line >= start
                })
            })
            .collect::<Vec<_>>()
    };
    let exact = hits(false);
    let functions = hits(true);
    Scores {
        top1: exact.first() == Some(&true),
        top5: exact.contains(&true),
        function_top1: functions.first() == Some(&true),
        function_top5: functions.contains(&true),
    }
}
fn score_ids(ids: &[String], expected: &[String]) -> Scores {
    Scores {
        top1: if expected.is_empty() {
            ids.is_empty()
        } else {
            ids.first().is_some_and(|id| expected.contains(id))
        },
        top5: if expected.is_empty() {
            ids.is_empty()
        } else {
            ids.iter().take(5).any(|id| expected.contains(id))
        },
        ..Default::default()
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Run {
    id: String,
    question: String,
    repeat: usize,
    mode: String,
    seconds: f64,
    #[serde(flatten)]
    scores: Scores,
    results: Vec<Location>,
    ids: Vec<String>,
    omitted_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    mode: String,
    runs: usize,
    errors: usize,
    top1: usize,
    top5: usize,
    function_top1: usize,
    function_top5: usize,
    median_seconds: Option<f64>,
}
fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    })
}
fn summarize(runs: &[Run], modes: &[&str]) -> Vec<Summary> {
    modes
        .iter()
        .map(|mode| {
            let selected: Vec<_> = runs.iter().filter(|r| r.mode == *mode).collect();
            let mut seconds: Vec<_> = selected
                .iter()
                .filter(|r| r.error.is_none())
                .map(|r| r.seconds)
                .collect();
            Summary {
                mode: (*mode).into(),
                runs: selected.len(),
                errors: selected.iter().filter(|r| r.error.is_some()).count(),
                top1: selected.iter().filter(|r| r.scores.top1).count(),
                top5: selected.iter().filter(|r| r.scores.top5).count(),
                function_top1: selected.iter().filter(|r| r.scores.function_top1).count(),
                function_top5: selected.iter().filter(|r| r.scores.function_top5).count(),
                median_seconds: median(&mut seconds),
            }
        })
        .collect()
}
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Snapshot {
    commit: String,
    status: String,
    hashes: BTreeMap<String, String>,
}
fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    if !output.status.success() {
        bail!("Could not read repository state with git");
    }
    Ok(String::from_utf8(output.stdout)?.trim().into())
}
fn snapshot(repo: &Path, cases: &[Case]) -> Result<Snapshot> {
    let mut hashes = BTreeMap::new();
    for case in cases {
        for target in &case.expected {
            let bytes = fs::read(repo.join(&target.location.path)).with_context(|| {
                format!("Could not read fixture target {}", target.location.path)
            })?;
            let hash = format!("{:x}", Sha256::digest(bytes));
            if hash != target.sha256 {
                bail!(
                    "Fixture is stale: {}. Review its expected lines before updating its hash.",
                    target.location.path
                );
            }
            hashes.insert(target.location.path.clone(), hash);
        }
    }
    Ok(Snapshot {
        commit: git(repo, &["rev-parse", "HEAD"])?,
        status: git(repo, &["status", "--porcelain"])?,
        hashes,
    })
}
// Reader runs concurrently so full stdout pipes cannot deadlock the child. Both
// output size and execution time are bounded; stderr/provider details are discarded.
fn execute(args: &[String], cwd: &Path, key: Option<&str>) -> Result<Value> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(key) = key {
        command.env("TYPESAFE_API_KEY", key);
    }
    let mut child = command
        .spawn()
        .context("CLI failed or returned invalid output")?;
    let stdout = child.stdout.take().context("CLI stdout unavailable")?;
    let reader = thread::spawn(move || {
        let mut bytes = vec![];
        stdout
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            bail!("Timed out after 60 seconds");
        }
        thread::sleep(Duration::from_millis(2));
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("CLI output reader failed"))??;
    if !status.success() || bytes.len() > 4 * 1024 * 1024 {
        bail!("CLI failed or returned invalid output");
    }
    serde_json::from_slice(&bytes).context("CLI failed or returned invalid output")
}
fn parse_code_output(output: &Value, mode: &str) -> Result<Vec<Location>> {
    if output["ranking"] != mode || !output["results"].is_array() {
        bail!("Invalid CLI output");
    }
    let results: Vec<Location> = serde_json::from_value(output["results"].clone())?;
    if results
        .iter()
        .any(|r| r.start_line < 1 || r.end_line < r.start_line)
    {
        bail!("Invalid CLI output");
    }
    Ok(results)
}
fn parse_item_output(output: &Value, mode: &str) -> Result<(Vec<String>, usize)> {
    if output["ranking"] != mode {
        bail!("Invalid CLI output");
    }
    let results = output["results"].as_array().context("Invalid CLI output")?;
    let ids = results
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .map(String::from)
                .context("Invalid CLI output")
        })
        .collect::<Result<Vec<_>>>()?;
    let omitted = output["omittedCount"]
        .as_u64()
        .context("Invalid CLI output")? as usize;
    Ok((ids, omitted))
}
fn baseline(path: &Path, before: &Snapshot, fixture: &Fixture) -> Result<Value> {
    let saved: Value = serde_json::from_slice(&fs::read(path)?)?;
    let old: Snapshot = serde_json::from_value(saved["repository"].clone())?;
    if saved["repositoryUnchanged"] != true || old != *before {
        bail!("Baseline repository snapshot differs; use a comparable baseline.");
    }
    let mut runs = vec![];
    for run in saved["runs"].as_array().context("Invalid baseline runs")? {
        let case = fixture
            .cases
            .iter()
            .find(|c| run["id"] == c.id && run["question"] == c.question)
            .context("Baseline question differs")?;
        let error = run
            .get("error")
            .filter(|e| !e.is_null())
            .map(|_| "Saved run failed".to_string());
        let results: Vec<Location> = if error.is_some() {
            vec![]
        } else {
            serde_json::from_value(run["results"].clone())?
        };
        let scores = score_locations(&results, &case.expected);
        runs.push(Run {
            id: case.id.clone(),
            question: case.question.clone(),
            repeat: 1,
            mode: run["mode"]
                .as_str()
                .context("Invalid baseline mode")?
                .into(),
            seconds: run["seconds"].as_f64().context("Invalid baseline timing")?,
            scores,
            results,
            ids: vec![],
            omitted_count: 0,
            error,
        });
    }
    Ok(
        json!({"path":path,"startedAt":saved["startedAt"],"summary":summarize(&runs,&["lexical","jev"])}),
    )
}
pub(crate) fn run(args: &[String], cwd: &Path) -> Result<()> {
    let options = options(args, cwd)?;
    let fixture_value: Value = serde_json::from_str(if options.items {
        include_str!("../benchmarks/support-tickets.json")
    } else {
        include_str!("../benchmarks/telemetry-studio.json")
    })?;
    let fixture: Fixture = serde_json::from_value(fixture_value.clone())?;
    let repo = match &options.repo {
        Some(path) => fs::canonicalize(path).context("Could not open benchmark repository")?,
        None => cwd.to_owned(),
    };
    let before = if options.items {
        None
    } else {
        Some(snapshot(&repo, &fixture.cases)?)
    };
    let baseline = match (&options.baseline, &before) {
        (Some(path), Some(before)) => Some(baseline(path, before, &fixture)?),
        _ => None,
    };
    let key = if options.no_jev {
        None
    } else {
        Some(super::api_key(cwd)?.context("Add TYPESAFE_API_KEY to the environment or current directory .env before benchmarking.")?)
    };
    let started_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let directory = cwd.join("benchmarks/results").join(format!(
        "{}{}",
        if options.items { "items-" } else { "" },
        started_at.replace(':', "-")
    ));
    fs::create_dir_all(&directory)?;
    let input = directory.join("items.json");
    if options.items {
        fs::write(&input, serde_json::to_vec_pretty(&fixture.items)?)?;
    }
    println!(
        "Testing {} questions, {} repeat(s); at most {} Jev requests.",
        fixture.cases.len(),
        options.repeats,
        if options.no_jev {
            0
        } else {
            fixture.cases.len() * options.repeats
        }
    );
    let local_mode = if options.items { "input" } else { "lexical" };
    let mut runs = vec![];
    for repeat in 0..options.repeats {
        for (index, case) in fixture.cases.iter().enumerate() {
            let modes = if options.no_jev {
                vec![local_mode]
            } else if (index + repeat) % 2 == 0 {
                vec![local_mode, "jev"]
            } else {
                vec!["jev", local_mode]
            };
            for mode in modes {
                let mut command = vec![
                    if options.items { "rank" } else { "ask" }.into(),
                    case.question.clone(),
                    "--json".into(),
                ];
                if options.items {
                    command.extend(["--input".into(), input.to_string_lossy().into_owned()]);
                }
                if mode != "jev" {
                    command.push("--no-jev".into());
                }
                let start = Instant::now();
                let output = execute(&command, &repo, key.as_deref());
                let mut run = Run {
                    id: case.id.clone(),
                    question: case.question.clone(),
                    repeat: repeat + 1,
                    mode: mode.into(),
                    seconds: 0.0,
                    scores: Scores::default(),
                    results: vec![],
                    ids: vec![],
                    omitted_count: 0,
                    error: None,
                };
                let evaluated = output.and_then(|value| {
                    if options.items {
                        let (ids, omitted) = parse_item_output(&value, mode)?;
                        run.scores = score_ids(&ids, &case.expected_ids);
                        run.ids = ids;
                        run.omitted_count = omitted;
                    } else {
                        let locations = parse_code_output(&value, mode)?;
                        run.scores = score_locations(&locations, &case.expected);
                        run.results = locations;
                    }
                    Ok(())
                });
                if let Err(error) = evaluated {
                    run.error = Some(if error.to_string() == "Timed out after 60 seconds" {
                        error.to_string()
                    } else {
                        "CLI failed or returned invalid output".into()
                    });
                }
                run.seconds = start.elapsed().as_secs_f64();
                println!(
                    "{mode:7} {:20} first={} top5={} {:.3}s{}",
                    run.id,
                    run.scores.top1,
                    run.scores.top5,
                    run.seconds,
                    if run.error.is_some() { " ERROR" } else { "" }
                );
                runs.push(run);
            }
        }
    }
    let unchanged = before
        .as_ref()
        .is_none_or(|before| snapshot(&repo, &fixture.cases).is_ok_and(|after| after == *before));
    let modes = if options.no_jev {
        vec![local_mode]
    } else {
        vec![local_mode, "jev"]
    };
    let summary = summarize(&runs, &modes);
    let report = json!({"metricVersion":2,"implementation":"rust","version":env!("CARGO_PKG_VERSION"),"timingScope":"end-to-end CLI including startup; build excluded","startedAt":started_at,"repo":repo,"repository":before,"repositoryUnchanged":unchanged,"baseline":baseline,"repeats":options.repeats,"fixture":fixture_value,"summary":summary,"runs":runs,"okoCommit":git(cwd,&["rev-parse","HEAD"]).ok(),"okoStatus":git(cwd,&["status","--porcelain"]).ok()});
    fs::write(
        directory.join("report.json"),
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    let mut markdown = format!(
        "# Oko Rust {} benchmark\n\n{} fixed questions, {} repeat(s). Smoke tests, not a general accuracy estimate.\n\nTimes are end-to-end CLI including startup, file loading/search, and Jev requests; build time is excluded. No warm-up; mode order alternates. Errors count as misses and are excluded from median time.\n\nRepository unchanged: {unchanged}.\n\n",
        if options.items {
            "support-ticket"
        } else {
            "code"
        },
        fixture.cases.len(),
        options.repeats
    );
    if options.items {
        markdown.push_str("Input baseline preserves supplied order; it does not search. No-match cases count as correct only for an empty result. Historical TypeScript item reports measured the API only, so their times are not directly comparable.\n\n");
    } else {
        markdown.push_str("Exact hits overlap verified implementation lines; function hits overlap the verified function. Same-file matches outside the function do not count.\n\n");
    }
    markdown.push_str("| Run | Mode | Correct first | Correct top five | Function first | Function top five | Median seconds | Errors |\n|---|---|---:|---:|---:|---:|---:|---:|\n");
    let row = |label: &str, s: &Value| {
        format!(
            "| {label} | {} | {}/{} | {}/{} | {}/{} | {}/{} | {} | {} |\n",
            s["mode"].as_str().unwrap_or("?"),
            s["top1"],
            s["runs"],
            s["top5"],
            s["runs"],
            s["functionTop1"],
            s["runs"],
            s["functionTop5"],
            s["runs"],
            s["medianSeconds"]
                .as_f64()
                .map_or_else(|| "N/A".into(), |n| format!("{n:.3}")),
            s["errors"]
        )
    };
    if let Some(baseline) = &baseline
        && let Some(summary) = baseline["summary"].as_array()
    {
        for s in summary {
            markdown.push_str(&row("Baseline (rescored)", s));
        }
    }
    for s in &summary {
        markdown.push_str(&row("Current", &serde_json::to_value(s)?));
    }
    markdown.push_str(
        "\n| Question | Mode | First | Top five | Seconds | Error |\n|---|---|---|---|---:|---|\n",
    );
    for r in &runs {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {:.3} | {} |\n",
            r.question.replace('|', "\\|"),
            r.mode,
            r.scores.top1,
            r.scores.top5,
            r.seconds,
            r.error.as_deref().unwrap_or("")
        ));
    }
    fs::write(directory.join("report.md"), markdown)?;
    println!("Report: {}", directory.join("report.md").display());
    if !unchanged || runs.iter().any(|r| r.error.is_some()) {
        bail!("Benchmark report saved, but errors or repository changes make this run invalid.");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> Target {
        serde_json::from_value(json!({"path":"x.rs","startLine":81,"endLine":90,"sha256":"unused","function":{"startLine":28,"endLine":103}})).unwrap()
    }
    #[test]
    fn distinguishes_function_and_exact_hits() {
        let scores = score_locations(
            &[Location {
                path: "x.rs".into(),
                start_line: 28,
                end_line: 40,
            }],
            &[target()],
        );
        assert_eq!(
            scores,
            Scores {
                function_top1: true,
                function_top5: true,
                ..Default::default()
            }
        );
        assert!(
            score_locations(
                &[Location {
                    path: "x.rs".into(),
                    start_line: 90,
                    end_line: 120
                }],
                &[target()]
            )
            .top1
        );
        assert!(
            !score_locations(
                &[Location {
                    path: "x.rs".into(),
                    start_line: 104,
                    end_line: 120
                }],
                &[target()]
            )
            .function_top5
        );
    }
    #[test]
    fn ignores_sixth_result_and_wrong_paths() {
        let mut results = vec![
            Location {
                path: "other.rs".into(),
                start_line: 1,
                end_line: 200
            };
            5
        ];
        results.push(Location {
            path: "x.rs".into(),
            start_line: 1,
            end_line: 200,
        });
        assert_eq!(score_locations(&results, &[target()]), Scores::default());
    }
    #[test]
    fn no_match_requires_no_results() {
        assert!(score_ids(&[], &[]).top1);
        assert!(!score_ids(&["a".into()], &[]).top5);
        assert!(score_ids(&["a".into(), "b".into()], &["b".into()]).top5);
        assert!(!score_ids(&["a".into(), "b".into()], &["b".into()]).top1);
    }
    #[test]
    fn timing_medians_and_invalid_outputs() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [3.0, 1.0]), Some(2.0));
        assert_eq!(median(&mut [9.0, 1.0, 2.0]), Some(2.0));
        assert!(
            parse_code_output(
                &json!({"ranking":"jev","results":[{"path":"x","startLine":0,"endLine":1}]}),
                "jev"
            )
            .is_err()
        );
        assert!(
            parse_item_output(
                &json!({"ranking":"jev","results":[{}],"omittedCount":0}),
                "jev"
            )
            .is_err()
        );
    }
    #[test]
    fn stale_sources_fail_before_requests() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("x.rs"), "changed").unwrap();
        let case = Case {
            id: "test".into(),
            question: "test".into(),
            expected: vec![target()],
            expected_ids: vec![],
        };
        assert!(
            snapshot(dir.path(), &[case])
                .unwrap_err()
                .to_string()
                .contains("Fixture is stale")
        );
    }
    #[test]
    fn argument_bounds() {
        let parse = |args: &[&str]| {
            options(
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                Path::new("/tmp"),
            )
        };
        assert!(parse(&["benchmark"]).is_err());
        assert!(parse(&["benchmark-items", "--repeats", "11"]).is_err());
        assert!(parse(&["benchmark-items", "--repo", "x"]).is_err());
        assert!(
            parse(&["benchmark", "--repo", "x", "--repeats", "2", "--no-jev"])
                .unwrap()
                .no_jev
        );
    }
    #[test]
    fn summary_counts_errors_as_misses_and_excludes_their_timing() {
        let make = |seconds, error| Run {
            id: "x".into(),
            question: "q".into(),
            repeat: 1,
            mode: "lexical".into(),
            seconds,
            scores: Scores::default(),
            results: vec![],
            ids: vec![],
            omitted_count: 0,
            error,
        };
        let summary = summarize(
            &[
                make(1.0, None),
                make(3.0, None),
                make(100.0, Some("failure".into())),
            ],
            &["lexical"],
        );
        assert_eq!(summary[0].runs, 3);
        assert_eq!(summary[0].errors, 1);
        assert_eq!(summary[0].median_seconds, Some(2.0));
        assert_eq!(summary[0].top1, 0);
    }
}
