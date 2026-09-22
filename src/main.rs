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

const USAGE: &str = "Usage: oko setup [--client codex|claude|opencode|all] [--root DIRECTORY] [--no-jev]\n       oko mcp [--root DIRECTORY] [--no-jev]\n       oko auth login|status|logout\n       oko ask [--intent implementation|explanation|general] [--json] [--no-jev] \"question\"\n       oko rank --input items.json [--intent general|implementation|explanation] [--json] [--no-jev] \"question\"\n       oko benchmark --repo /path/to/repository [--repeats 1]\n       oko benchmark-items [--repeats 1]\n\nNormal ranking requires a TypeSafe key: run `oko auth login`, set TYPESAFE_API_KEY, or use .env.\n--intent defaults to implementation for ask, general for rank.\n--no-jev skips intent-based reranking and uses lexical code search or preserves supplied item order.";

#[derive(Debug, PartialEq)]
struct Arguments {
    question: String,
    json: bool,
    no_jev: bool,
    input: Option<String>,
    intent: RankingIntent,
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
    /// Why results are in keyword order although Jev was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lexical_fallback: Option<&'static str>,
    /// Every candidate of the final ranking, best first: Jev's relevance for
    /// each, or keyword order without one. Shows whether missed code was
    /// judged irrelevant, fell just below the threshold, or was never
    /// shortlisted, and supplies the runners-up offered to the agent.
    pub candidates: Vec<CandidateScore>,
    /// The best candidates rated just below the relevance cutoff, with source,
    /// for a caller that has room to show them as lower-confidence matches.
    #[serde(skip)]
    pub runners_up: Vec<CodeResult>,
    pub jev_calls: Vec<JevCallStats>,
}

// Measured on 169 replayed agent questions: at 0.35, 11 of 30 added excerpts held
// expected code that was otherwise missing; at 0.2 it was 12 of 65.
const RUNNER_UP_FLOOR: f64 = 0.35;
const RUNNERS_UP: usize = 2;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CandidateScore {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Proposed for its connection to the strongest matches, not by keywords.
    /// Its relevance is scaled by `CONNECTED_WEIGHT`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub connected: bool,
}

fn lexical_candidates(chunks: &[search::Chunk]) -> Vec<CandidateScore> {
    chunks
        .iter()
        .map(|chunk| CandidateScore {
            path: chunk.path.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            score: None,
            connected: false,
        })
        .collect()
}

/// A provider that is slow, unreachable, or overloaded says nothing about the
/// request, so keyword order can stand in. Rejections such as a bad key or a
/// malformed request must still reach the operator.
fn provider_unavailable(calls: &[JevCallStats]) -> Option<&'static str> {
    let last = calls.last().filter(|call| !call.success)?;
    match (last.error_class.as_deref()?, last.http_status) {
        ("timeout", _) => Some("timeout"),
        ("transport" | "response_read", _) => Some("unreachable"),
        ("http_status", Some(429 | 500..)) => Some("unavailable"),
        _ => None,
    }
}

/// Candidates judged beside the keyword shortlist, in requests of their own
/// that run while the shortlist is judged, so they add no wait. They never
/// reach the excerpts, the possible matches or the recovery call: what the
/// agent is shown is decided by the shortlist alone. The ones Jev rates
/// relevant are named in the list of other files. Letting them into the
/// excerpts for multi-line questions was measured on SWE-Explore (848
/// issues) and made no difference; see docs/benchmark-results.md.
pub(crate) struct Further {
    /// Files one hop from the strongest matches: tests, callers, definitions.
    pub connected: Vec<search::Chunk>,
    /// The keyword matches after the shortlist; the recovery call draws on them.
    pub keywords: Vec<search::Chunk>,
}

/// What one search judges beside its shortlist.
pub(crate) fn further_candidates(
    snapshot: &std::sync::Arc<oko::search_cache::WorkspaceSnapshot>,
    shortlist: &[search::Chunk],
    question: &str,
    intent: RankingIntent,
) -> Further {
    // Chosen as the recovery call has always chosen them.
    let keywords = snapshot.rank_excluding(question, intent, shortlist);
    let mut connected = snapshot.connected_to(shortlist, question);
    connected.retain(|chunk| {
        !keywords.iter().any(|old| {
            (&old.path, old.start_line, old.end_line)
                == (&chunk.path, chunk.start_line, chunk.end_line)
        })
    });
    Further {
        connected,
        keywords,
    }
}

