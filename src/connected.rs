//! EXPERIMENT: candidates one hop from the strongest keyword matches.
//!
//! Keyword search finds code that shares words with the question. The code a
//! change also touches often shares none: the test of a file, a caller of a
//! function, the definition of a type. This step proposes such files so the
//! reranker can judge them. It never displaces a keyword candidate.
//!
//! Links are lexical, not parsed: a file is connected to a seed when it
//! mentions a distinctive name the seed defines, defines one the seed
//! mentions, or is its test (or tested source) by file name. Names defined in
//! many files connect everything to everything and are ignored.

use crate::search::{Chunk, is_test_path};
use regex::Regex;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

/// One reranker request holds thirty candidates.
pub const CONNECTED_SLOTS: usize = 30;
/// Only the strongest keyword files seed the hop; weaker ones add noise.
const SEED_FILES: usize = 5;
/// A name defined in more files than this identifies none of them.
const MAX_DEFINERS: usize = 5;
const MIN_NAME: usize = 4;
/// A file named in the question counts for more than one found by keywords.
const NAMED_WEIGHT: f64 = 2.0;
const PAIR_SCORE: f64 = 1_000.0;

fn definition() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?m)^[ \t]*(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|export|default|public|private|protected|static|final|abstract|internal|open|sealed|data)[ \t]+)*(?:(?:fn|function\*?|def|fun|class|struct|enum|trait|interface|type|object|module)[ \t]+([A-Za-z_][A-Za-z0-9_]*)|func[ \t]+(?:\([^\n)]*\)[ \t]*)?([A-Za-z_][A-Za-z0-9_]*))",
        )
        .unwrap()
    })
}

fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| word.len() >= MIN_NAME && !word.as_bytes()[0].is_ascii_digit())
}

/// `pkg/foo_test.go`, `tests/test_foo.py`, `Foo.spec.ts` and `foo.rs` share the stem `foo`.
fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let base = name.split('.').next().unwrap_or(&name);
    let base = base.strip_prefix("test_").unwrap_or(base);
    let base = ["_test", "_tests", "_spec", "test", "tests", "spec"]
        .iter()
        .find_map(|suffix| base.strip_suffix(suffix))
        .unwrap_or(base);
    base.trim_matches(|c| c == '_' || c == '-').to_owned()
}

