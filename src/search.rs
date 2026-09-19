use crate::{ranking::RankingIntent, stemmer::stemmer};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::Path,
    process::Command,
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
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
// Implementation searches protect half the bounded reranking request for
// source matches. The other half remains available to the broad ranking so
// prose and unsupported source formats can still supply useful evidence.
const IMPLEMENTATION_SOURCE_SLOTS: usize = SHORTLIST_LIMIT / 2;
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
pub(crate) struct PreparedCorpus {
    source_chunks: Arc<[Chunk]>,
    chunks: Vec<PreparedChunk>,
    content_stats: FieldStats,
    path_stats: FieldStats,
    symbol_stats: FieldStats,
}
struct PreparedChunk {
    source_index: usize,
    content: Arc<Field>,
    path: Arc<Field>,
    symbols: Vec<Symbol>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Symbol {
    name: String,
    field: Arc<Field>,
    #[serde(skip)]
    reference_weight: f64,
}

/// Query-independent file features. Raw source text is deliberately not persisted.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PreparedFile {
    path: String,
    path_field: Arc<Field>,
    chunks: Vec<PreparedChunkFeatures>,
    // All identifiers, including names with no declaration in the current corpus:
    // a new declaration can change reference weights in otherwise unchanged files.
    identifiers: HashSet<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct PreparedChunkFeatures {
    start_line: usize,
    end_line: usize,
    source_digest: [u8; 32],
    content: Arc<Field>,
    symbols: Vec<Symbol>,
}

impl PreparedFile {
    /// Reattach cached boundaries to freshly captured source. File hashes are
    /// checked by the caller; chunk hashes and complete range coverage defend
    /// against malformed records without repeating declaration discovery.
    pub(crate) fn restore_chunks(&self, path: &str, text: &str) -> Option<Vec<Chunk>> {
        let mut lines: Vec<&str> = patterns().lines.split(text).collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        if lines.is_empty() {
            return (self.chunks.is_empty() && self.matches(&[])).then(Vec::new);
        }
        if self.path != path || self.chunks.is_empty() {
            return None;
        }
        let mut previous_end: usize = 0;
        let mut chunks = Vec::with_capacity(self.chunks.len());
        for cached in &self.chunks {
            let start = cached.start_line.checked_sub(1)?;
            if cached.end_line <= start
                || cached.end_line > lines.len()
                || cached.end_line - start > FUNCTION_CHUNK_LINES
                || cached.end_line <= previous_end
                || (chunks.is_empty() && start != 0)
                || (!chunks.is_empty()
                    && start != previous_end
                    && previous_end.checked_sub(CHUNK_OVERLAP) != Some(start))
            {
                return None;
            }
            chunks.push(Chunk {
                path: path.to_owned(),
                start_line: cached.start_line,
                end_line: cached.end_line,
                text: lines[start..cached.end_line].join("\n"),
                lexical_score: 0.0,
            });
            previous_end = cached.end_line;
        }
        (previous_end == lines.len() && self.matches(&chunks)).then_some(chunks)
    }