// Connected code is judged by criteria of its own, so its relevance is not on
// the shortlist's scale. At equal weight it crowded keyword candidates out of
// the list; between 0.3 and 0.5 Agent Retrieval Bench recall was flat.
const CONNECTED_WEIGHT: f64 = 0.4;

/// Relevance of each of `chunks`, by index. A failed or slow request yields
/// nothing: these candidates are extra, and the search must not depend on them.
fn judge_beside(
    question: &str,
    chunks: &[search::Chunk],
    corpus: &[search::Chunk],
    key: Option<&str>,
    intent: RankingIntent,
    timeout: std::time::Duration,
    phase: &'static str,
) -> (Vec<(usize, f64)>, Vec<JevCallStats>) {
    let mut calls = Vec::new();
    if chunks.is_empty() {
        return (Vec::new(), calls);
    }
    let judged = oko::preview::ranking_previews_with_context(question, chunks, corpus, intent)
        .and_then(|items| {
            ranking::rank_items_with_stats(
                question,
                &items,
                &RankOptions {
                    api_key: key.map(str::to_owned),
                    limit: 30,
                    no_jev: false,
                    intent,
                    timeout,
                },
                phase,
                &mut calls,
            )
        })
        .map(|ranking| {
            ranking
                .judged
                .into_iter()
                .map(|(id, score)| (id.parse().expect("IDs generated locally"), score))
                .collect()
        })
        .unwrap_or_default();
    (judged, calls)
}

fn lexical_results(chunks: &[search::Chunk]) -> Vec<CodeResult> {
    chunks
        .iter()
        .take(search::RESULT_LIMIT)
        .map(|chunk| CodeResult {
            path: chunk.path.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            score: chunk.lexical_score,
            text: chunk.text.clone(),
        })
        .collect()
}

/// How the shortlist is ordered.
pub(crate) enum Reranker {
    /// Keyword order only (`--no-jev`).
    Lexical,
    Jev {
        key: Option<String>,
        /// How long to wait for each request before ranking by keywords
        /// instead. `None` waits the default timeout and reports provider
        /// failures, so measurements never mistake keyword order for Jev's.
        patience: Option<std::time::Duration>,
    },
}