/// Up to `CONNECTED_SLOTS` chunks from files absent from `shortlist`, best first.
pub fn connected_to(corpus: &[Chunk], shortlist: &[Chunk], question: &str) -> Vec<Chunk> {
    let mut files: BTreeMap<&str, Vec<&Chunk>> = BTreeMap::new();
    for chunk in corpus {
        files.entry(chunk.path.as_str()).or_default().push(chunk);
    }
    let mut definers: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut defines: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (path, chunks) in &files {
        for chunk in chunks {
            for found in definition().captures_iter(&chunk.text) {
                let name = found.get(1).or(found.get(2)).unwrap().as_str();
                if name.len() >= MIN_NAME {
                    definers.entry(name).or_default().insert(path);
                    defines.entry(path).or_default().insert(name);
                }
            }
        }
    }
    definers.retain(|_, paths| paths.len() <= MAX_DEFINERS);
    // Only names with a definition can link files, so nothing else is kept.
    let mut mentions: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut mentioned_in: HashMap<&str, usize> = HashMap::new();
    for (path, chunks) in &files {
        let names: HashSet<&str> = chunks
            .iter()
            .flat_map(|chunk| identifiers(&chunk.text))
            .filter(|word| definers.contains_key(word))
            .collect();
        for name in &names {
            *mentioned_in.entry(name).or_default() += 1;
        }
        mentions.insert(path, names);
    }
    let total = files.len().max(1) as f64;
    let weight = |name: &str| {
        let rarity = (1.0 + total / mentioned_in.get(name).copied().unwrap_or(1) as f64).ln();
        let asked = question.contains(name);
        rarity * if asked { 3.0 } else { 1.0 } * if name.starts_with('_') { 0.3 } else { 1.0 }
    };

    let lowered = question.to_ascii_lowercase();
    let named = |path: &str| {
        let path = path.to_ascii_lowercase();
        let file = path.rsplit('/').next().unwrap_or(&path);
        lowered.contains(&path) || (file.len() >= 6 && file.contains('.') && lowered.contains(file))
    };
    let mut seeds: Vec<(&str, f64)> = files
        .keys()
        .filter(|path| named(path))
        .map(|path| (*path, NAMED_WEIGHT))
        .collect();
    let mut from_keywords = 0;
    for chunk in shortlist {
        if from_keywords == SEED_FILES {
            break;
        }
        if !seeds.iter().any(|(path, _)| *path == chunk.path) {
            seeds.push((chunk.path.as_str(), 1.0));
            from_keywords += 1;
        }
    }
    let shortlisted: HashSet<&str> = shortlist.iter().map(|chunk| chunk.path.as_str()).collect();
    let empty = HashSet::new();
    let mut scored: Vec<(f64, &str, HashSet<&str>)> = Vec::new();
    for path in files.keys().copied() {
        if shortlisted.contains(path) {
            continue;
        }
        let mut score = 0.0;
        let mut shared: HashSet<&str> = HashSet::new();
        if named(path) {
            // The question names this file and keyword search still missed it.
            score += PAIR_SCORE * 2.0;
        }
        for (seed, seed_weight) in &seeds {
            if *seed == path {
                continue;
            }
            let uses = mentions[path].intersection(defines.get(seed).unwrap_or(&empty));
            let provides = defines
                .get(path)
                .unwrap_or(&empty)
                .intersection(&mentions[seed]);
            for name in uses
                .chain(provides)
                .filter(|name| definers.contains_key(*name))
            {
                if shared.insert(name) {
                    score += seed_weight * weight(name);
                }
            }
            let key = stem(path);
            if key.len() >= 3 && key == stem(seed) && is_test_path(path) != is_test_path(seed) {
                score += seed_weight * PAIR_SCORE;
            }
        }
        if score > 0.0 {
            scored.push((score, path, shared));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(CONNECTED_SLOTS)
        .map(|(_, path, shared)| {
            // The part of the file that carries the connection.
            let best = files[path]
                .iter()
                .max_by(|a, b| {
                    let carried = |chunk: &Chunk| -> f64 {
                        identifiers(&chunk.text)
                            .collect::<HashSet<_>>()
                            .into_iter()
                            .filter(|word| shared.contains(word))
                            .map(weight)
                            .sum()
                    };
                    carried(a)
                        .total_cmp(&carried(b))
                        .then_with(|| b.start_line.cmp(&a.start_line))
                })
                .expect("a listed file has chunks");
            // These did not match the question; they carry no keyword score.
            Chunk {
                lexical_score: 0.0,
                ..(*best).clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(path: &str, text: &str) -> Chunk {
        Chunk {
            path: path.into(),
            start_line: 1,
            end_line: 3,
            text: text.into(),
            lexical_score: 1.0,
        }
    }

    #[test]
    fn proposes_the_test_the_caller_and_the_definition_but_not_hub_names() {
        let mut corpus = vec![
            chunk(
                "pkg/ledger.go",
                "func ParseLedger(raw string) Ledger {\n  return newLedgerState(raw)\n}",
            ),
            chunk(
                "pkg/ledger_test.go",
                "func TestParse(t *testing.T) {\n  ParseLedger(\"x\")\n}",
            ),
            chunk("cmd/report.go", "func Report() {\n  ParseLedger(input)\n}"),
            chunk(
                "pkg/state.go",
                "func newLedgerState(raw string) Ledger {\n  return Ledger{}\n}",
            ),
            chunk(
                "pkg/unrelated.go",
                "func Weather() string {\n  return \"rain\"\n}",
            ),
        ];
        // A name defined everywhere links nothing.
        for n in 0..8 {
            corpus.push(chunk(
                &format!("hub/h{n}.go"),
                "func String() string {\n  return \"\"\n}",
            ));
        }
        corpus[0]
            .text
            .push_str("\nfunc String() string { return \"\" }");
        let shortlist = vec![corpus[0].clone()];
        let found = connected_to(&corpus, &shortlist, "ledger parsing drops the last row");
        let paths: Vec<_> = found.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths[0], "pkg/ledger_test.go");
        assert!(paths.contains(&"cmd/report.go") && paths.contains(&"pkg/state.go"));
        assert!(!paths.contains(&"pkg/unrelated.go") && !paths.contains(&"pkg/ledger.go"));
        assert!(paths.iter().all(|path| !path.starts_with("hub/")));
        assert!(found.iter().all(|c| c.lexical_score == 0.0));
    }

    #[test]
    fn a_file_named_in_the_question_is_proposed_and_seeds_its_neighbours() {
        let corpus = vec![
            chunk(
                "src/context.go",
                "func (c *Context) GetError() error {\n  return lookupFailure(c)\n}",
            ),
            chunk(
                "src/failure.go",
                "func lookupFailure(c *Context) error {\n  return nil\n}",
            ),
            chunk("docs/readme.md", "how to build"),
        ];
        let shortlist = vec![corpus[2].clone()];
        let found = connected_to(
            &corpus,
            &shortlist,
            "./context.go:524: c.GetError undefined",
        );
        let paths: Vec<_> = found.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["src/context.go", "src/failure.go"]);
    }

    #[test]
    fn stems_pair_tests_with_sources_across_conventions() {
        for path in [
            "pkg/foo_test.go",
            "tests/test_foo.py",
            "src/Foo.spec.ts",
            "src/foo.rs",
            "FooTest.java",
        ] {
            assert_eq!(stem(path), "foo", "{path}");
        }
    }
}