    /// Reject incompatible/corrupt records without repeating tokenization or stemming.
    pub(crate) fn matches(&self, chunks: &[Chunk]) -> bool {
        self.chunks.len() == chunks.len()
            && self.path_field.valid_for(self.path.len())
            && self.identifiers.iter().all(|name| {
                !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            })
            && self.chunks.iter().zip(chunks).all(|(prepared, chunk)| {
                chunk.path == self.path
                    && prepared.start_line == chunk.start_line
                    && prepared.end_line == chunk.end_line
                    && prepared.source_digest
                        == <[u8; 32]>::from(Sha256::digest(chunk.text.as_bytes()))
                    && prepared.content.valid_for(chunk.text.len())
                    && prepared.symbols.len() <= chunk.text.len()
                    && prepared.symbols.iter().all(|symbol| {
                        !symbol.name.is_empty()
                            && chunk.text.contains(&symbol.name)
                            && symbol.field.valid_for(symbol.name.len())
                    })
            })
    }
}

fn words(text: &str, stems: &mut HashMap<String, String>) -> Field {
    let mut field = Field::default();
    for token in tokenize(text) {
        *field.counts.entry(normalize(token, stems)).or_default() += 1;
        field.length += 1;
    }
    field
}

/// Shares stemming work across changed files during one workspace refresh.
#[derive(Default)]
pub(crate) struct FilePreparer {
    stems: HashMap<String, String>,
}
impl FilePreparer {
    pub(crate) fn prepare_file(&mut self, chunks: &[Chunk]) -> PreparedFile {
        prepare_file_with_stems(chunks, &mut self.stems)
    }
}

#[cfg(test)]
fn prepare_file(chunks: &[Chunk]) -> PreparedFile {
    FilePreparer::default().prepare_file(chunks)
}

fn prepare_file_with_stems(chunks: &[Chunk], stems: &mut HashMap<String, String>) -> PreparedFile {
    let path = chunks.first().map_or("", |chunk| chunk.path.as_str());
    debug_assert!(chunks.iter().all(|chunk| chunk.path == path));
    let supported = patterns().symbol_extension.is_match(path);
    let pattern = if patterns().typed_extension.is_match(path) {
        &patterns().typed_symbol
    } else {
        &patterns().symbol
    };
    let mut identifiers = HashSet::new();
    let prepared = chunks
        .iter()
        .map(|chunk| {
            let symbols = if supported {
                identifiers.extend(
                    chunk
                        .text
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned),
                );
                pattern
                    .captures_iter(&chunk.text)
                    .flat_map(|captures| {
                        captures
                            .iter()
                            .skip(1)
                            .flatten()
                            .map(|name| name.as_str().to_owned())
                            .collect::<Vec<_>>()
                    })
                    .map(|name| Symbol {
                        field: Arc::new(words(&name, stems)),
                        name,
                        reference_weight: 1.0,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            PreparedChunkFeatures {
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                source_digest: Sha256::digest(chunk.text.as_bytes()).into(),
                content: Arc::new(words(&chunk.text, stems)),
                symbols,
            }
        })
        .collect();
    PreparedFile {
        path: path.to_owned(),
        path_field: Arc::new(words(path, stems)),
        chunks: prepared,
        identifiers,
    }
}
#[derive(Clone, Copy)]
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
#[derive(Clone, Default, Serialize, Deserialize)]
struct Field {
    counts: HashMap<String, usize>,
    length: usize,
}
impl Field {
    fn valid_for(&self, source_bytes: usize) -> bool {
        self.length <= source_bytes
            && self
                .counts
                .values()
                .all(|&count| count > 0 && count <= self.length)
            && self
                .counts
                .values()
                .try_fold(0usize, |sum, &count| sum.checked_add(count))
                == Some(self.length)
            && self.counts.keys().all(|term| {
                !term.is_empty()
                    && term.len() <= source_bytes
                    && term
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            })
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct FieldStats {
    documents: usize,
    total_length: usize,
    frequencies: HashMap<String, usize>,
}

/// Only corpus-wide data is saved here; token fields remain in PreparedFile.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PreparedStatistics {
    content: FieldStats,
    path: FieldStats,
    symbol: FieldStats,
    reference_weights: Vec<Vec<f64>>,
}

impl FieldStats {
    fn valid_for<'a>(&self, fields: impl Iterator<Item = &'a Field>) -> bool {
        let mut documents = 0usize;
        let mut total_length = 0usize;
        for field in fields {
            documents += 1;
            let Some(length) = total_length.checked_add(field.length) else {
                return false;
            };
            total_length = length;
            if field
                .counts
                .keys()
                .any(|term| !self.frequencies.contains_key(term))
            {
                return false;
            }
        }
        self.documents == documents
            && self.total_length == total_length
            && self
                .frequencies
                .values()
                .all(|&count| count > 0 && count <= documents)
            && self.frequencies.keys().all(|term| {
                !term.is_empty()
                    && term
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            })
    }

    fn add(&mut self, field: &Field) {
        self.documents += 1;
        self.total_length += field.length;
        for term in field.counts.keys() {
            if let Some(frequency) = self.frequencies.get_mut(term) {
                *frequency += 1;
            } else {
                self.frequencies.insert(term.clone(), 1);
            }
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
impl PreparedCorpus {
    pub(crate) fn statistics(&self) -> PreparedStatistics {
        PreparedStatistics {
            content: self.content_stats.clone(),
            path: self.path_stats.clone(),
            symbol: self.symbol_stats.clone(),
            reference_weights: self
                .chunks
                .iter()
                .map(|chunk| {
                    chunk
                        .symbols
                        .iter()
                        .map(|symbol| symbol.reference_weight)
                        .collect()
                })
                .collect(),
        }
    }

    /// An unchanged workspace can reuse its corpus statistics. This path is
    /// intentionally restricted to ordered, distinct source chunks from the
    /// workspace cache; the general constructor still accepts arbitrary input.
    pub(crate) fn from_cached_statistics<'a>(
        source_chunks: Arc<[Chunk]>,
        files: impl IntoIterator<Item = &'a PreparedFile>,
        statistics: &PreparedStatistics,
    ) -> Result<Self> {
        let mut chunks = Vec::with_capacity(source_chunks.len());
        for file in files {
            for features in &file.chunks {
                let index = chunks.len();
                let source = source_chunks
                    .get(index)
                    .context("Missing cached source chunk")?;
                if source.path != file.path
                    || source.start_line != features.start_line
                    || source.end_line != features.end_line
                {
                    bail!("Cached corpus boundaries mismatch");
                }
                let weights = statistics
                    .reference_weights
                    .get(index)
                    .context("Missing cached reference weights")?;
                if weights.len() != features.symbols.len()
                    || weights
                        .iter()
                        .any(|weight| !weight.is_finite() || !(1.0..=3.0).contains(weight))
                {
                    bail!("Invalid cached reference weights");
                }
                let mut symbols = features.symbols.clone();
                for (symbol, weight) in symbols.iter_mut().zip(weights) {
                    symbol.reference_weight = *weight;
                }
                chunks.push(PreparedChunk {
                    source_index: index,
                    content: Arc::clone(&features.content),
                    path: Arc::clone(&file.path_field),
                    symbols,
                });
            }
        }
        if chunks.len() != source_chunks.len()
            || statistics.reference_weights.len() != chunks.len()
            || !statistics
                .content
                .valid_for(chunks.iter().map(|c| c.content.as_ref()))
            || !statistics
                .path
                .valid_for(chunks.iter().map(|c| c.path.as_ref()))
            || !statistics.symbol.valid_for(
                chunks
                    .iter()
                    .flat_map(|c| c.symbols.iter().map(|s| s.field.as_ref())),
            )
        {
            bail!("Invalid cached corpus statistics");
        }
        Ok(Self {
            source_chunks,
            chunks,
            content_stats: statistics.content.clone(),
            path_stats: statistics.path.clone(),
            symbol_stats: statistics.symbol.clone(),
        })
    }

    pub(crate) fn new(chunks: &[Chunk]) -> Self {
        let mut by_path: HashMap<&str, Vec<Chunk>> = HashMap::new();
        let mut paths = Vec::new();
        for chunk in chunks {
            if !by_path.contains_key(chunk.path.as_str()) {
                paths.push(chunk.path.as_str());
            }
            by_path.entry(&chunk.path).or_default().push(chunk.clone());
        }
        let mut preparer = FilePreparer::default();
        let files: Vec<_> = paths
            .iter()
            .map(|path| preparer.prepare_file(&by_path[path]))
            .collect();
        Self::from_files(chunks, &files).expect("freshly prepared files match their source")
    }

    pub(crate) fn from_files<'a>(
        chunks: &[Chunk],
        files: impl IntoIterator<Item = &'a PreparedFile>,
    ) -> Result<Self> {
        Self::from_shared_files(Arc::from(chunks), files)
    }

    /// Share captured source with context/preview consumers without duplicating
    /// every source string. Prepared chunks refer only to stable array indexes.
    pub(crate) fn from_shared_files<'a>(
        chunks: Arc<[Chunk]>,
        files: impl IntoIterator<Item = &'a PreparedFile>,
    ) -> Result<Self> {
        let mut prepared_files = HashMap::new();
        for file in files {
            if prepared_files.insert(file.path.as_str(), file).is_some() {
                bail!("Duplicate prepared file");
            }
        }
        let mut offsets: HashMap<&str, usize> = HashMap::new();
        let mut seen = HashSet::new();
        let mut content_stats = FieldStats::default();
        let mut path_stats = FieldStats::default();
        let mut symbol_stats = FieldStats::default();
        let mut prepared_chunks = Vec::new();
        for (source_index, chunk) in chunks.iter().enumerate() {
            let file = prepared_files
                .get(chunk.path.as_str())
                .context("Missing prepared file")?;
            let offset = offsets.entry(&chunk.path).or_default();
            let features = file.chunks.get(*offset).context("Missing prepared chunk")?;
            *offset += 1;
            if features.start_line != chunk.start_line || features.end_line != chunk.end_line {
                bail!("Prepared chunk boundaries do not match source");
            }
            // Preserve input order and identical-source deduplication from the
            // uncached corpus, including callers that provide interleaved files.
            if !seen.insert((
                chunk.path.as_str(),
                chunk.start_line,
                chunk.end_line,
                chunk.text.as_str(),
            )) {
                continue;
            }
            content_stats.add(&features.content);
            path_stats.add(&file.path_field);
            for symbol in &features.symbols {
                symbol_stats.add(&symbol.field);
            }
            prepared_chunks.push(PreparedChunk {
                source_index,
                content: features.content.clone(),
                path: file.path_field.clone(),
                symbols: features.symbols.clone(),
            });
        }
        if prepared_files
            .iter()
            .any(|(path, file)| offsets.get(path).copied().unwrap_or(0) != file.chunks.len())
        {
            bail!("Prepared file chunk count does not match source");
        }
        // Rebuild corpus-wide statistics when any file changes. Each file gets
        // at most one reference vote, regardless of overlapping/repeated chunks.
        let mut references: HashMap<&str, usize> = prepared_files
            .values()
            .flat_map(|file| file.chunks.iter())
            .flat_map(|chunk| chunk.symbols.iter().map(|symbol| symbol.name.as_str()))
            .map(|name| (name, 0))
            .collect();
        for file in prepared_files.values() {
            for identifier in &file.identifiers {
                if let Some(files) = references.get_mut(identifier.as_str()) {
                    // identifiers is already distinct within this file, and
                    // prepared_files has exactly one record per source path.
                    *files += 1;
                }
            }
        }
        for prepared in &mut prepared_chunks {
            for symbol in &mut prepared.symbols {
                let own_file = prepared_files[chunks[prepared.source_index].path.as_str()];
                let own_reference = usize::from(own_file.identifiers.contains(&symbol.name));
                let other_files = references[symbol.name.as_str()] - own_reference;
                symbol.reference_weight = 1.0 + (other_files as f64).ln_1p().min(2.0);
            }
        }
        Ok(Self {
            source_chunks: chunks,
            chunks: prepared_chunks,
            content_stats,
            path_stats,
            symbol_stats,
        })
    }

    pub(crate) fn rank(&self, question: &str, include: impl Fn(&Chunk) -> bool) -> Vec<Chunk> {
        self.rank_with_intent(question, include, RankingIntent::General)
    }

    pub(crate) fn rank_with_intent(
        &self,
        question: &str,
        include: impl Fn(&Chunk) -> bool,
        intent: RankingIntent,
    ) -> Vec<Chunk> {
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
            let chunk = &self.source_chunks[prepared.source_index];
            if !include(chunk) {
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
                    chunk,
                    baseline,
                    symbol_aware: score,
                    fusion: 0.0,
                });
            }
        }
        if !matches!(intent, RankingIntent::Implementation) {
            return fuse_candidates(candidates);
        }
        // Score against the same corpus-wide statistics in both lanes. File
        // extensions are only candidate hints: tests, callers and comments
        // still need the reranker's implementation judgment.
        let source = fuse_candidates(
            candidates
                .iter()
                .filter(|candidate| patterns().symbol_extension.is_match(&candidate.chunk.path))
                .copied()
                .collect(),
        );
        let broad = fuse_candidates(candidates);
        let mut selected: Vec<_> = source
            .into_iter()
            .take(IMPLEMENTATION_SOURCE_SLOTS)
            .collect();
        for candidate in broad {
            if !selected
                .iter()
                .any(|previous| redundant_source(&candidate, previous))
            {
                selected.push(candidate);
            }
            if selected.len() == SHORTLIST_LIMIT {
                break;
            }
        }
        selected
    }
}

