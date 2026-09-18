use crate::stemmer::stemmer;
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    process::Command,
    sync::OnceLock,
};

pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const CHUNK_LINES: usize = 40;
pub const CHUNK_OVERLAP: usize = 5;
pub const FUNCTION_CHUNK_LINES: usize = 120;
pub const SHORTLIST_LIMIT: usize = 30;
pub const RESULT_LIMIT: usize = 5;
// Retrieve broadly in memory, then keep the existing small Jev request.
const RETRIEVAL_WINDOW: usize = 100;
const RRF_CONSTANT: f64 = 60.0;
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "by", "do", "does", "for", "how", "in", "is", "it", "its", "of", "on",
    "or", "the", "this", "that", "to", "what", "when", "where", "which", "who", "why",
];
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    pub lexical_score: f64,
}
struct Patterns {
    acronym: Regex,
    camel: Regex,
    words: Regex,
    lines: Regex,
    extension: Regex,
    declaration: Regex,
    comment: Regex,
    symbol: Regex,
    symbol_extension: Regex,
    typed_symbol: Regex,
    typed_extension: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        // ECMAScript whitespace, excluding Rust regex's additional U+0085.
        let ws = "[\\t\\n\\v\\f\\r \\u{00a0}\\u{1680}\\u{2000}-\\u{200a}\\u{2028}\\u{2029}\\u{202f}\\u{205f}\\u{3000}\\u{feff}]";
        Patterns {
            // Deliberately conservative declaration hints, not a language parser.
            // Unsupported syntax still receives ordinary content/path ranking.
            typed_extension: Regex::new(r"\.(?:java|cs|c|h|cc|cpp|hpp)$").unwrap(),
            typed_symbol: Regex::new(r"(?m)^[ \t]*(?:[A-Za-z_][A-Za-z0-9_.<>,?\[\]:*&]*[ \t]+)+([A-Za-z_][A-Za-z0-9_]*)[ \t]*\([^;\n]*\)[ \t]*(?:\{|throws\b)").unwrap(),
            symbol_extension: Regex::new(r"\.(?:rs|[cm]?js|jsx|ts|tsx|py|go|java|cs|c|h|cc|cpp|hpp|rb|php|swift|kt)$").unwrap(),
            symbol: Regex::new(r"(?m)^\s*(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|export|default|public|private|protected|static|final|override|abstract|internal|open|suspend)\s+)*(?:(?:fn|function\*?|def|fun)\s+([A-Za-z_][A-Za-z0-9_]*)|func\s+(?:\([^\n)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)|(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:async\s+)?(?:\([^\n)]*\)|[A-Za-z_][A-Za-z0-9_]*)\s*=>)").unwrap(),
            acronym: Regex::new("([A-Z]+)([A-Z][a-z])").unwrap(),
            camel: Regex::new("([a-z0-9])([A-Z])").unwrap(),
            words: Regex::new("[a-z0-9]+").unwrap(),
            lines: Regex::new("\\r\\n|\\n|\\r").unwrap(),
            extension: Regex::new(r"\.(?:rs|[cm]?js|jsx|ts|tsx|py)$").unwrap(),
            declaration: Regex::new(&format!(r"^{ws}*(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|export|default){ws}+)*(?:fn|function\*?|def){ws}+[A-Za-z0-9_]+")).unwrap(),
            comment: Regex::new(&format!(r"^{ws}*(?://|#|/\*\*|\*)")).unwrap(),
        }
    })
}
pub fn tokenize(value: &str) -> Vec<String> {
    let p = patterns();
    let a = p.acronym.replace_all(value, "$1 $2");
    let b = p.camel.replace_all(&a, "$1 $2").to_lowercase();
    p.words
        .find_iter(&b)
        .map(|m| m.as_str().to_owned())
        .collect()
}
pub fn compare_text(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
pub fn compare_chunks(a: &Chunk, b: &Chunk) -> Ordering {
    b.lexical_score
        .partial_cmp(&a.lexical_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| compare_text(&a.path, &b.path))
        .then(a.start_line.cmp(&b.start_line))
        .then(a.end_line.cmp(&b.end_line))
}
pub fn chunk_text(path: &str, text: &str) -> Vec<Chunk> {
    let p = patterns();
    let mut lines: Vec<&str> = p.lines.split(text).collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let mut declarations = HashSet::new();
    let mut boundaries = vec![0];
    if p.extension.is_match(path) {
        for (i, line) in lines.iter().enumerate() {
            if !p.declaration.is_match(line) {
                continue;
            }
            let mut start = i;
            while start > 0 && p.comment.is_match(lines[start - 1]) {
                start -= 1;
            }
            declarations.insert(start);
            if !boundaries.contains(&start) {
                boundaries.push(start);
            }
        }
    }
    if !boundaries.contains(&lines.len()) {
        boundaries.push(lines.len());
    }
    let mut chunks = vec![];
    for section in boundaries.windows(2) {
        let (mut start, section_end) = (section[0], section[1]);
        let limit = if declarations.contains(&start) {
            FUNCTION_CHUNK_LINES
        } else {
            CHUNK_LINES
        };
        while start < section_end {
            let end = section_end.min(start + limit);
            chunks.push(Chunk {
                path: path.into(),
                start_line: start + 1,
                end_line: end,
                text: lines[start..end].join("\n"),
                lexical_score: 0.0,
            });
            if end == section_end {
                break;
            }
            start += limit - CHUNK_OVERLAP;
        }
    }
    chunks
}
fn normalize(term: String, stems: &mut HashMap<String, String>) -> String {
    stems.entry(term).or_insert_with_key(|s| stemmer(s)).clone()
}

/// BM25 over a snapshot: cache tokens once and retain corpus-wide statistics
/// when filtering candidates. Body, path and declaration names are independent fields.
pub(crate) struct PreparedCorpus<'a> {
    chunks: Vec<PreparedChunk<'a>>,
    content_stats: FieldStats,
    path_stats: FieldStats,
    symbol_stats: FieldStats,
}
struct PreparedChunk<'a> {
    chunk: &'a Chunk,
    content: Field,
    path: Field,
    symbols: Vec<Symbol<'a>>,
}
struct Symbol<'a> {
    name: &'a str,
    field: Field,
    reference_weight: f64,
}
struct Candidate<'a> {
    chunk: &'a Chunk,
    baseline: f64,
    symbol_aware: f64,
    fusion: f64,
}

