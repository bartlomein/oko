mod auth;
mod benchmark;
mod config;

use anyhow::{Context, Result, bail};
use oko::{RankOptions, RankingIntent, parse_items, rank_items, search};
use serde::Serialize;
use std::{env, fs::File, io::Read, path::Path};

const USAGE: &str = "Usage: oko auth login|status|logout\n       oko ask [--deep [--max-steps N]] [--intent implementation|explanation|general] [--json] [--no-jev] \"question\"\n       oko rank --input items.json [--intent general|implementation|explanation] [--json] [--no-jev] \"question\"\n       oko benchmark --repo /path/to/repository [--repeats 1]\n       oko benchmark-items [--repeats 1]\n\nNormal ranking requires a TypeSafe key: run `oko auth login`, set TYPESAFE_API_KEY, or use .env.\n--intent defaults to implementation for ask, general for rank.\n--deep lets Jev choose further searches and reads; --max-steps optionally caps local actions.\n--no-jev skips intent-based reranking and uses lexical code search or preserves supplied item order.";

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

pub(crate) fn rank_code(
    question: &str,
    chunks: &[search::Chunk],
    key: Option<String>,
    no_jev: bool,
    intent: RankingIntent,
) -> Result<Vec<CodeResult>> {
    if !no_jev
        && key
            .as_deref()
            .is_none_or(|key| config::trim(key).is_empty())
    {
        bail!(
            "TYPESAFE_API_KEY is required for normal `oko ask`; run `oko auth login`, set it in the environment or .env; use `--no-jev` for explicit lexical-only benchmarking."
        );
    }
    let scored: Vec<(search::Chunk, f64)> = if no_jev {
        chunks
            .iter()
            .take(search::RESULT_LIMIT)
            .map(|chunk| (chunk.clone(), chunk.lexical_score))
            .collect()
    } else {
        let items: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| oko::RankItem {
                id: index.to_string(),
                text: chunk.text.clone(),
                source: Some(format!(
                    "{}:{}-{}",
                    chunk.path, chunk.start_line, chunk.end_line
                )),
            })
            .collect();
        let ranking = rank_items(
            question,
            &items,
            &RankOptions {
                api_key: key,
                limit: 30,
                no_jev: false,
                intent,
            },
        )?;
        let mut scored: Vec<_> = ranking
            .results
            .into_iter()
            .map(|item| {
                let index: usize = item.id.parse().expect("IDs generated locally");
                (chunks[index].clone(), item.score)
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
    Ok(scored
        .into_iter()
        .map(|(chunk, score)| CodeResult {
            path: chunk.path,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            score,
            text: chunk.text,
        })
        .collect())
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
        let mut investigation = None;
        let results = if parsed.deep {
            let key = key
                .as_deref()
                .context("TYPESAFE_API_KEY is required for --deep investigation.")?;
            let corpus = search::workspace_chunks(&cwd)?;
            let run = oko::investigate::investigate_with(
                &parsed.question,
                &corpus,
                parsed.intent,
                parsed.max_steps,
                |request| oko::ranking::call_jev(request, key),
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
            eprintln!(
                "Investigation: {} steps, {} Jev calls, stopped: {}",
                run.steps, run.jev_calls, run.stop_reason
            );
            investigation = Some(metadata);
            results
        } else {
            let shortlist = search::search_workspace(&cwd, &parsed.question)?;
            rank_code(
                &parsed.question,
                &shortlist,
                key,
                parsed.no_jev,
                parsed.intent,
            )?
        };
        let notice = "Lexical-only ranking requested via --no-jev.";
        if parsed.no_jev {
            eprintln!("Notice: {notice}");
        }
        if parsed.json {
            let mut output = serde_json::json!({"question": parsed.question, "ranking": if parsed.no_jev { "lexical" } else { "jev" }, "results": results});
            if let Some(metadata) = investigation {
                output["investigation"] = metadata;
            }
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