/// Fuse bounded body/path and symbol-aware BM25 rankings, then suppress
/// redundant source ranges. Returned scores remain raw relevance values;
/// shortlist order is determined by rank fusion, not by sorting those scores.
pub fn rank_lexically(chunks: &[Chunk], question: &str) -> Vec<Chunk> {
    PreparedCorpus::new(chunks).rank(question, |_| true)
}
/// Bound implementation retrieval without letting keyword-rich prose occupy
/// every reranking slot. General and explanation searches retain broad ranking.
pub fn rank_lexically_with_intent(
    chunks: &[Chunk],
    question: &str,
    intent: RankingIntent,
) -> Vec<Chunk> {
    PreparedCorpus::new(chunks).rank_with_intent(question, |_| true, intent)
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
    Ok(workspace_files(cwd)?
        .into_iter()
        .flat_map(|file| chunk_text(&file.path, &file.text))
        .collect())
}

#[derive(Clone)]
pub(crate) struct WorkspaceFile {
    pub path: String,
    pub text: Arc<str>,
    /// Digest includes the original bytes, including any leading BOM.
    pub digest: String,
    /// Digest of the captured, BOM-normalized text, reused by syntax facts.
    pub text_digest: [u8; 32],
    pub stamp: Option<SourceStamp>,
}