fn compare_sources(a: &Chunk, b: &Chunk) -> Ordering {
    compare_text(&a.path, &b.path)
        .then(a.start_line.cmp(&b.start_line))
        .then(a.end_line.cmp(&b.end_line))
        .then_with(|| compare_text(&a.text, &b.text))
}

fn redundant_source(a: &Chunk, b: &Chunk) -> bool {
    if a.path != b.path || a.start_line > a.end_line || b.start_line > b.end_line {
        return false;
    }
    let start = a.start_line.max(b.start_line);
    let end = a.end_line.min(b.end_line);
    if start > end {
        return false;
    }
    let overlap = end.saturating_sub(start).saturating_add(1);
    let shorter = (a.end_line - a.start_line)
        .min(b.end_line - b.start_line)
        .saturating_add(1);
    // Suppress substantially overlapping source, not distinct functions or
    // separate portions of a long function that happen to share a file.
    overlap >= shorter.div_ceil(2)
}

fn fuse_candidates(mut candidates: Vec<Candidate<'_>>) -> Vec<Chunk> {
    // Each route gets one vote per source, regardless of score scale. Keep body
    // evidence in the symbol-aware route: names alone lose contextual matches.
    for use_symbols in [false, true] {
        let score = |candidate: &Candidate<'_>| {
            if use_symbols {
                candidate.symbol_aware
            } else {
                candidate.baseline
            }
        };
        let mut order: Vec<_> = (0..candidates.len())
            .filter(|&index| score(&candidates[index]) > 0.0)
            .collect();
        let compare = |&a: &usize, &b: &usize| {
            score(&candidates[b])
                .total_cmp(&score(&candidates[a]))
                .then_with(|| compare_sources(candidates[a].chunk, candidates[b].chunk))
        };
        if order.len() > RETRIEVAL_WINDOW {
            order.select_nth_unstable_by(RETRIEVAL_WINDOW, compare);
            order.truncate(RETRIEVAL_WINDOW);
        }
        order.sort_unstable_by(compare);
        for (rank, index) in order.into_iter().enumerate() {
            candidates[index].fusion += 1.0 / (RRF_CONSTANT + (rank + 1) as f64);
        }
    }
    candidates.retain(|candidate| candidate.fusion > 0.0);
    candidates.sort_unstable_by(|a, b| {
        b.fusion
            .total_cmp(&a.fusion)
            .then_with(|| compare_sources(a.chunk, b.chunk))
    });
    let mut ranked = Vec::with_capacity(SHORTLIST_LIMIT);
    for candidate in candidates {
        if ranked
            .iter()
            .any(|previous| redundant_source(candidate.chunk, previous))
        {
            continue;
        }
        let mut selected = candidate.chunk.clone();
        // Preserve the raw BM25 score used by CLI output and deep search.
        // Fusion chooses shortlist order; its rank-dependent score is not a
        // replacement for relevance values compared across filtered searches.
        selected.lexical_score = candidate.symbol_aware;
        ranked.push(selected);
        if ranked.len() == SHORTLIST_LIMIT {
            break;
        }
    }
    ranked
}
#[derive(Clone, Default)]
struct Field {
    counts: HashMap<String, usize>,
    length: usize,
}
#[derive(Default)]
struct FieldStats {
    documents: usize,
    total_length: usize,
    frequencies: HashMap<String, usize>,
}
impl FieldStats {
    fn add(&mut self, field: &Field) {
        self.documents += 1;
        self.total_length += field.length;
        for term in field.counts.keys() {
            *self.frequencies.entry(term.clone()).or_default() += 1;
        }
    }
    fn score(&self, field: &Field, terms: &[String]) -> f64 {
        if self.total_length == 0 || field.length == 0 {
            return 0.0;
        }
        // Positive IDF and standard defaults; use exact token lengths.
        // https://lucene.apache.org/core/9_6_0/core/org/apache/lucene/search/similarities/BM25Similarity.html
        const K1: f64 = 1.2;
        const B: f64 = 0.75;
        let average = self.total_length as f64 / self.documents as f64;
        let norm = K1 * (1.0 - B + B * field.length as f64 / average);
        terms
            .iter()
            .map(|term| {
                let tf = *field.counts.get(term).unwrap_or(&0) as f64;
                if tf == 0.0 {
                    return 0.0;
                }
                let df = self.frequencies[term] as f64;
                let idf = (1.0 + (self.documents as f64 - df + 0.5) / (df + 0.5)).ln();
                idf * (tf * (K1 + 1.0) / (tf + norm))
            })
            .sum()
    }
}
impl<'a> PreparedCorpus<'a> {
    pub(crate) fn new(chunks: &'a [Chunk]) -> Self {
        let mut stems = HashMap::new();
        let mut words = |text: &str| {
            let mut field = Field::default();
            for token in tokenize(text) {
                *field
                    .counts
                    .entry(normalize(token, &mut stems))
                    .or_default() += 1;
                field.length += 1;
            }
            field
        };
        let mut content_stats = FieldStats::default();
        let mut path_stats = FieldStats::default();
        let mut symbol_stats = FieldStats::default();
        let mut path_fields = HashMap::new();
        let mut seen = HashSet::new();
        let mut prepared_chunks: Vec<PreparedChunk<'_>> = chunks
            .iter()
            // An identical source submitted twice must neither change corpus
            // statistics nor consume a retrieval slot or earn a second vote.
            .filter(|chunk| {
                seen.insert((
                    chunk.path.as_str(),
                    chunk.start_line,
                    chunk.end_line,
                    chunk.text.as_str(),
                ))
            })
            .map(|chunk| {
                let content = words(&chunk.text);
                let path = path_fields
                    .entry(chunk.path.as_str())
                    .or_insert_with(|| words(&chunk.path))
                    .clone();
                let symbol_names: Vec<&str> = if patterns().symbol_extension.is_match(&chunk.path) {
                    let pattern = if patterns().typed_extension.is_match(&chunk.path) {
                        &patterns().typed_symbol
                    } else {
                        &patterns().symbol
                    };
                    pattern
                        .captures_iter(&chunk.text)
                        .flat_map(|captures| {
                            captures
                                .iter()
                                .skip(1)
                                .flatten()
                                .map(|name| name.as_str())
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let symbols = symbol_names
                    .into_iter()
                    .map(|name| {
                        let field = words(name);
                        symbol_stats.add(&field);
                        Symbol {
                            name,
                            field,
                            reference_weight: 1.0,
                        }
                    })
                    .collect();
                content_stats.add(&content);
                path_stats.add(&path);
                PreparedChunk {
                    chunk,
                    content,
                    path,
                    symbols,
                }
            })
            .collect();
        // Cross-file references distinguish reusable entry points from isolated
        // declarations. Count each file once, irrespective of overlapping chunks.
        let mut references: HashMap<&str, HashSet<&str>> = prepared_chunks
            .iter()
            .flat_map(|chunk| chunk.symbols.iter().map(|symbol| symbol.name))
            .map(|name| (name, HashSet::new()))
            .collect();
        for chunk in chunks {
            if !patterns().symbol_extension.is_match(&chunk.path) {
                continue;
            }
            for identifier in chunk
                .text
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            {
                if let Some(files) = references.get_mut(identifier) {
                    files.insert(&chunk.path);
                }
            }
        }
        for prepared in &mut prepared_chunks {
            for symbol in &mut prepared.symbols {
                let other_files = references[symbol.name]
                    .iter()
                    .filter(|path| **path != prepared.chunk.path)
                    .count();
                // Bounded lexical hint, not a resolved call graph.
                symbol.reference_weight = 1.0 + (other_files as f64).ln_1p().min(2.0);
            }
        }
        Self {
            chunks: prepared_chunks,
            content_stats,
            path_stats,
            symbol_stats,
        }
    }

    pub(crate) fn rank(&self, question: &str, include: impl Fn(&Chunk) -> bool) -> Vec<Chunk> {
        let mut terms: Vec<String> = tokenize(question)
            .into_iter()
            .filter(|s| !STOP_WORDS.contains(&s.as_str()))
            .map(|s| stemmer(&s))
            .collect();
        // Stable floating-point accumulation regardless of HashMap random seeds.
        terms.sort();
        terms.dedup();
        if terms.is_empty() {
            return vec![];
        }
        // Rank borrowed candidates first; clone only the final shortlist.
        let mut candidates = Vec::new();
        for prepared in &self.chunks {
            if !include(prepared.chunk) {
                continue;
            }
            let baseline = self.content_stats.score(&prepared.content, &terms)
                + 0.3 * self.path_stats.score(&prepared.path, &terms);
            let score = baseline
                + prepared
                    .symbols
                    .iter()
                    .map(|symbol| {
                        symbol.reference_weight * self.symbol_stats.score(&symbol.field, &terms)
                    })
                    .fold(0.0, f64::max);
            if score > 0.0 {
                candidates.push(Candidate {
                    chunk: prepared.chunk,
                    baseline,
                    symbol_aware: score,
                    fusion: 0.0,
                });
            }
        }
        fuse_candidates(candidates)
    }
}

/// Fuse bounded body/path and symbol-aware BM25 rankings, then suppress
/// redundant source ranges. Returned scores remain raw relevance values;
/// shortlist order is determined by rank fusion, not by sorting those scores.
pub fn rank_lexically(chunks: &[Chunk], question: &str) -> Vec<Chunk> {
    PreparedCorpus::new(chunks).rank(question, |_| true)
}
fn js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000b}' | '\u{000c}' | '\r' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
pub fn search_workspace(cwd: &Path, question: &str) -> Result<Vec<Chunk>> {
    Ok(rank_lexically(&workspace_chunks(cwd)?, question))
}

/// Read one snapshot for a multi-step investigation; no repeated filesystem scans.
pub fn workspace_chunks(cwd: &Path) -> Result<Vec<Chunk>> {
    let root = cwd
        .canonicalize()
        .context("Cannot resolve workspace root")?;
    let output = Command::new(std::env::var_os("OKO_RIPGREP").unwrap_or_else(|| "rg".into()))
        .args(["--no-config", "--files", "--null"])
        .current_dir(cwd)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("Could not run `rg --files`; install ripgrep and try again.")
            } else {
                anyhow::Error::new(e)
            }
        })?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "File discovery failed: {}",
            if stderr.trim().is_empty() {
                format!(
                    "rg exited with code {}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "null".into(), |c| c.to_string())
                )
            } else {
                stderr.trim().to_owned()
            }
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<String> = stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| s.replace('\\', "/"))
        .collect();
    files.sort_by(|a, b| compare_text(a, b));
    files.dedup();
    let mut chunks = vec![];
    for file in files {
        let read = || -> Result<Option<String>> {
            let path = cwd.join(&file).canonicalize()?;
            if !path.starts_with(&root) {
                return Ok(None);
            }
            if fs::metadata(&path)?.len() > MAX_FILE_BYTES as u64 {
                return Ok(None);
            }
            let bytes = fs::read(path).context("reading search file")?;
            if bytes.len() > MAX_FILE_BYTES || bytes.contains(&0) {
                return Ok(None);
            }
            // TextDecoder strips one leading UTF-8 BOM by default.
            let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
            Ok(std::str::from_utf8(bytes).ok().map(str::to_owned))
        };
        if let Ok(Some(text)) = read()
            && !text.trim_matches(js_whitespace).is_empty()
        {
            chunks.extend(chunk_text(&file, &text));
        }
    }
    Ok(chunks)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_names_and_word_variants() {
        assert_eq!(
            tokenize("HTTPServer parseRaceboxCSV save_project_v2"),
            [
                "http", "server", "parse", "racebox", "csv", "save", "project", "v2"
            ]
        );
        let chunks = chunk_text("project.rs", "fn atomic_rename() {}\n");
        assert_eq!(
            rank_lexically(&chunks, "where is renaming done atomically"),
            rank_lexically(&chunks, "rename atomic")
        );
    }
    #[test]
    fn function_boundaries_and_overlap() {
        let text = format!(
            "header\r\n// doc\r\npub(crate) async fn parse() {{\r\n{}",
            vec!["body"; 125].join("\r")
        );
        let c = chunk_text("x.rs", &text);
        assert_eq!(
            c.iter()
                .map(|c| (c.start_line, c.end_line))
                .collect::<Vec<_>>(),
            [(1, 1), (2, 121), (117, 128)]
        );
        assert!(chunk_text("x.rs", "").is_empty());
    }
    #[test]
    fn deterministic_utf16_ties_and_limit() {
        assert_eq!(compare_text("\u{10000}", "\u{e000}"), Ordering::Less);
        let chunks: Vec<_> = (0..40)
            .rev()
            .flat_map(|i| chunk_text(&format!("{i:02}.txt"), "needle"))
            .collect();
        let ranked = rank_lexically(&chunks, "needle");
        assert_eq!(ranked.len(), 30);
        assert_eq!(ranked[0].path, "00.txt");
        assert_eq!(ranked[29].path, "29.txt");
        assert!(rank_lexically(&chunks, "where is it").is_empty());
    }
    #[test]
    fn workspace_ignores_and_decoding_limits() {
        let temp = std::env::temp_dir().join(format!(
            "oko-rust-search-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(temp.join(".git")).unwrap();
        fs::write(temp.join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(temp.join("good.ts"), "const needle = true;\n").unwrap();
        fs::write(temp.join("bom.txt"), b"\xef\xbb\xbfneedle").unwrap();
        fs::write(temp.join("ignored.ts"), "needle").unwrap();
        fs::write(temp.join(".hidden"), "needle").unwrap();
        fs::write(temp.join("binary.dat"), b"needle\0").unwrap();
        fs::write(temp.join("invalid.dat"), b"needle\xc3\x28").unwrap();
        fs::write(temp.join("large.txt"), vec![b'n'; MAX_FILE_BYTES + 1]).unwrap();
        let result = search_workspace(&temp, "needle");
        fs::remove_dir_all(&temp).unwrap();
        let result = result.unwrap();
        assert_eq!(
            result.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            ["bom.txt", "good.ts"]
        );
        assert_eq!(result[0].text, "needle");
    }
}

#[cfg(test)]
#[path = "bm25_tests.rs"]
mod bm25_tests;