pub(crate) fn rank_code_with_stats(
    question: &str,
    chunks: &[search::Chunk],
    corpus: &[search::Chunk],
    reranker: Reranker,
    intent: RankingIntent,
    further: impl FnOnce() -> Result<Further>,
) -> Result<(Vec<CodeResult>, CodeRankingStats)> {
    let (no_jev, key, patience) = match reranker {
        Reranker::Lexical => (true, None, None),
        Reranker::Jev { key, patience } => (false, key, patience),
    };
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
        stats.candidates = lexical_candidates(chunks);
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
        let timeout = patience.unwrap_or(ranking::JEV_TIMEOUT);
        let further = further()?;
        let (beside_connected, beside_keywords) = (&further.connected, &further.keywords);
        let (first, beside) = std::thread::scope(|scope| {
            let key = key.as_deref();
            let connected = scope.spawn(move || {
                // The implementation criteria exclude tests and callers by
                // design; those are what a connection finds.
                let intent = RankingIntent::Related;
                judge_beside(
                    question,
                    beside_connected,
                    corpus,
                    key,
                    intent,
                    timeout,
                    "connected",
                )
            });
            let keywords = scope.spawn(move || {
                judge_beside(
                    question,
                    beside_keywords,
                    corpus,
                    key,
                    intent,
                    timeout,
                    "further",
                )
            });
            let first = ranking::rank_items_with_stats(
                question,
                &items,
                &RankOptions {
                    api_key: key.map(str::to_owned),
                    limit: 30,
                    no_jev: false,
                    intent,
                    timeout,
                },
                "normal",
                &mut stats.jev_calls,
            );
            let join = |handle: std::thread::ScopedJoinHandle<'_, _>| {
                handle.join().expect("ranking thread")
            };
            (first, [join(connected), join(keywords)])
        });
        let [(connected, connected_calls), (keywords, keyword_calls)] = beside;
        // Ahead of the shortlist's call: whether Jev was reachable is read from the last one.
        stats
            .jev_calls
            .splice(0..0, connected_calls.into_iter().chain(keyword_calls));
        let ranking = match first {
            Ok(ranking) => ranking,
            Err(error) => {
                stats.rerank_ms = rerank_started.elapsed().as_millis() as u64;
                stats.attempts = 1;
                let Some(reason) = patience.and(provider_unavailable(&stats.jev_calls)) else {
                    return Err(error);
                };
                stats.lexical_fallback = Some(reason);
                stats.candidates = lexical_candidates(chunks);
                return Ok((lexical_results(chunks), stats));
            }
        };
        stats.rerank_ms = rerank_started.elapsed().as_millis() as u64;
        stats.ranked_candidates = items.len().saturating_sub(ranking.omitted_count);
        stats.omitted_candidates = chunks.len().saturating_sub(stats.ranked_candidates);
        stats.attempts = usize::from(!items.is_empty());
        let mut selected = chunks.to_vec();
        let mut ranking = ranking;
        let listed = |chunk: &search::Chunk, score: f64, connected: bool| CandidateScore {
            path: chunk.path.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            score: Some(if connected {
                score * CONNECTED_WEIGHT
            } else {
                score
            }),
            connected,
        };
        let position = |id: &str| id.parse::<usize>().expect("IDs generated locally");
        // A recovery call replaces the ranking; what the first call judged still counts.
        let mut beside: Vec<CandidateScore> = ranking
            .judged
            .iter()
            .map(|(id, score)| listed(&chunks[position(id)], *score, false))
            .collect();
        beside.extend(
            connected
                .iter()
                .map(|(index, score)| listed(&further.connected[*index], *score, true)),
        );
        beside.extend(
            keywords
                .iter()
                .map(|(index, score)| listed(&further.keywords[*index], *score, false)),
        );
        if ranking.results.is_empty() && !items.is_empty() {
            let recovery_started = Instant::now();
            let mut recovery: Vec<_> = chunks.iter().take(8).cloned().collect();
            recovery.extend(further.keywords.iter().take(8).cloned());
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
                let recovered = ranking::rank_items_with_stats(
                    question,
                    &recovery_items,
                    &RankOptions {
                        api_key: key.clone(),
                        limit: 30,
                        no_jev: false,
                        intent,
                        timeout,
                    },
                    "recovery",
                    &mut stats.jev_calls,
                );
                stats.rerank_ms += started.elapsed().as_millis() as u64;
                stats.attempts += 1;
                ranking = match recovered {
                    Ok(ranking) => ranking,
                    // Jev already judged the first shortlist irrelevant; keyword
                    // order must not overrule that, so the miss stands.
                    Err(_)
                        if patience
                            .and(provider_unavailable(&stats.jev_calls))
                            .is_some() =>
                    {
                        return Ok((Vec::new(), stats));
                    }
                    Err(error) => return Err(error),
                };
                stats.recovered = !ranking.results.is_empty();
                selected = recovery;
            }
        }
        stats.candidates = ranking
            .judged
            .iter()
            .map(|(id, score)| listed(&selected[position(id)], *score, false))
            .collect();
        for candidate in beside {
            let seen = stats.candidates.iter().any(|old| {
                (&old.path, old.start_line, old.end_line)
                    == (&candidate.path, candidate.start_line, candidate.end_line)
            });
            if !seen {
                stats.candidates.push(candidate);
            }
        }
        // Stable: at equal relevance the shortlist's own order stands.
        stats
            .candidates
            .sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
        let accepted: std::collections::HashSet<&str> = ranking
            .results
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        stats.runners_up = ranking
            .judged
            .iter()
            .filter(|(id, score)| *score >= RUNNER_UP_FLOOR && !accepted.contains(id.as_str()))
            .take(RUNNERS_UP)
            .map(|(id, score)| {
                let chunk = &selected[id.parse::<usize>().expect("IDs generated locally")];
                CodeResult {
                    path: chunk.path.clone(),
                    start_line: chunk.start_line,
                    end_line: chunk.end_line,
                    score: *score,
                    text: chunk.text.clone(),
                }
            })
            .collect();
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
        let workspace = oko::search_cache::WorkspaceCache::new()
            .without_watching()
            .load(&cwd)?;
        let snapshot = workspace.snapshot;
        let (results, retrieval) = {
            let shortlist = if parsed.no_jev {
                snapshot.rank(&parsed.question)
            } else {
                snapshot.rank_with_intent(&parsed.question, parsed.intent)
            };
            let (results, stats) = rank_code_with_stats(
                &parsed.question,
                &shortlist,
                snapshot.chunks(),
                if parsed.no_jev {
                    Reranker::Lexical
                } else {
                    Reranker::Jev {
                        key,
                        patience: None,
                    }
                },
                parsed.intent,
                || {
                    Ok(further_candidates(
                        &snapshot,
                        &shortlist,
                        &parsed.question,
                        parsed.intent,
                    ))
                },
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