pub(crate) fn workspace_files(cwd: &Path) -> Result<Vec<WorkspaceFile>> {
    let root = cwd
        .canonicalize()
        .context("Cannot resolve workspace root")?;
    let files = workspace_paths(cwd)?;
    let workers = if files.len() < 64 {
        1
    } else {
        thread::available_parallelism().map_or(1, |count| count.get().min(4))
    };
    read_workspace_paths(&root, &files, workers)
}

/// Discovery remains authoritative even when native filesystem events lag.
pub(crate) fn workspace_paths(cwd: &Path) -> Result<Vec<String>> {
    // Release archives keep rg beside oko, including when neither is on PATH.
    // An explicit override remains authoritative for embedders and MCP setup.
    let rg = std::env::var_os("OKO_RIPGREP").unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|dir| dir.join(if cfg!(windows) { "rg.exe" } else { "rg" }))
            })
            .filter(|path| path.is_file())
            .map_or_else(|| "rg".into(), |path| path.into_os_string())
    });
    let output = Command::new(rg)
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
    Ok(files)
}

/// Metadata reuse is enabled only for Unix stamps with subsecond change time.
/// Other platforms and coarse timestamps continue reading contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceStamp {
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl SourceStamp {
    pub(crate) fn device(&self) -> u64 {
        self.device
    }
    fn from_metadata(metadata: &fs::Metadata) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
                mode: metadata.mode(),
                size: metadata.len(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            None
        }
    }

    pub(crate) fn can_reuse(&self, now: SystemTime) -> bool {
        // Like Git's racy-index protection, recently changed timestamps need
        // a content check. A two-second window also covers coarse clocks.
        let Some(cutoff) = now.checked_sub(Duration::from_secs(2)) else {
            return false;
        };
        self.changed.1 > 0
            && [self.modified, self.changed]
                .into_iter()
                .all(|(seconds, nanos)| {
                    let Ok(seconds) = u64::try_from(seconds) else {
                        return false;
                    };
                    let Ok(nanos) = u32::try_from(nanos) else {
                        return false;
                    };
                    nanos < 1_000_000_000
                        && UNIX_EPOCH
                            .checked_add(Duration::new(seconds, nanos))
                            .is_some_and(|time| time < cutoff)
                })
    }
}

