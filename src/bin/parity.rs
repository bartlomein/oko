//! Development-only differential-testing entry point. Never contacts Jev.
use anyhow::Result;
use oko::{ranking, search, stemmer};
use serde_json::{Value, json};
use std::io::{self, Read};
#[path = "../config.rs"]
mod config;

fn main() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .take(16 * 1024 * 1024)
        .read_to_string(&mut input)?;
    let fixture: Value = serde_json::from_str(&input)?;
    let question = fixture["question"].as_str().unwrap_or("");
    let mut chunks = vec![];
    if let Some(files) = fixture["files"].as_array() {
        for file in files {
            chunks.extend(search::chunk_text(
                file["path"].as_str().unwrap(),
                file["text"].as_str().unwrap(),
            ));
        }
    }
    let shortlist = search::rank_lexically(&chunks, question);
    let (request, omitted_count) = if !fixture["items"].is_null() {
        let items = ranking::parse_items(fixture["items"].clone())?;
        if items.is_empty() {
            (Value::Null, 0)
        } else {
            let (request, candidates) = ranking::prepare_request(question, &items)?;
            (request, items.len() - candidates.len())
        }
    } else {
        (Value::Null, 0)
    };
    let stems: Vec<String> = fixture["words"]
        .as_array()
        .map(|words| {
            words
                .iter()
                .map(|word| stemmer::stem(word.as_str().unwrap()))
                .collect()
        })
        .unwrap_or_default();
    let mut output = json!({"chunks": chunks, "shortlist": shortlist, "request": request, "omittedCount": omitted_count, "stems": stems});
    if let Some(dotenv) = fixture["dotenv"].as_str() {
        output["dotenvValues"] = serde_json::to_value(config::parse_env(dotenv))?;
    }
    if let Some(response) = fixture.get("response") {
        let items = ranking::parse_items(fixture["items"].clone())?;
        let (_, candidates) = ranking::prepare_request(question, &items)?;
        let limit = fixture["limit"].as_u64().unwrap_or(5) as usize;
        output["ranking"] = serde_json::to_value(ranking::rank_response(
            &candidates,
            response,
            limit,
            items.len(),
        )?)?;
    }
    println!("{output}");
    Ok(())
}
