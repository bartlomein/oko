mod auth;
mod benchmark;
mod config;
mod mcp;
mod setup;

use anyhow::{Context, Result, bail};
use oko::ranking::{self, JevCallStats};
use oko::{RankOptions, RankingIntent, parse_items, rank_items, search};
use serde::Serialize;
use serde_json::json;
use std::{env, fs::File, io::Read, path::Path, time::Instant};

const USAGE: &str = "Usage: oko setup [--root DIRECTORY] [--no-jev]\n       oko mcp [--root DIRECTORY] [--no-jev]\n       oko auth login|status|logout\n       oko ask [--deep [--max-steps N]] [--intent implementation|explanation|general] [--json] [--no-jev] \"question\"\n       oko rank --input items.json [--intent general|implementation|explanation] [--json] [--no-jev] \"question\"\n       oko benchmark --repo /path/to/repository [--repeats 1]\n       oko benchmark-items [--repeats 1]\n\nNormal ranking requires a TypeSafe key: run `oko auth login`, set TYPESAFE_API_KEY, or use .env.\n--intent defaults to implementation for ask, general for rank.\n--deep lets Jev choose further searches and reads; --max-steps optionally caps local actions.\n--no-jev skips intent-based reranking and uses lexical code search or preserves supplied item order.";

#[derive(Debug, PartialEq)]
struct Arguments {
    question: String,
    json: bool,
    no_jev: bool,
    input: Option<String>,
    intent: RankingIntent,
    deep: bool,
    max_steps: Option<usize>,
}

fn parse_arguments(args: &[String]) -> Result<Arguments> {
    let command = args.first().map(String::as_str).unwrap_or("");
    if command != "ask" && command != "rank" {
        bail!(
            "{}",
            if command.is_empty() {
                "A command is required.".to_string()
            } else {
                format!("Unknown command: {command}")
            }
        );
    }
    let mut parsed = Arguments {
        question: String::new(),
        json: false,
        no_jev: false,
        input: None,
        deep: false,
        max_steps: None,
        intent: if command == "ask" {
            RankingIntent::Implementation
        } else {
            RankingIntent::General
        },
    };
    let mut intent_seen = false;
    let mut question = vec![];
    let mut index = 1;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--intent" => {
                if intent_seen {
                    bail!("Provide --intent only once.");
                }
                index += 1;
                parsed.intent = args
                    .get(index)
                    .context("--intent requires implementation, explanation, or general.")?
                    .parse()?;
                intent_seen = true;
            }
            "--deep" if command == "ask" => parsed.deep = true,
            "--max-steps" if command == "ask" => {
                if parsed.max_steps.is_some() {
                    bail!("Provide --max-steps only once.");
                }
                index += 1;
                let steps: usize = args
                    .get(index)
                    .context("--max-steps requires a positive integer.")?
                    .parse()
                    .context("--max-steps requires a positive integer.")?;
                if steps == 0 {
                    bail!("--max-steps must be greater than zero.");
                }
                parsed.max_steps = Some(steps);
            }
            "--json" => parsed.json = true,
            "--no-jev" => parsed.no_jev = true,
            "--input" if command == "rank" => {
                if parsed.input.is_some()
                    || args
                        .get(index + 1)
                        .is_none_or(|value| value.is_empty() || value.starts_with('-'))
                {
                    bail!("Provide --input exactly once, followed by a JSON file path.");
                }
                index += 1;
                parsed.input = Some(args[index].clone());
            }
            _ if argument.starts_with('-') => bail!("Unknown flag: {argument}"),
            _ => question.push(argument.clone()),
        }
        index += 1;
    }
    parsed.question = config::trim(&question.join(" ")).to_string();
    if parsed.question.is_empty() {
        bail!("A non-empty question is required.");
    }
    if command == "rank" && parsed.input.is_none() {
        bail!("oko rank requires --input items.json.");
    }
    if parsed.max_steps.is_some() && !parsed.deep {
        bail!("--max-steps requires --deep.");
    }
    if parsed.deep && parsed.no_jev {
        bail!("--deep uses Jev and cannot be combined with --no-jev.");
    }
    Ok(parsed)
}