pub(crate) fn workspace_file_stamp(root: &Path, file: &str) -> Result<Option<SourceStamp>> {
    let path = root.join(file).canonicalize()?;
    if !path.starts_with(root) {
        return Ok(None);
    }
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
        return Ok(None);
    }
    Ok(SourceStamp::from_metadata(&metadata))
}

fn read_workspace_file(
    root: &Path,
    file: &str,
    capture_stamp: bool,
) -> Result<Option<WorkspaceFile>> {
    let capture_started = SystemTime::now();
    let path = root.join(file).canonicalize()?;
    if !path.starts_with(root) {
        return Ok(None);
    }
    let handle = fs::File::open(path).context("opening search file")?;
    let metadata = handle.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
        return Ok(None);
    }
    // A file growing after the metadata check cannot cause an unbounded read.
    let mut bytes = Vec::with_capacity(metadata.len() as usize + 1);
    (&handle)
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("reading search file")?;
    let stamp = if capture_stamp {
        let before = SourceStamp::from_metadata(&metadata);
        let after = handle
            .metadata()
            .ok()
            .as_ref()
            .and_then(SourceStamp::from_metadata);
        // Never associate old captured bytes with a newer post-write stamp.
        before.filter(|before| {
            after.as_ref() == Some(before)
                && before.can_reuse(capture_started)
                && workspace_file_stamp(root, file).ok().flatten().as_ref() == Some(before)
        })
    } else {
        None
    };
    if bytes.len() > MAX_FILE_BYTES || bytes.contains(&0) {
        return Ok(None);
    }
    let raw_digest = Sha256::digest(&bytes);
    let digest = format!("{raw_digest:x}");
    // TextDecoder strips one leading UTF-8 BOM by default.
    let (bytes, text_digest) = match bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        Some(text) => (text, Sha256::digest(text).into()),
        None => (bytes.as_slice(), raw_digest.into()),
    };
    Ok(std::str::from_utf8(bytes).ok().and_then(|text| {
        (!text.trim_matches(js_whitespace).is_empty()).then(|| WorkspaceFile {
            path: file.to_owned(),
            text: Arc::from(text),
            digest,
            text_digest,
            stamp,
        })
    }))
}

