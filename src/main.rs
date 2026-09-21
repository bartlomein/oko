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

const USAGE: &str = "Usage: oko setup [--client codex|claude|opencode|all] [--root DIRECTORY] [--no-jev]\n       oko mcp [--root DIRECTORY] [--no-jev]\n       oko auth login|status|logout\n       oko ask [--deep [--max-steps N]] [--intent implementation|explanation|general] [--json] [--no-jev] \"question\"\n       oko rank --input items.json [--intent general|implementation|explanation] [--json] [--no-jev] \"question\"\n       oko benchmark --repo /path/to/repository [--repeats 1]\n       oko benchmark-items [--repeats 1]\n\nNormal ranking requires a TypeSafe key: run `oko auth login`, set TYPESAFE_API_KEY, or use .env.\n--intent defaults to implementation for ask, general for rank.\n--deep lets Jev choose further searches and reads; --max-steps optionally caps local actions.\n--no-jev skips intent-based reranking and uses lexical code search or preserves supplied item order.";

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
    /// EXPERIMENT: position among the judged candidates before reranking
    /// (keyword shortlist first, then each extra batch in its own order).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<usize>,
    /// EXPERIMENT: proposed by the one-hop step, not by keywords.
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
            position: None,
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

/// EXPERIMENT: one Jev request per candidate group and intent, all at once.
/// Candidate ids are offset per group so they index the combined selection;
/// a candidate judged more than once keeps its highest relevance.
fn judge_in_parallel(
    question: &str,
    groups: &[(&[search::Chunk], usize)],
    corpus: &[search::Chunk],
    key: Option<&str>,
    intents: Vec<RankingIntent>,
    timeout: std::time::Duration,
    // Candidates below this index outrank accepted ones at or above it.
    first_pool: Option<usize>,
    stats: &mut CodeRankingStats,
) -> Result<ranking::ItemRanking> {
    // EXPERIMENT: OKO_EXPERIMENT_HOP_INTENT=general judges one-hop candidates
    // (which carry no keyword score) as related code, not as the implementation asked about.
    let hop_intent = match std::env::var("OKO_EXPERIMENT_HOP_INTENT").as_deref() {
        Ok("general") => Some(RankingIntent::General),
        // Judge connected candidates as connected code, which the
        // implementation criteria exclude by design (tests, callers).
        Ok("related") => Some(RankingIntent::Related),
        _ => None,
    };
    let jobs: Vec<_> = groups
        .iter()
        .filter(|(group, _)| !group.is_empty())
        .flat_map(|(group, offset)| {
            let connected = *offset > 0 && group[0].lexical_score == 0.0;
            let intents = match hop_intent {
                Some(intent) if connected => vec![intent],
                _ => intents.clone(),
            };
            intents
                .into_iter()
                .map(move |intent| (*group, *offset, intent))
        })
        .collect();
    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|&(group, offset, intent)| {
                scope.spawn(move || -> Result<_> {
                    let items = oko::preview::ranking_previews_with_context(
                        question, group, corpus, intent,
                    )?;
                    let mut calls = Vec::new();
                    let ranking = ranking::rank_items_with_stats(
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
                        &mut calls,
                    );
                    Ok((offset, ranking, calls))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("ranking thread"))
            .collect()
    });
    let mut best: std::collections::HashMap<usize, f64> = std::collections::HashMap::new();
    let mut failure = None;
    for outcome in outcomes {
        let (offset, ranking, calls) = outcome?;
        stats.jev_calls.extend(calls);
        match ranking {
            Ok(ranking) => {
                for (id, score) in ranking.judged {
                    let index = offset + id.parse::<usize>().expect("IDs generated locally");
                    let kept = best.entry(index).or_insert(score);
                    *kept = kept.max(score);
                }
            }
            // A slow or failed extra batch must not cost the search its first thirty.
            Err(_) if first_pool.is_some() && offset > 0 => {}
            Err(error) => failure = Some(error),
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    let mut judged: Vec<(String, f64)> = best
        .into_iter()
        .map(|(index, score)| (index.to_string(), score))
        .collect();
    judged.sort_by(|a, b| {
        b.1.total_cmp(&a.1).then_with(|| {
            a.0.parse::<usize>()
                .unwrap_or(0)
                .cmp(&b.0.parse::<usize>().unwrap_or(0))
        })
    });
    let mut accepted: Vec<_> = judged
        .iter()
        .filter(|(_, score)| *score > ranking::RELEVANCE_THRESHOLD)
        .collect();
    if let Some(boundary) = first_pool {
        // Stable: relevance order is kept within each pool.
        accepted.sort_by_key(|(id, _)| id.parse::<usize>().unwrap_or(0) >= boundary);
    }
    Ok(ranking::ItemRanking {
        method: "jev".into(),
        results: accepted
            .into_iter()
            .take(30)
            .map(|(id, score)| ranking::RankedItem {
                id: id.clone(),
                text: String::new(),
                source: None,
                score: *score,
            })
            .collect(),
        omitted_count: 0,
        judged,
    })
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

/// EXPERIMENT: how many further batches of 30 keyword candidates to judge.
pub(crate) fn experiment_extra_batches() -> usize {
    let pool = std::env::var("OKO_EXPERIMENT_POOL").unwrap_or_default();
    let size: usize = pool
        .trim_end_matches("first")
        .trim_end_matches("list")
        .parse()
        .unwrap_or(60);
    (size / 30).saturating_sub(1).max(1)
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
    recovery_candidates: impl FnOnce() -> Result<Vec<search::Chunk>>,
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
        // EXPERIMENT (replay only): judge more per search with parallel Jev
        // requests, which add no wall time. OKO_EXPERIMENT_POOL=60 also judges
        // the next 30 keyword candidates; OKO_EXPERIMENT_INTENTS=1 judges under
        // both the implementation and explanation criteria and keeps the higher
        // relevance, because scores swing with the intent an agent happens to pass.
        // "60first": as 60, but matches among the first 30 keep their rank ahead
        // of any from the next 30, which then only fill slots left free.
        let pool = std::env::var("OKO_EXPERIMENT_POOL").unwrap_or_default();
        // "90first" / "120first": as "60first" with two or three extra batches.
        // "90list": the extra batches never reach the excerpts, the possible
        // matches or the recovery call. What the agent is shown stays exactly
        // today's; their judged candidates only join the list of other files.
        let additive = pool.ends_with("list");
        let wider = pool == "60" || pool.ends_with("first") || additive;
        let first_pool = (pool.ends_with("first") || additive).then_some(chunks.len());
        let both_intents = std::env::var("OKO_EXPERIMENT_INTENTS").is_ok_and(|v| v == "1");
        let experiment = wider || both_intents;
        let mut recovery_candidates = Some(recovery_candidates);
        let mut extra = Vec::new();
        if wider {
            extra = (recovery_candidates.take().expect("unused"))()?;
        }
        let first = if experiment {
            judge_in_parallel(
                question,
                &std::iter::once((chunks, 0))
                    .chain(
                        extra
                            .chunks(30)
                            .enumerate()
                            .map(|(batch, group)| (group, chunks.len() + batch * 30)),
                    )
                    .collect::<Vec<_>>(),
                corpus,
                key.as_deref(),
                if both_intents {
                    vec![RankingIntent::Implementation, RankingIntent::Explanation]
                } else {
                    vec![intent]
                },
                timeout,
                first_pool,
                &mut stats,
            )
        } else {
            ranking::rank_items_with_stats(
                question,
                &items,
                &RankOptions {
                    api_key: key.clone(),
                    limit: 30,
                    no_jev: false,
                    intent,
                    timeout,
                },
                "normal",
                &mut stats.jev_calls,
            )
        };
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
        selected.extend(extra);
        let mut ranking = ranking;
        let candidate = |chunk: &search::Chunk, score: f64, position: usize| CandidateScore {
            path: chunk.path.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            score: Some(score),
            position: Some(position),
            // Only one-hop candidates carry no keyword score.
            connected: position >= chunks.len() && chunk.lexical_score == 0.0,
        };
        let mut listed_only = Vec::new();
        if additive {
            let position = |id: &str| id.parse::<usize>().expect("IDs generated locally");
            for (id, score) in &ranking.judged {
                listed_only.push(candidate(&selected[position(id)], *score, position(id)));
            }
            ranking
                .results
                .retain(|item| position(&item.id) < chunks.len());
            ranking.judged.retain(|(id, _)| position(id) < chunks.len());
        }
        if ranking.results.is_empty() && !items.is_empty() && (!experiment || additive) {
            let recovery_started = Instant::now();
            let mut recovery: Vec<_> = chunks.iter().take(8).cloned().collect();
            let next: Vec<search::Chunk> = match recovery_candidates.take() {
                Some(unseen) => unseen()?,
                // Already fetched for the extra batches: the next keyword matches.
                None => selected[chunks.len()..]
                    .iter()
                    .filter(|chunk| chunk.lexical_score != 0.0)
                    .cloned()
                    .collect(),
            };
            recovery.extend(next.into_iter().take(8));
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
                        api_key: key,
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
            .map(|(id, score)| {
                let position = id.parse::<usize>().expect("IDs generated locally");
                candidate(&selected[position], *score, position)
            })
            .collect();
        if additive {
            // Everything judged, best first; a recovery call re-judges some of the first thirty.
            for extra in listed_only {
                let seen = stats.candidates.iter().any(|old| {
                    (&old.path, old.start_line, old.end_line)
                        == (&extra.path, extra.start_line, extra.end_line)
                });
                if !seen {
                    stats.candidates.push(extra);
                }
            }
            stats
                .candidates
                .sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
        }
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
        // EXPERIMENT: "60first" has already put the first pool's matches ahead.
        if first_pool.is_none() {
            scored.sort_by(|(a, a_score), (b, b_score)| {
                b_score
                    .total_cmp(a_score)
                    .then_with(|| search::compare_chunks(a, b))
            });
        }
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
                if parsed.no_jev {
                    Reranker::Lexical
                } else {
                    Reranker::Jev {
                        key,
                        patience: None,
                    }
                },
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