pub(crate) fn api_key(cwd: &Path) -> Result<Option<String>> {
    auth::api_key(cwd)
}

fn read_items(path: &Path) -> Result<Vec<oko::RankItem>> {
    let mut bytes = vec![];
    File::open(path)
        .with_context(|| format!("Could not open {}", path.display()))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("JSON input exceeds the 1 MiB limit.");
    }
    let value = serde_json::from_str(&String::from_utf8_lossy(&bytes))
        .context("Input file must contain valid JSON.")?;
    parse_items(value)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodeResult {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub score: f64,
    pub text: String,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodeRankingStats {
    pub shortlisted_candidates: usize,
    pub ranked_candidates: usize,
    pub omitted_candidates: usize,
    /// Budgeted request JSON before transport adds its model field.
    pub request_bytes: usize,
    pub preview_ms: u64,
    /// Client-side reranking, including HTTP, provider wait, and response parsing.
    pub rerank_ms: u64,
    pub attempts: usize,
    pub recovery_candidates: usize,
    pub recovered: bool,
    pub jev_calls: Vec<JevCallStats>,
}

pub(crate) fn rank_code_with_stats(
    question: &str,
    chunks: &[search::Chunk],
    corpus: &[search::Chunk],
    key: Option<String>,
    no_jev: bool,
    intent: RankingIntent,
    recovery_candidates: impl FnOnce() -> Result<Vec<search::Chunk>>,
) -> Result<(Vec<CodeResult>, CodeRankingStats)> {
    if !no_jev
        && key
            .as_deref()
            .is_none_or(|key| config::trim(key).is_empty())
    {
        bail!(
            "TYPESAFE_API_KEY is required for normal `oko ask`; run `oko auth login`, set it in the environment or .env; use `--no-jev` for explicit lexical-only benchmarking."
        );
    }
    let mut stats = CodeRankingStats {
        shortlisted_candidates: chunks.len(),
        ..Default::default()
    };
    let scored: Vec<(search::Chunk, f64)> = if no_jev {
        chunks
            .iter()
            .take(search::RESULT_LIMIT)
            .map(|chunk| (chunk.clone(), chunk.lexical_score))
            .collect()
    } else {
        let preview_started = Instant::now();
        let items = oko::preview::ranking_previews_with_context(question, chunks, corpus, intent)?;
        if !items.is_empty() {
            let (request, _) = oko::ranking::prepare_request_with_intent(question, &items, intent)?;
            stats.request_bytes = serde_json::to_vec(&request)?.len();
        }
        stats.preview_ms = preview_started.elapsed().as_millis() as u64;
        let rerank_started = Instant::now();
        let ranking = ranking::rank_items_with_stats(
            question,
            &items,
            &RankOptions {
                api_key: key.clone(),
                limit: 30,
                no_jev: false,
                intent,
            },
            "normal",
            &mut stats.jev_calls,
        )?;
        stats.rerank_ms = rerank_started.elapsed().as_millis() as u64;
        stats.ranked_candidates = items.len().saturating_sub(ranking.omitted_count);
        stats.omitted_candidates = chunks.len().saturating_sub(stats.ranked_candidates);
        stats.attempts = usize::from(!items.is_empty());
        let mut selected = chunks.to_vec();
        let mut ranking = ranking;
        if ranking.results.is_empty() && !items.is_empty() {
            let recovery_started = Instant::now();
            let mut recovery: Vec<_> = chunks.iter().take(8).cloned().collect();
            recovery.extend(recovery_candidates()?.into_iter().take(8));
            let recovery_items =
                oko::preview::recovery_previews_with_context(question, &recovery, corpus, intent)?;
            // Do not spend a second call on the same evidence with different IDs.
            let changed = recovery_items.iter().any(|candidate| {
                !items
                    .iter()
                    .any(|old| old.source == candidate.source && old.text == candidate.text)
            });
            stats.preview_ms += recovery_started.elapsed().as_millis() as u64;
            if changed {
                let (request, kept) =
                    oko::ranking::prepare_request_with_intent(question, &recovery_items, intent)?;
                stats.request_bytes += serde_json::to_vec(&request)?.len();
                stats.recovery_candidates = kept.len();
                let started = Instant::now();
                ranking = ranking::rank_items_with_stats(
                    question,
                    &recovery_items,
                    &RankOptions {
                        api_key: key,
                        limit: 30,
                        no_jev: false,
                        intent,
                    },
                    "recovery",
                    &mut stats.jev_calls,
                )?;
                stats.rerank_ms += started.elapsed().as_millis() as u64;
                stats.attempts += 1;
                stats.recovered = !ranking.results.is_empty();
                selected = recovery;
            }
        }
        let mut scored: Vec<_> = ranking
            .results
            .into_iter()
            .map(|item| {
                let index: usize = item.id.parse().expect("IDs generated locally");
                (selected[index].clone(), item.score)
            })
            .collect();
        scored.sort_by(|(a, a_score), (b, b_score)| {
            b_score
                .total_cmp(a_score)
                .then_with(|| search::compare_chunks(a, b))
        });
        scored.truncate(search::RESULT_LIMIT);
        scored
    };
    let results = scored
        .into_iter()
        .map(|(chunk, score)| CodeResult {
            path: chunk.path,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            score,
            text: chunk.text,
        })
        .collect();
    Ok((results, stats))
}

fn snippet(text: &str) -> String {
    let lines: Vec<_> = text.split('\n').collect();
    let mut excerpt = lines.iter().take(8).copied().collect::<Vec<_>>().join("\n");
    if lines.len() > 8 {
        excerpt.push_str("\n…");
    }
    excerpt
}

fn run() -> Result<()> {
    let command_started = Instant::now();
    let args: Vec<String> = env::args().skip(1).collect();
    let cwd = env::current_dir()?;
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!("{USAGE}");
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "--version") {
        println!("oko {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args
        .first()
        .is_some_and(|arg| arg == "benchmark" || arg == "benchmark-items")
    {
        return benchmark::run(&args, &cwd);
    }
    if args.first().is_some_and(|arg| arg == "auth") {
        return auth::run(&args[1..], &cwd);
    }
    if args.first().is_some_and(|arg| arg == "mcp") {
        return mcp::run(&args[1..], &cwd);
    }
    if args.first().is_some_and(|arg| arg == "setup") {
        return setup::run(&args[1..], &cwd);
    }
    let parsed = parse_arguments(&args).map_err(|error| anyhow::anyhow!("{error}\n\n{USAGE}"))?;
    let key = if parsed.no_jev { None } else { api_key(&cwd)? };
    if let Some(input) = parsed.input {
        let items = read_items(&cwd.join(input))?;
        let ranking = rank_items(
            &parsed.question,
            &items,
            &RankOptions {
                api_key: key,
                no_jev: parsed.no_jev,
                intent: parsed.intent,
                ..Default::default()
            },
        )?;
        if ranking.omitted_count > 0 {
            eprintln!(
                "Notice: {} trailing items omitted to fit the request size budget.",
                ranking.omitted_count
            );
        }
        if parsed.json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"question": parsed.question, "ranking": ranking.method, "results": ranking.results, "omittedCount": ranking.omitted_count})
                )?
            );
        } else {
            println!(
                "Ranking: {}\nQuestion: {}\n",
                if ranking.method == "input" {
                    "input-order (--no-jev)"
                } else {
                    "jev"
                },
                parsed.question
            );
            if ranking.results.is_empty() {
                println!("No matching items.");
            }
            for (index, item) in ranking.results.iter().enumerate() {
                let source = item
                    .source
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default();
                println!(
                    "{}. {}{} score={}\n{}\n",
                    index + 1,
                    item.id,
                    source,
                    item.score,
                    snippet(&item.text)
                );
            }
        }
    } else {
        if parsed.deep && key.is_none() {
            bail!("TYPESAFE_API_KEY is required for --deep investigation.");
        }
        let workspace = oko::search_cache::WorkspaceCache::new()
            .without_watching()
            .load(&cwd)?;
        let snapshot = workspace.snapshot;
        let mut investigation = None;
        let (results, retrieval) = if parsed.deep {
            let key = key
                .as_deref()
                .context("TYPESAFE_API_KEY is required for --deep investigation.")?;
            let mut provider_calls = Vec::new();
            let run = oko::investigate::investigate_snapshot_with(
                &parsed.question,
                &snapshot,
                parsed.intent,
                parsed.max_steps,
                |request| {
                    oko::ranking::call_jev_observed(request, key, "deep", &mut provider_calls)
                },
            )?;
            let results = run
                .results
                .iter()
                .map(|f| CodeResult {
                    path: f.chunk.path.clone(),
                    start_line: f.chunk.start_line,
                    end_line: f.chunk.end_line,
                    text: f.chunk.text.clone(),
                    score: f.score,
                })
                .collect();
            let mut metadata = serde_json::to_value(&run)?;
            metadata.as_object_mut().unwrap().remove("results");
            metadata["providerCalls"] = serde_json::to_value(&provider_calls)?;
            let retrieval = Some(json!({
                "attempts": provider_calls.len(),
                "jevCalls": provider_calls,
            }));
            eprintln!(
                "Investigation: {} steps, {} Jev calls, stopped: {}",
                run.steps, run.jev_calls, run.stop_reason
            );
            investigation = Some(metadata);
            (results, retrieval)
        } else {
            let shortlist = if parsed.no_jev {
                snapshot.rank(&parsed.question)
            } else {
                snapshot.rank_with_intent(&parsed.question, parsed.intent)
            };
            let (results, stats) = rank_code_with_stats(
                &parsed.question,
                &shortlist,
                snapshot.chunks(),
                key,
                parsed.no_jev,
                parsed.intent,
                || Ok(snapshot.rank_excluding(&parsed.question, parsed.intent, &shortlist)),
            )?;
            (results, Some(serde_json::to_value(stats)?))
        };
        let notice = "Lexical-only ranking requested via --no-jev.";
        if parsed.no_jev {
            eprintln!("Notice: {notice}");
        }
        if parsed.json {
            let mut output = serde_json::json!({"question": parsed.question, "ranking": if parsed.no_jev { "lexical" } else { "jev" }, "results": results});
            output["cache"] = serde_json::to_value(workspace.timings)?;
            if let Some(metadata) = investigation {
                output["investigation"] = metadata;
            }
            if let Some(stats) = retrieval {
                output["retrieval"] = stats;
            }
            output["timings"] = json!({
                "totalWallNs": command_started.elapsed().as_nanos() as u64,
                "totalMs": command_started.elapsed().as_millis() as u64,
            });
            if parsed.no_jev {
                output["notice"] = notice.into();
            }
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!(
                "Ranking: {}\nQuestion: {}\n",
                if parsed.no_jev {
                    "lexical-only (--no-jev)"
                } else {
                    "jev"
                },
                parsed.question
            );
            if results.is_empty() {
                println!("No matching chunks.");
            }
            for (index, result) in results.iter().enumerate() {
                println!(
                    "{}. {}:{}-{} score={}",
                    index + 1,
                    result.path,
                    result.start_line,
                    result.end_line,
                    result.score
                );
                for line in snippet(&result.text).split('\n') {
                    println!("   {line}");
                }
                println!();
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }
    #[test]
    fn arguments_and_errors() {
        assert_eq!(
            parse_arguments(&args(&["ask", "where", "is", "it?", "--json"]))
                .unwrap()
                .question,
            "where is it?"
        );
        assert!(parse_arguments(&args(&["rank", "q"])).is_err());
        assert!(parse_arguments(&args(&["ask", "--input", "x", "q"])).is_err());
        assert!(parse_arguments(&args(&["rank", "--input", "a", "--input", "b", "q"])).is_err());
        assert!(
            parse_arguments(&args(&["rank", "--input", "a", "q", "--no-jev"]))
                .unwrap()
                .no_jev
        );
    }
    #[test]
    fn bounded_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("items.json");
        std::fs::write(&path, "not JSON").unwrap();
        assert!(read_items(&path).is_err());
        std::fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert!(read_items(&path).unwrap_err().to_string().contains("1 MiB"));
    }
}