pub(crate) fn read_workspace_paths(
    root: &Path,
    files: &[String],
    workers: usize,
) -> Result<Vec<WorkspaceFile>> {
    read_workspace_paths_inner(root, files, workers, false)
}

pub(crate) fn read_workspace_paths_watched(
    root: &Path,
    files: &[String],
    workers: usize,
) -> Result<Vec<WorkspaceFile>> {
    read_workspace_paths_inner(root, files, workers, true)
}

fn read_workspace_paths_inner(
    root: &Path,
    files: &[String],
    workers: usize,
    capture_stamp: bool,
) -> Result<Vec<WorkspaceFile>> {
    let read_batch = |batch: &[String]| {
        batch
            .iter()
            .filter_map(|file| {
                read_workspace_file(root, file, capture_stamp)
                    .ok()
                    .flatten()
            })
            .collect::<Vec<_>>()
    };
    if workers <= 1 || files.is_empty() {
        return Ok(read_batch(files));
    }
    thread::scope(|scope| {
        let batches: Vec<_> = files
            .chunks(files.len().div_ceil(workers.min(4)))
            .map(|batch| {
                match thread::Builder::new().spawn_scoped(scope, move || read_batch(batch)) {
                    Ok(handle) => Ok(handle),
                    // Resource-constrained hosts can keep using serial reads.
                    Err(_) => Err(read_batch(batch)),
                }
            })
            .collect();
        let mut accepted = Vec::new();
        let mut failure = None;
        // Joining in batch order preserves the deterministic discovery order.
        for batch in batches {
            let result = match batch {
                Ok(handle) => handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("Search file reader failed")),
                Err(files) => Ok(files),
            };
            match result {
                Ok(files) => accepted.extend(files),
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(accepted),
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_file_reads_preserve_order_bytes_and_eligibility() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut files = Vec::new();
        for index in 0..96 {
            let path = format!("{index:03}.rs");
            fs::write(
                root.join(&path),
                format!("\u{feff}fn archive_{index}() {{}}\r\n// 🦀\r\n"),
            )
            .unwrap();
            files.push(path);
        }
        for (path, bytes) in [
            ("binary", b"a\0b".to_vec()),
            ("invalid", vec![0xff]),
            ("large", vec![b'x'; MAX_FILE_BYTES + 1]),
            ("blank", b" \n\r\t".to_vec()),
        ] {
            fs::write(root.join(path), bytes).unwrap();
            files.push(path.into());
        }
        files.push("missing".into());
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("external.rs"), "fn external() {}").unwrap();
        files.push(outside.path().join("external.rs").to_str().unwrap().into());
        let serial = read_workspace_paths(&root, &files, 1).unwrap();
        assert_eq!(serial.len(), 96);
        for _ in 0..3 {
            let parallel = read_workspace_paths(&root, &files, 4).unwrap();
            assert_eq!(
                serial
                    .iter()
                    .map(|file| (&file.path, &file.text, &file.digest))
                    .collect::<Vec<_>>(),
                parallel
                    .iter()
                    .map(|file| (&file.path, &file.text, &file.digest))
                    .collect::<Vec<_>>(),
            );
        }
        assert!(serial[0].text.starts_with("fn archive_0"));
        assert!(serial[0].text.ends_with("// 🦀\r\n"));
        assert!(read_workspace_paths(&root, &[], 4).unwrap().is_empty());
    }

    #[test]
    fn persisted_features_preserve_scores_and_source_order() {
        let first = chunk_text("engine.rs", "pub fn parse_ledger() { load_accounts(); }\n");
        let second = chunk_text(
            "client.ts",
            "export function loadAccounts() { parse_ledger(); }\n",
        );
        let docs = chunk_text("guide.md", "The ledger parser loads account records.\n");
        let mut chunks = Vec::new();
        chunks.extend(second.clone());
        chunks.extend(first.clone());
        chunks.extend(docs.clone());
        // Duplicate and interleaved inputs are supported by the public ranker.
        chunks.extend(first.clone());
        let mut repeated_first = first.clone();
        repeated_first.extend(first);
        let files: Vec<PreparedFile> = [&repeated_first[..], &second, &docs]
            .into_iter()
            .map(|file| {
                let prepared = prepare_file(file);
                let encoded = serde_json::to_vec(&prepared).unwrap();
                let decoded: PreparedFile = serde_json::from_slice(&encoded).unwrap();
                assert!(decoded.matches(file));
                decoded
            })
            .collect();
        let restored = PreparedCorpus::from_files(&chunks, &files).unwrap();
        let fresh = PreparedCorpus::new(&chunks);
        for question in [
            "ledger parser",
            "loading accounts",
            "parse_ledger",
            "unmatched",
            "",
        ] {
            assert_eq!(
                restored.rank(question, |_| true),
                fresh.rank(question, |_| true)
            );
            assert_eq!(
                restored.rank(question, |c| c.path.ends_with(".rs")),
                fresh.rank(question, |c| c.path.ends_with(".rs"))
            );
        }
    }

    #[test]
    fn reused_identifiers_notice_new_declarations_and_removed_callers() {
        let callers = chunk_text("client.rs", "fn existing_client() { parse_ledger(); }\n");
        // Prepare before a declaration exists. Caching only names known to the
        // old corpus would miss this reference when the implementation appears.
        let retained = prepare_file(&callers);
        assert!(retained.identifiers.contains("parse_ledger"));
        let implementation = chunk_text("engine.rs", "pub fn parse_ledger() {}\n");
        let new_file = prepare_file(&implementation);
        let mut combined = implementation.clone();
        combined.extend(callers);
        let refreshed = PreparedCorpus::from_files(&combined, [&new_file, &retained]).unwrap();
        assert_eq!(
            refreshed.rank("parse ledger", |_| true),
            rank_lexically(&combined, "parse ledger")
        );
        assert_eq!(
            refreshed.chunks[0].symbols[0].reference_weight,
            1.0 + 2.0_f64.ln()
        );
        let removed = PreparedCorpus::from_files(&implementation, [&new_file]).unwrap();
        assert_eq!(removed.chunks[0].symbols[0].reference_weight, 1.0);
        assert_eq!(
            removed.rank("parse ledger", |_| true),
            rank_lexically(&implementation, "parse ledger")
        );
    }

    #[test]
    fn invalid_prepared_records_are_rejected() {
        let chunks = chunk_text("engine.rs", "fn parse_ledger() {}\n");
        let prepared = prepare_file(&chunks);
        assert!(prepared.matches(&chunks));
        let changed = chunk_text("engine.rs", "fn other_ledger() {}\n");
        assert!(!prepared.matches(&changed));
        let renamed = chunk_text("renamed.rs", "fn parse_ledger() {}\n");
        assert!(!prepared.matches(&renamed));
        let mut corrupt = prepared.clone();
        Arc::make_mut(&mut corrupt.chunks[0].content).length = usize::MAX;
        assert!(!corrupt.matches(&chunks));
        let mut corrupt = prepared.clone();
        Arc::make_mut(&mut corrupt.chunks[0].content)
            .counts
            .insert("parse".into(), usize::MAX);
        assert!(!corrupt.matches(&chunks));
        let mut corrupt = prepared.clone();
        corrupt.chunks[0].symbols[0].name = "not_in_source".into();
        assert!(!corrupt.matches(&chunks));
        assert!(PreparedCorpus::from_files(&chunks, []).is_err());
        assert!(PreparedCorpus::from_files(&chunks, [&prepared, &prepared]).is_err());
        let mut corrupt = prepared;
        corrupt.chunks.clear();
        assert!(PreparedCorpus::from_files(&chunks, [&corrupt]).is_err());
    }

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
