//! Bounded evidence from the same source snapshot used for search.
//!
//! Declaration detection is deliberately lexical. Related definitions require
//! compatible source evidence; unknown qualification or ambiguity is omitted.
mod related;
use crate::navigation::{DefinitionKind, NavigationIndex, Relation};
use crate::search::{Chunk, tokenize};
use regex::Regex;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::OnceLock,
};

pub const PACKET_MAX_BYTES: usize = 16_000;
const RESULT_LIMIT: usize = 3;
const RELATED_LIMIT: usize = 2;
const EXCERPT_LINES: usize = 60;
// Spend more context on a proven primary implementation, not every candidate.
// Uncertain declarations can retain the bounded primary winning chunk without
// claiming a complete function. Larger spans use a focused source window.
const PRIMARY_IMPLEMENTATION_LINES: usize = 256;
const RELATED_LINES: usize = 32;
// Bound lexical signature scanning; uncertain spans fall back to source context.
const SIGNATURE_LINES: usize = 128;
const MIN_TEXT_BYTES: usize = 180;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPacket {
    pub results: Vec<ContextMatch>,
    pub related: Vec<RelatedDefinition>,
    /// Some source context or ranked matches were omitted to keep the packet bounded.
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMatch {
    #[serde(flatten)]
    pub excerpt: SourceExcerpt,
    pub score: f64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedDefinition {
    #[serde(flatten)]
    pub excerpt: SourceExcerpt,
    pub relation: &'static str,
    pub ambiguous: bool,
    /// Number of compatible declaration locations after conservative matching.
    pub candidate_count: usize,
    pub referenced_from: Vec<SourceLocation>,
    /// For a caller, the definition in the primary results that it refers to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SourceLocation>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocation {
    pub path: String,
    pub line: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceExcerpt {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<SymbolHeader>,
    /// The excerpt is incomplete; a single very long line may be cut at a UTF-8 boundary.
    pub truncated: bool,
    /// True only when the complete enclosing definition is proven and included.
    /// This says nothing about whether all surrounding dependencies were returned.
    pub definition_complete: bool,
    #[serde(skip)]
    focus_line: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolHeader {
    pub name: String,
    pub line: usize,
    pub text: String,
    pub detection: &'static str,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
struct Declaration {
    name: String,
    line: usize,
    end: usize,
    complete: bool,
}
struct Snapshot<'a> {
    lines: BTreeMap<usize, &'a str>,
    declarations: Vec<Declaration>,
    code: BTreeMap<usize, String>,
}
#[derive(Clone, Copy, PartialEq)]
enum Language {
    Braces,
    Python,
    Ruby,
    Other,
}
fn language(path: &str) -> Language {
    match path.rsplit('.').next().unwrap_or("") {
        "py" | "pyi" => Language::Python,
        "rb" => Language::Ruby,
        "rs" | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "go" | "java" | "cs" | "c" | "h"
        | "cc" | "cpp" | "hpp" | "php" | "swift" | "kt" => Language::Braces,
        _ => Language::Other,
    }
}
struct Patterns {
    declaration: Regex,
    typed: Regex,
    candidate_declaration: Regex,
    candidate_typed: Regex,
    simple_return_type: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let declaration = Regex::new(r"^[ \t]*(?:(?:pub(?:\([^)]*\))?|async|unsafe|const|export|default|public|private|protected|static|final|override|abstract|internal|open|suspend)\s+)*(?:(?:fn|function\*?|def|fun)\s+([A-Za-z_][A-Za-z0-9_]*)|func\s+(?:\([^)]*\)\s*)?([A-Za-z_][A-Za-z0-9_]*)|(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:async\s+)?(?:\([^)]*\)|[A-Za-z_][A-Za-z0-9_]*)\s*=>)").unwrap();
        let typed = Regex::new(r"^[ \t]*(?:[A-Za-z_][A-Za-z0-9_.<>,?\[\]:*&]*[ \t]+)+([A-Za-z_][A-Za-z0-9_]*)[ \t]*\([^;]*\)[ \t]*(?:\{|throws\b)").unwrap();
        Patterns {
        candidate_declaration: Regex::new(&format!("(?m){}", declaration.as_str())).unwrap(),
        candidate_typed: Regex::new(&format!("(?m){}", typed.as_str())).unwrap(),
        declaration, typed,
        simple_return_type: Regex::new(r"^\s*[A-Za-z_$][A-Za-z0-9_$]*(?:\s*\.\s*[A-Za-z_$][A-Za-z0-9_$]*)*\s*$").unwrap(),
        }
    })
}

/// Remove comments and string contents without changing line or byte positions.
/// Ignoring template interpolation is conservative: it can miss references, but
/// does not pretend text within a string is an actual declaration.
fn code_lines(lines: &BTreeMap<usize, &str>, path: &str) -> BTreeMap<usize, String> {
    let lang = language(path);
    let rust = path.ends_with(".rs");
    let mut output = BTreeMap::new();
    let mut block = 0usize;
    let mut quote: Option<(u8, bool)> = None;
    let mut raw_hashes: Option<usize> = None;
    let mut previous = None;
    for (&number, text) in lines {
        if previous.is_some_and(|n| number != n + 1) {
            block = 0;
            quote = None;
            raw_hashes = None;
        }
        previous = Some(number);
        let bytes = text.as_bytes();
        let mut clean = vec![b' '; bytes.len()];
        let mut i = 0;
        while i < bytes.len() {
            if let Some(hashes) = raw_hashes {
                if bytes[i] == b'"'
                    && bytes
                        .get(i + 1..i + 1 + hashes)
                        .is_some_and(|suffix| suffix.iter().all(|c| *c == b'#'))
                {
                    raw_hashes = None;
                    i += 1 + hashes;
                } else {
                    i += 1;
                }
                continue;
            }
            if let Some((q, triple)) = quote {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == q && (!triple || bytes.get(i..i + 3) == Some(&[q, q, q])) {
                    quote = None;
                    i += if triple { 3 } else { 1 };
                } else {
                    i += 1;
                }
                continue;
            }
            if block > 0 {
                if bytes.get(i..i + 2) == Some(b"/*") {
                    block += 1;
                    i += 2;
                } else if bytes.get(i..i + 2) == Some(b"*/") {
                    block -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if (matches!(lang, Language::Python | Language::Ruby) && bytes[i] == b'#')
                || (lang == Language::Braces && bytes.get(i..i + 2) == Some(b"//"))
            {
                break;
            }
            if lang == Language::Braces && bytes.get(i..i + 2) == Some(b"/*") {
                block = 1;
                i += 2;
                continue;
            }
            if rust && bytes[i] == b'r' {
                let hashes = bytes[i + 1..].iter().take_while(|b| **b == b'#').count();
                if bytes.get(i + 1 + hashes) == Some(&b'"') {
                    raw_hashes = Some(hashes);
                    i += 2 + hashes;
                    continue;
                }
            }
            if matches!(bytes[i], b'"' | b'\'' | b'`') {
                let q = bytes[i];
                // Rust lifetimes ('a, 'static) are not quoted character strings.
                let lifetime = q == b'\''
                    && rust
                    && bytes
                        .get(i + 1)
                        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                    && bytes.get(i + 2) != Some(&b'\'');
                if lifetime {
                    clean[i] = bytes[i];
                    i += 1;
                    continue;
                }
                let triple = lang == Language::Python && bytes.get(i..i + 3) == Some(&[q, q, q]);
                quote = Some((q, triple));
                i += if triple { 3 } else { 1 };
                continue;
            }
            clean[i] = bytes[i];
            i += 1;
        }
        // Ordinary single/double quoted strings cannot span source lines in
        // Python; backtick strings and triple quoted strings can.
        if lang == Language::Python && quote.is_some_and(|(q, t)| q != b'`' && !t) {
            quote = None;
        }
        output.insert(
            number,
            String::from_utf8(clean).expect("whole UTF-8 characters preserved"),
        );
    }
    output
}
fn declarations(path: &str, code: &BTreeMap<usize, String>) -> Vec<Declaration> {
    let lang = language(path);
    if lang == Language::Other {
        return vec![];
    }
    let typed = matches!(
        path.rsplit('.').next(),
        Some("java" | "cs" | "c" | "h" | "cc" | "cpp" | "hpp")
    );
    let mut declarations = Vec::new();
    for (&line, text) in code {
        let captures = patterns()
            .declaration
            .captures(text)
            .or_else(|| typed.then(|| patterns().typed.captures(text)).flatten());
        let Some(name) = captures
            .as_ref()
            .and_then(|c| c.iter().skip(1).flatten().next())
        else {
            continue;
        };
        if name.as_str().len() > 128
            // Qualified/escaped/Unicode identifiers need language-specific
            // parsing. Do not turn their ASCII prefix into a fake symbol.
            || text[name.end()..].chars().next().is_some_and(|c| !c.is_ascii() || matches!(c, '.' | '#' | '$' | '?' | '!'))
            || matches!(name.as_str(), "if" | "while" | "for" | "switch" | "catch")
        {
            continue;
        }
        let (end, complete) = match lang {
            Language::Python => {
                let indent = text.len() - text.trim_start().len();
                let mut end = line;
                let mut nesting = 0usize;
                let mut signature_done = false;
                let mut contiguous = true;
                let mut previous = line;
                for (&n, next) in code.range(line..) {
                    if n > previous + 1 {
                        contiguous = false;
                        break;
                    }
                    previous = n;
                    if !signature_done {
                        if n - line >= SIGNATURE_LINES
                            || (n > line && patterns().declaration.is_match(next))
                        {
                            break;
                        }
                        for byte in next.bytes() {
                            match byte {
                                b'(' | b'[' | b'{' => nesting += 1,
                                b')' | b']' | b'}' => nesting = nesting.saturating_sub(1),
                                b':' if nesting == 0 => {
                                    signature_done = true;
                                    break;
                                }
                                _ => {}
                            }
                        }
                    } else if !next.trim().is_empty()
                        && next.len() - next.trim_start().len() <= indent
                    {
                        break;
                    }
                    end = n;
                }
                if signature_done && contiguous {
                    (end, true)
                } else {
                    (line, false)
                }
            }
            Language::Braces => {
                // Keep an unproven declaration anchored to its header only for
                // ownership. Excerpt expansion uses a bounded source fallback.
                let mut depth = 0usize;
                let mut parens = 0usize;
                let mut brackets = 0usize;
                let mut opened = false;
                let mut parameters_closed = false;
                let mut return_type = false;
                let mut return_annotation = Vec::new();
                let mut end = line;
                let mut complete = false;
                let mut previous = line;
                'body: for (&n, next) in code.range(line..) {
                    if n > previous + 1 || (!opened && n - line >= SIGNATURE_LINES) {
                        break;
                    }
                    previous = n;
                    if return_type && !opened {
                        return_annotation.push(b' ');
                    }
                    if n > line
                        && !opened
                        && (patterns().declaration.is_match(next)
                            || (typed && patterns().typed.is_match(next)))
                    {
                        break;
                    }
                    for byte in next.bytes() {
                        if !opened {
                            if return_type && byte != b'{' {
                                if return_annotation.len() >= 4096 {
                                    break 'body;
                                }
                                return_annotation.push(byte);
                            }
                            match byte {
                                b'(' => parens += 1,
                                b')' => {
                                    parens = parens.saturating_sub(1);
                                    parameters_closed |= parens == 0;
                                }
                                b':' if parameters_closed
                                    && parens == 0
                                    && matches!(path.rsplit('.').next(), Some("ts" | "tsx")) =>
                                {
                                    return_type = true;
                                }
                                b'[' => brackets += 1,
                                b']' => brackets = brackets.saturating_sub(1),
                                // A prototype ends here, before any subsequent
                                // implementation. Array type semicolons do not.
                                b';' if parens == 0 && brackets == 0 => {
                                    end = n;
                                    complete = true;
                                    break 'body;
                                }
                                b'}' if parens == 0 && brackets == 0 => break 'body,
                                b'{' if parens == 0 && brackets == 0 => {
                                    // Only simple TS names are unambiguous here.
                                    // Structural/conditional/generic types need
                                    // a parser; their braces are not body proof.
                                    if return_type
                                        && matches!(path.rsplit('.').next(), Some("ts" | "tsx"))
                                        && !simple_typescript_return_type(&return_annotation)
                                    {
                                        break 'body;
                                    }
                                    opened = true;
                                    depth = 1;
                                }
                                _ => {}
                            }
                        } else {
                            match byte {
                                b'{' => depth += 1,
                                b'}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        end = n;
                                        complete = true;
                                        break 'body;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                (end, complete)
            }
            // Ruby's nested do/end syntax requires a parser to infer parents.
            Language::Ruby | Language::Other => (line, false),
        };
        declarations.push(Declaration {
            name: name.as_str().into(),
            line,
            end,
            complete,
        });
    }
    declarations
}
fn simple_typescript_return_type(annotation: &[u8]) -> bool {
    let Ok(annotation) = std::str::from_utf8(annotation) else {
        return false;
    };
    // Type operators can precede a structural type: `keyof { ... }`.
    !matches!(
        annotation.trim(),
        "keyof" | "typeof" | "readonly" | "infer" | "asserts" | "new" | "abstract"
    ) && patterns().simple_return_type.is_match(annotation)
}
impl<'a> Snapshot<'a> {
    fn with_navigation(
        path: &str,
        chunks: &[&'a Chunk],
        navigation: Option<&NavigationIndex>,
    ) -> Self {
        let mut snapshot = Self::new(path, chunks);
        if let Some(index) = navigation.filter(|_| crate::navigation::supports(path)) {
            // Parser-proven boundaries replace lexical guesses only for parsed
            // definitions. Unsupported or malformed syntax keeps the fallback.
            for definition in index.definitions(path) {
                if definition.kind != DefinitionKind::Function || !definition.complete {
                    continue;
                }
                if !(definition.start_line..=definition.end_line)
                    .all(|line| snapshot.lines.contains_key(&line))
                {
                    continue;
                }
                snapshot.declarations.retain(|d| {
                    d.line != definition.start_line
                        && !(d.name == definition.name
                            && (definition.start_line..=definition.end_line).contains(&d.line))
                });
                snapshot.declarations.push(Declaration {
                    name: definition.name.clone(),
                    line: definition.start_line,
                    end: definition.end_line,
                    complete: true,
                });
            }
            snapshot.declarations.sort_by_key(|d| d.line);
        }
        snapshot
    }
    fn new(path: &str, chunks: &[&'a Chunk]) -> Self {
        let mut lines = BTreeMap::new();
        let mut conflicts = BTreeSet::new();
        for chunk in chunks {
            if chunk.start_line == 0 || chunk.end_line < chunk.start_line {
                continue;
            }
            let count = chunk.end_line - chunk.start_line + 1;
            if chunk.text.split('\n').count() != count {
                continue;
            }
            for (offset, text) in chunk.text.split('\n').enumerate() {
                let number = chunk.start_line + offset;
                if lines
                    .insert(number, text)
                    .is_some_and(|previous| previous != text)
                {
                    conflicts.insert(number);
                }
            }
        }
        for number in conflicts {
            lines.remove(&number);
        }
        let code = code_lines(&lines, path);
        let declarations = declarations(path, &code);
        Self {
            lines,
            declarations,
            code,
        }
    }
    fn containing(&self, number: usize) -> Option<&Declaration> {
        self.declarations
            .iter()
            .filter(|d| d.line <= number && d.end >= number)
            .max_by_key(|d| d.line)
    }
}
fn prefix(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn symbol_header(snapshot: &Snapshot<'_>, declaration: &Declaration) -> SymbolHeader {
    let text = snapshot.lines[&declaration.line];
    let header = prefix(text, 320);
    SymbolHeader {
        name: declaration.name.clone(),
        line: declaration.line,
        text: header.into(),
        detection: "lexical_declaration",
        truncated: header.len() < text.len(),
    }
}
fn excerpt(
    path: &str,
    snapshot: &Snapshot<'_>,
    focus: usize,
    range: (usize, usize),
    limit: usize,
    preserve_range: bool,
) -> Option<SourceExcerpt> {
    snapshot.lines.get(&focus)?;
    let parent = snapshot.containing(focus);
    let first = *snapshot.lines.first_key_value()?.0;
    let last = *snapshot.lines.last_key_value()?.0;
    let bounded_parent = parent.filter(|d| d.complete);
    let low = parent.map_or(range.0.saturating_sub(8).max(first), |d| d.line);
    let mut high = bounded_parent.map_or(range.1.saturating_add(12).min(last), |d| d.end);
    if parent.is_some_and(|d| !d.complete)
        && let Some(next) = snapshot.declarations.iter().find(|d| d.line > focus)
    {
        high = high.min(next.line - 1);
    }
    let mut start = focus;
    let mut end = focus;
    // Do not discard evidence that survived ranking just because an earlier
    // line repeats more query words. The caller validates the winning interval.
    // Respect known declaration boundaries and keep byte fitting authoritative.
    if preserve_range
        && range.0 >= low
        && range.1 <= high
        && range.0 <= focus
        && focus <= range.1
        && range.1 - range.0 < limit
    {
        start = range.0;
        end = range.1;
    }
    // Grow around the useful source line, preferring following code 2:1.
    while end - start + 1 < limit {
        let before = start > low && snapshot.lines.contains_key(&(start - 1));
        let after = end < high && snapshot.lines.contains_key(&(end + 1));
        if !before && !after {
            break;
        }
        if before && (!after || (end - focus) >= (focus - start) * 2 + 2) {
            start -= 1;
        } else {
            end += 1;
        }
    }
    let text = (start..=end)
        .map(|n| snapshot.lines[&n])
        .collect::<Vec<_>>()
        .join("\n");
    let symbol = parent.map(|d| symbol_header(snapshot, d));
    let truncated = start > low
        || end < high
        // Without a proven declaration boundary, source context must not
        // advertise a complete implementation (even when it reaches EOF).
        || (bounded_parent.is_none() && language(path) != Language::Other)
        || symbol.as_ref().is_some_and(|s| s.truncated);
    Some(SourceExcerpt {
        path: path.into(),
        start_line: start,
        end_line: end,
        text,
        symbol,
        truncated,
        definition_complete: !truncated && bounded_parent.is_some(),
        focus_line: focus,
    })
}
fn query_terms(question: &str) -> HashSet<String> {
    const STOP: &[&str] = &[
        "where",
        "what",
        "which",
        "how",
        "the",
        "and",
        "for",
        "from",
        "with",
        "does",
        "that",
        "this",
        "are",
        "implemented",
    ];
    tokenize(question)
        .into_iter()
        .filter(|t| t.len() > 2 && !STOP.contains(&t.as_str()))
        .take(32)
        .collect()
}
fn focus_line(chunk: &Chunk, terms: &HashSet<String>, snapshot: &Snapshot<'_>) -> usize {
    chunk
        .text
        .split('\n')
        .enumerate()
        .max_by_key(|(offset, line)| {
            let matches = tokenize(line)
                .iter()
                .filter(|term| terms.contains(*term))
                .collect::<HashSet<_>>()
                .len();
            (
                matches,
                snapshot
                    .declarations
                    .iter()
                    .any(|d| d.line == chunk.start_line + offset),
                std::cmp::Reverse(*offset),
            )
        })
        .map_or(chunk.start_line, |(offset, _)| chunk.start_line + offset)
}
fn overlaps(a: &SourceExcerpt, b: &SourceExcerpt) -> bool {
    a.path == b.path && a.start_line <= b.end_line && b.start_line <= a.end_line
}

impl SourceExcerpt {
    fn shrink(&mut self) -> bool {
        if self.text.len() <= MIN_TEXT_BYTES {
            return false;
        }
        if self.start_line < self.end_line {
            // Preserve the useful line as the excerpt shrinks.
            if self.focus_line - self.start_line > self.end_line - self.focus_line {
                if let Some(newline) = self.text.find('\n') {
                    self.text.drain(..=newline);
                    self.start_line += 1;
                }
            } else if let Some(newline) = self.text.rfind('\n') {
                self.text.truncate(newline);
                self.end_line -= 1;
            }
        } else {
            let target = (self.text.len() * 3 / 4).max(MIN_TEXT_BYTES);
            self.text.truncate(prefix(&self.text, target).len());
        }
        self.truncated = true;
        self.definition_complete = false;
        true
    }
}
impl ContextPacket {
    fn prune_related(&mut self) {
        for related in &mut self.related {
            if let Some(target) = &related.target {
                if !self.results.iter().any(|result| {
                    result.excerpt.path == target.path
                        && (result.excerpt.start_line..=result.excerpt.end_line)
                            .contains(&target.line)
                }) {
                    self.truncated = true;
                    related.referenced_from.clear();
                }
                continue;
            }
            let before = related.referenced_from.len();
            related.referenced_from.retain(|source| {
                self.results.iter().any(|result| {
                    result.excerpt.path == source.path
                        && (result.excerpt.start_line..=result.excerpt.end_line)
                            .contains(&source.line)
                })
            });
            self.truncated |= related.referenced_from.len() != before;
        }
        self.related
            .retain(|related| !related.referenced_from.is_empty());
    }

    /// Fit a serialized packet, including escaping and metadata, to a byte budget.
    /// Drop lower-ranked matches, then related definitions, before shortening
    /// the primary evidence. Only shorten the primary when it cannot fit alone.
    /// Budgets below the empty packet size yield an empty
    /// packet; the caller must reserve at least 44 bytes for that JSON envelope.
    pub fn fit_to_budget(&mut self, max_bytes: usize) {
        while serde_json::to_vec(self)
            .expect("finite scores and strings")
            .len()
            > max_bytes
        {
            self.truncated = true;
            if self.results.len() > 1 {
                self.results.pop();
                self.prune_related();
            } else if self.related.pop().is_some() {
                // Related definitions are already ordered by query relevance.
            } else if let Some(primary) = self.results.first_mut() {
                if !primary.excerpt.shrink() {
                    self.results.pop();
                }
            } else {
                break;
            }
        }
    }
}

/// Expand up to three ranked winners and attach at most two lexical definition
/// candidates. All evidence comes from `corpus`; no reads or model calls occur.
pub fn build_packet(corpus: &[Chunk], winners: &[(Chunk, f64)], question: &str) -> ContextPacket {
    build_packet_inner(corpus, winners, question, None)
}

/// Use syntax facts from the same captured workspace snapshot as `corpus`.
/// No filesystem reads or model calls occur during expansion.
pub fn build_packet_with_navigation(
    corpus: &[Chunk],
    winners: &[(Chunk, f64)],
    question: &str,
    navigation: &NavigationIndex,
) -> ContextPacket {
    build_packet_inner(corpus, winners, question, Some(navigation))
}

fn build_packet_inner(
    corpus: &[Chunk],
    winners: &[(Chunk, f64)],
    question: &str,
    navigation: Option<&NavigationIndex>,
) -> ContextPacket {
    let mut packet = ContextPacket {
        results: vec![],
        related: vec![],
        truncated: false,
    };
    if winners.is_empty() {
        return packet;
    }
    let mut by_path: BTreeMap<&str, Vec<&Chunk>> = BTreeMap::new();
    for chunk in corpus {
        by_path.entry(&chunk.path).or_default().push(chunk);
    }
    let terms = query_terms(question);
    let mut snapshots = HashMap::new();
    for (chunk, score) in winners {
        if packet.results.len() == RESULT_LIMIT {
            packet.truncated = true;
            break;
        }
        let Some(chunks) = by_path.get(chunk.path.as_str()) else {
            packet.truncated = true;
            continue;
        };
        let snapshot = snapshots
            .entry(chunk.path.as_str())
            .or_insert_with(|| Snapshot::with_navigation(&chunk.path, chunks, navigation));
        // A caller-provided winner must actually match the source snapshot.
        if chunk.start_line == 0
            || chunk.end_line < chunk.start_line
            || chunk.text.split('\n').count() != chunk.end_line - chunk.start_line + 1
            || chunk
                .text
                .split('\n')
                .enumerate()
                .any(|(i, text)| snapshot.lines.get(&(chunk.start_line + i)) != Some(&text))
        {
            packet.truncated = true;
            continue;
        }
        let focus = focus_line(chunk, &terms, snapshot);
        let primary = packet.results.is_empty();
        let (limit, preserve_range) = if primary
            && snapshot.containing(focus).is_some_and(|declaration| {
                declaration.complete
                    && declaration.end - declaration.line < PRIMARY_IMPLEMENTATION_LINES
            }) {
            (PRIMARY_IMPLEMENTATION_LINES, false)
        } else if primary
            && language(&chunk.path) != Language::Other
            && chunk.end_line - chunk.start_line < PRIMARY_IMPLEMENTATION_LINES
        {
            (
                EXCERPT_LINES.max(chunk.end_line - chunk.start_line + 1),
                true,
            )
        } else {
            (EXCERPT_LINES, false)
        };
        let Some(context) = excerpt(
            &chunk.path,
            snapshot,
            focus,
            (chunk.start_line, chunk.end_line),
            limit,
            preserve_range,
        ) else {
            continue;
        };
        if packet
            .results
            .iter()
            .any(|r| overlaps(&r.excerpt, &context))
        {
            continue;
        }
        packet.results.push(ContextMatch {
            excerpt: context,
            score: if score.is_finite() { *score } else { 0.0 },
        });
    }
    if let Some(navigation) = navigation {
        attach_navigation(&mut packet, &by_path, navigation, question);
    }
    // Prefer names that occur in the question, then their order of appearance
    // in the ranked evidence. Keep the amount of expansion work bounded.
    let mut references: BTreeMap<String, Vec<related::Reference>> = BTreeMap::new();
    let mut name_order = Vec::new();
    for result in &packet.results {
        if language(&result.excerpt.path) == Language::Other
            || navigation.is_some_and(|index| index.is_parsed(&result.excerpt.path))
        {
            continue;
        }
        let snapshot = &snapshots[result.excerpt.path.as_str()];
        for reference in related::references(snapshot, &result.excerpt) {
            if !references.contains_key(&reference.name) {
                if name_order.len() == 32 {
                    continue;
                }
                name_order.push(reference.name.clone());
            }
            let locations = references.entry(reference.name.clone()).or_default();
            if locations.len() < 3 {
                locations.push(reference);
            }
        }
    }
    name_order.sort_by_key(|name| {
        std::cmp::Reverse(tokenize(name).iter().filter(|t| terms.contains(*t)).count())
    });
    let mut matcher = related::Matcher::new();
    let wanted_names = references
        .values()
        .flatten()
        .filter_map(|reference| {
            matcher.definition_name(reference, &snapshots[reference.source.path.as_str()])
        })
        .collect::<HashSet<_>>();
    let mut definitions: BTreeMap<String, Vec<(String, Declaration)>> = BTreeMap::new();
    if !references.is_empty() {
        for (&path, chunks) in &by_path {
            let typed = matches!(
                path.rsplit('.').next(),
                Some("java" | "cs" | "c" | "h" | "cc" | "cpp" | "hpp")
            );
            // Cheap declaration-only prefilter. Comments and string literals
            // can overmatch here; the full snapshot lexer validates candidates.
            let has_definition = |chunk: &&Chunk| {
                let named = |capture: regex::Captures<'_>| {
                    capture
                        .iter()
                        .skip(1)
                        .flatten()
                        .any(|name| wanted_names.contains(name.as_str()))
                };
                patterns()
                    .candidate_declaration
                    .captures_iter(&chunk.text)
                    .any(named)
                    || (typed
                        && patterns()
                            .candidate_typed
                            .captures_iter(&chunk.text)
                            .any(named))
            };
            if language(path) == Language::Other || !chunks.iter().any(has_definition) {
                continue;
            }
            let snapshot = snapshots
                .entry(path)
                .or_insert_with(|| Snapshot::new(path, chunks));
            for declaration in &snapshot.declarations {
                if wanted_names.contains(&declaration.name) {
                    definitions
                        .entry(declaration.name.clone())
                        .or_default()
                        .push((path.into(), declaration.clone()));
                }
            }
        }
    }
    for name in name_order {
        let mut selected: BTreeMap<(String, usize), (Declaration, Vec<SourceLocation>)> =
            BTreeMap::new();
        for reference in &references[&name] {
            let Some(target_name) =
                matcher.definition_name(reference, &snapshots[reference.source.path.as_str()])
            else {
                continue;
            };
            let Some(candidates) = definitions.get(&target_name) else {
                continue;
            };
            if let Some((path, declaration)) = matcher.select(reference, candidates, &snapshots) {
                let (_, sources) = selected
                    .entry((path.clone(), declaration.line))
                    .or_insert_with(|| (declaration.clone(), vec![]));
                if !sources.iter().any(|source| {
                    source.path == reference.source.path && source.line == reference.source.line
                }) {
                    sources.push(reference.source.clone());
                }
            }
        }
        for ((path, _), (declaration, sources)) in selected {
            if packet.related.len() == RELATED_LIMIT {
                break;
            }
            let snapshot = &snapshots[path.as_str()];
            let Some(context) = excerpt(
                &path,
                snapshot,
                declaration.line,
                (declaration.line, declaration.end),
                RELATED_LINES,
                false,
            ) else {
                continue;
            };
            if packet
                .results
                .iter()
                .any(|r| overlaps(&r.excerpt, &context))
                || packet
                    .related
                    .iter()
                    .any(|r| overlaps(&r.excerpt, &context))
            {
                continue;
            }
            packet.related.push(RelatedDefinition {
                excerpt: context,
                relation: "lexical_definition",
                ambiguous: false,
                candidate_count: 1,
                referenced_from: sources,
                target: None,
            });
        }
        if packet.related.len() == RELATED_LIMIT {
            break;
        }
    }
    packet.truncated |= packet.results.iter().any(|r| r.excerpt.truncated)
        || packet.related.iter().any(|r| r.excerpt.truncated);
    packet.fit_to_budget(PACKET_MAX_BYTES);
    packet
}

fn attach_navigation(
    packet: &mut ContextPacket,
    by_path: &BTreeMap<&str, Vec<&Chunk>>,
    navigation: &NavigationIndex,
    question: &str,
) {
    for result in &packet.results {
        if packet.related.len() >= RELATED_LIMIT {
            break;
        }
        for candidate in navigation.related(
            &result.excerpt.path,
            result.excerpt.start_line,
            result.excerpt.end_line,
            question,
            RELATED_LIMIT * 4,
        ) {
            if packet.related.len() >= RELATED_LIMIT {
                break;
            }
            let Some(chunks) = by_path.get(candidate.path.as_str()) else {
                continue;
            };
            let snapshot = Snapshot::with_navigation(&candidate.path, chunks, Some(navigation));
            if candidate.start_line == 0
                || candidate.end_line < candidate.start_line
                || !(candidate.start_line..=candidate.end_line)
                    .all(|line| snapshot.lines.contains_key(&line))
            {
                continue;
            }
            let caller = candidate.relation == Relation::Caller;
            let focus = if caller {
                candidate.reference_line
            } else {
                candidate.start_line
            };
            if !(candidate.start_line..=candidate.end_line).contains(&focus) {
                continue;
            }
            let start = if caller {
                focus.saturating_sub(8).max(candidate.start_line)
            } else {
                candidate.start_line
            };
            let end = candidate.end_line.min(start + RELATED_LINES - 1);
            let complete = start == candidate.start_line
                && end == candidate.end_line
                && navigation
                    .definitions(&candidate.path)
                    .iter()
                    .any(|d| d.complete && d.start_line == start && d.end_line == end);
            let context = SourceExcerpt {
                path: candidate.path.clone(),
                start_line: start,
                end_line: end,
                text: (start..=end)
                    .map(|line| snapshot.lines[&line])
                    .collect::<Vec<_>>()
                    .join("\n"),
                symbol: None,
                truncated: !complete,
                definition_complete: complete,
                focus_line: focus,
            };
            if packet
                .results
                .iter()
                .any(|r| overlaps(&r.excerpt, &context))
                || packet
                    .related
                    .iter()
                    .any(|r| overlaps(&r.excerpt, &context))
            {
                continue;
            }
            let anchor = if caller {
                candidate.target_line
            } else {
                candidate.reference_line
            };
            let anchor_path = if caller {
                &candidate.target_path
            } else {
                &candidate.reference_path
            };
            if anchor_path != &result.excerpt.path
                || !(result.excerpt.start_line..=result.excerpt.end_line).contains(&anchor)
            {
                continue;
            }
            packet.related.push(RelatedDefinition {
                excerpt: context,
                relation: if caller {
                    "resolved_caller"
                } else {
                    "resolved_definition"
                },
                ambiguous: false,
                candidate_count: 1,
                referenced_from: vec![SourceLocation {
                    path: candidate.reference_path,
                    line: candidate.reference_line,
                }],
                target: caller.then_some(SourceLocation {
                    path: candidate.target_path,
                    line: candidate.target_line,
                }),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::chunk_text;

    fn packet(path: &str, source: &str, question: &str) -> ContextPacket {
        let corpus = chunk_text(path, source);
        build_packet(&corpus, &[(corpus[0].clone(), 0.9)], question)
    }
    #[test]
    fn expands_with_exact_source_lines_and_deduplicates_overlapping_winners() {
        let source = format!(
            "fn process_video() {{\n{}\n}}\n",
            (0..160)
                .map(|n| format!("    step_{n}();"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let corpus = chunk_text("worker.rs", &source);
        let winners = vec![(corpus[1].clone(), 0.9), (corpus[1].clone(), 0.8)];
        let packet = build_packet(&corpus, &winners, "step_140");
        assert_eq!(packet.results.len(), 1);
        let result = &packet.results[0].excerpt;
        assert_eq!(result.symbol.as_ref().unwrap().name, "process_video");
        assert_eq!(result.symbol.as_ref().unwrap().line, 1);
        assert_eq!(
            result.text,
            source
                .lines()
                .skip(result.start_line - 1)
                .take(result.end_line - result.start_line + 1)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    #[test]
    fn long_signatures_include_the_body_and_stop_at_its_end() {
        for (path, header, parameter, suffix) in [
            (
                "worker.rs",
                "pub fn transform<T>(",
                "    input: [u8; 4],",
                ") where T: Clone {",
            ),
            (
                "worker.ts",
                "export function transform(",
                "    input: number,",
                ") {",
            ),
            ("worker.go", "func transform(", "    input int,", ") {"),
        ] {
            let source = format!(
                "{header}\n{}\n{suffix}\n    produce_output();\n}}\nfn unrelated() {{}}",
                [parameter; 16].join("\n")
            );
            let result = packet(path, &source, "transform");
            let excerpt = &result.results[0].excerpt;
            assert_eq!(excerpt.start_line, 1, "{path}");
            assert_eq!(excerpt.end_line, 20, "{path}");
            assert!(excerpt.text.contains("produce_output();"), "{path}");
            assert!(!excerpt.text.contains("unrelated"), "{path}");
            assert_eq!(excerpt.symbol.as_ref().unwrap().name, "transform");
            assert!(!excerpt.truncated, "{path}");
        }
    }
    #[test]
    fn incomplete_body_returns_context_without_claiming_complete_ownership() {
        let source = "fn transform() {\n    produce_output();\n    another_step();";
        for question in ["transform", "produce_output"] {
            let result = packet("worker.rs", source, question);
            let excerpt = &result.results[0].excerpt;
            assert!(excerpt.text.contains("produce_output();"));
            assert!(excerpt.truncated);
            assert!(result.truncated);
            if question == "produce_output" {
                assert!(excerpt.symbol.is_none());
            }
        }
    }
    #[test]
    fn bounded_signature_fallback_preserves_source_without_inventing_a_span() {
        let source = format!(
            "fn transform(\n{}\n) {{\n    produce_output();\n}}",
            ["    value: u32,"; SIGNATURE_LINES + 1].join("\n")
        );
        let result = packet("worker.rs", &source, "transform");
        let excerpt = &result.results[0].excerpt;
        assert!(excerpt.end_line > excerpt.start_line);
        assert!(excerpt.end_line - excerpt.start_line < PRIMARY_IMPLEMENTATION_LINES);
        assert!(excerpt.text.contains("value: u32"));
        assert!(excerpt.truncated);
        let chunks = chunk_text("worker.rs", &source);
        let refs = chunks.iter().collect::<Vec<_>>();
        let snapshot = Snapshot::new("worker.rs", &refs);
        assert!(snapshot.containing(SIGNATURE_LINES + 4).is_none());
    }
    #[test]
    fn prototypes_and_missing_signature_boundaries_do_not_capture_next_function() {
        let source =
            "fn pending(\n    values: [u8; 8],\n);\nfn actual() {\n    produce_output();\n}";
        let result = packet("worker.rs", source, "pending");
        let excerpt = &result.results[0].excerpt;
        assert_eq!(excerpt.end_line, 3);
        assert!(!excerpt.text.contains("actual"));
        assert!(!excerpt.truncated);
        let corpus = chunk_text("worker.rs", source);
        let winner = corpus
            .iter()
            .find(|chunk| chunk.text.contains("produce_output"))
            .unwrap();
        let result = build_packet(&corpus, &[(winner.clone(), 1.0)], "produce_output");
        assert_eq!(
            result.results[0].excerpt.symbol.as_ref().unwrap().name,
            "actual"
        );

        let source = "fn pending(\n    values: u32,\nfn actual() {\n    produce_output();\n}";
        let chunks = chunk_text("worker.rs", source);
        let refs = chunks.iter().collect::<Vec<_>>();
        let snapshot = Snapshot::new("worker.rs", &refs);
        assert_eq!(snapshot.declarations[0].end, 1);
        assert!(!snapshot.declarations[0].complete);
        assert_eq!(snapshot.containing(4).unwrap().name, "actual");
    }
    #[test]
    fn python_multiline_signature_includes_its_body() {
        let source = "def transform(\n    value: int,\n    other: str,\n) -> dict[str, int]:\n    produce_output()\n    return {}\ndef unrelated():\n    pass";
        let result = packet("worker.py", source, "transform");
        let excerpt = &result.results[0].excerpt;
        assert_eq!(excerpt.end_line, 6);
        assert!(excerpt.text.contains("produce_output()"));
        assert!(!excerpt.text.contains("unrelated"));
        assert!(!excerpt.truncated);
    }
    #[test]
    fn typescript_structural_return_type_uses_honest_context_fallback() {
        let source = "export function transform(): {\n    value: number;\n} {\n    return { value: 42 };\n}\nfunction unrelated() {}";
        let result = packet("worker.ts", source, "transform");
        let excerpt = &result.results[0].excerpt;
        assert!(excerpt.text.contains("return { value: 42 }"));
        assert!(!excerpt.text.contains("unrelated"));
        assert!(excerpt.truncated);
        let corpus = chunk_text("worker.ts", source);
        let refs = corpus.iter().collect::<Vec<_>>();
        let snapshot = Snapshot::new("worker.ts", &refs);
        assert!(snapshot.containing(4).is_none());
    }
    #[test]
    fn does_not_claim_a_nearby_function_as_parent() {
        let source = "fn old() {}\n\nstatic DATA: &[u8] = &[1, 2, 3];\n";
        let corpus = chunk_text("data.rs", source);
        let winner = Chunk {
            path: "data.rs".into(),
            start_line: 3,
            end_line: 3,
            text: "static DATA: &[u8] = &[1, 2, 3];".into(),
            lexical_score: 1.0,
        };
        let result = build_packet(&corpus, &[(winner, 1.0)], "DATA");
        assert!(result.results[0].excerpt.symbol.is_none());
    }
    #[test]
    fn ambiguous_related_definitions_and_missing_names_are_omitted() {
        let mut corpus = chunk_text(
            "main.rs",
            "fn main() {\n    encode_video();\n    absent();\n}\n",
        );
        let winner = corpus[0].clone();
        corpus.extend(chunk_text(
            "first.rs",
            "pub fn encode_video() {\n    output();\n}\n",
        ));
        corpus.extend(chunk_text("second.rs", "fn encode_video() {}\n"));
        corpus.extend(chunk_text("fake.md", "fn absent() {}\n"));
        let packet = build_packet(&corpus, &[(winner, 1.0)], "video");
        assert!(packet.related.is_empty());
    }
    #[test]
    fn supports_common_languages_and_plain_text_fallback() {
        for (path, source, name) in [
            ("main.py", "async def process():\n    send()\n", "process"),
            (
                "main.go",
                "func (s *Server) Serve() {\n  send()\n}\n",
                "Serve",
            ),
            (
                "main.ts",
                "export const render = () => {\n send();\n};\n",
                "render",
            ),
            (
                "Main.java",
                "public void render() {\n send();\n}\n",
                "render",
            ),
        ] {
            let packet = packet(path, source, "send");
            assert_eq!(
                packet.results[0].excerpt.symbol.as_ref().unwrap().name,
                name,
                "{path}"
            );
        }
        let packet = packet(
            "notes.txt",
            "Header\nimportant fact\nother context\n",
            "important",
        );
        assert!(packet.results[0].excerpt.symbol.is_none());
        assert!(packet.related.is_empty());
        assert!(packet.results[0].excerpt.text.contains("other context"));
        assert!(!packet.truncated);
    }
    #[test]
    fn comments_and_strings_do_not_create_fake_definitions_or_break_braces() {
        let mut corpus = chunk_text("main.py", "def main():\n    target()\n");
        let winner = corpus[0].clone();
        corpus.extend(chunk_text(
            "fake.py",
            "text = \"\"\"\ndef target():\n    pass\n\"\"\"\n",
        ));
        corpus.extend(chunk_text("fake.rs", "/*\nfn target() {}\n*/\n"));
        let packet = build_packet(&corpus, &[(winner, 0.9)], "target");
        assert!(packet.related.is_empty());
        let p = super::tests::packet(
            "real.rs",
            "fn real() {\n let x = \"}\";\n target();\n}\n",
            "target",
        );
        assert_eq!(p.results[0].excerpt.symbol.as_ref().unwrap().name, "real");
    }
    #[test]
    fn budgets_include_utf8_escaping_long_lines_and_prioritize_primary() {
        let mut corpus = Vec::new();
        for path in ["one.txt", "two.txt", "three.txt"] {
            corpus.extend(chunk_text(path, &format!("{}\n", "🦀\"\\\t".repeat(4000))));
        }
        let winners = corpus.iter().map(|c| (c.clone(), 1.0)).collect::<Vec<_>>();
        let mut packet = build_packet(&corpus, &winners, "question");
        assert!(serde_json::to_vec(&packet).unwrap().len() <= PACKET_MAX_BYTES);
        packet.fit_to_budget(6000);
        assert!(serde_json::to_vec(&packet).unwrap().len() <= 6000);
        assert_eq!(packet.results.len(), 1);
        assert_eq!(packet.results[0].excerpt.path, "one.txt");
        assert!(packet.truncated);
        assert!(
            packet.results.iter().all(|r| r.excerpt.truncated
                && r.excerpt.start_line == 1
                && r.excerpt.end_line == 1)
        );
    }
    #[test]
    fn complete_primary_expands_past_the_source_window_across_languages() {
        for (path, header, body, footer) in [
            (
                "delay.rs",
                "fn delay() {",
                "    record();",
                "    final_branch();\n}",
            ),
            (
                "delay.go",
                "func delay() {",
                "    record()",
                "    final_branch()\n}",
            ),
            (
                "delay.py",
                "def delay():",
                "    record()",
                "    final_branch()",
            ),
            (
                "delay.ts",
                "export function delay(args: {\n    count: number;\n}): DelayResult {",
                "    record();",
                "    return final_branch();\n}",
            ),
        ] {
            let source = format!("{header}\n{}\n{footer}", [body; 78].join("\n"));
            let result = packet(path, &source, "delay");
            assert_eq!(result.results[0].excerpt.text, source, "{path}");
            assert!(!result.results[0].excerpt.truncated, "{path}");
        }
    }

    #[test]
    fn simple_typescript_return_annotations_preserve_complete_bodies() {
        for annotation in [
            "number",
            "Result",
            "Types.Result",
            "Types.\n Result",
            "/* note */ Result",
        ] {
            let source = format!(
                "export function compute(value: number): {annotation} {{\n    return value;\n}}"
            );
            let result = packet("compute.ts", &source, "compute");
            assert_eq!(result.results[0].excerpt.text, source);
            assert!(!result.results[0].excerpt.truncated, "{annotation}");
        }
        for annotation in [
            "{ value: number }",
            "Promise<{ value: number }>",
            "Promise<Result>",
            "T extends X ? Y : Z",
            "value is Result",
            "keyof { value: number }",
            "Foo | Bar",
            "Foo[]",
        ] {
            let source = format!(
                "export function compute(value: number): {annotation} {{\n    return value;\n}}\nfunction other() {{}}"
            );
            let result = packet("compute.ts", &source, "compute");
            assert!(result.results[0].excerpt.truncated, "{annotation}");
            assert!(!result.results[0].excerpt.text.contains("function other"));
        }
    }

    #[test]
    fn budget_preserves_complete_primary_and_discards_orphan_helpers() {
        let source = "fn primary() {\n    shared();\n}";
        let mut corpus = chunk_text("primary.rs", source);
        let first = corpus[0].clone();
        let second = chunk_text(
            "secondary.rs",
            "fn secondary() {\n    shared();\n    secondary_only();\n}",
        );
        let winner = second[0].clone();
        corpus.extend(second);
        let mut result = build_packet(&corpus, &[(first, 1.0), (winner, 0.5)], "primary");
        let primary = result.results[0].clone();
        // Use explicit related provenance to isolate budgeting from resolution.
        let helper = packet("helper.rs", "fn shared() { work(); }", "shared")
            .results
            .remove(0)
            .excerpt;
        for (name, sources) in [
            (
                "shared.rs",
                vec![
                    SourceLocation {
                        path: "primary.rs".into(),
                        line: 2,
                    },
                    SourceLocation {
                        path: "secondary.rs".into(),
                        line: 2,
                    },
                ],
            ),
            (
                "secondary_helper.rs",
                vec![SourceLocation {
                    path: "secondary.rs".into(),
                    line: 3,
                }],
            ),
        ] {
            let mut excerpt = helper.clone();
            excerpt.path = name.into();
            result.related.push(RelatedDefinition {
                excerpt,
                relation: "lexical_definition",
                ambiguous: false,
                candidate_count: 1,
                referenced_from: sources,
                target: None,
            });
        }
        let mut expected = result.clone();
        expected.results.truncate(1);
        expected.related.truncate(1);
        expected.related[0].referenced_from.truncate(1);
        expected.truncated = true;
        let budget = serde_json::to_vec(&expected).unwrap().len();
        result.fit_to_budget(budget);
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let primary_only = ContextPacket {
            results: vec![primary],
            related: vec![],
            truncated: true,
        };
        let budget = serde_json::to_vec(&primary_only).unwrap().len();
        result.fit_to_budget(budget);
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::to_value(&primary_only).unwrap()
        );
        assert_eq!(result.results[0].excerpt.text, source);
        assert!(!result.results[0].excerpt.truncated);
        result.fit_to_budget(budget);
        assert_eq!(serde_json::to_vec(&result).unwrap().len(), budget);
        result.fit_to_budget(44);
        assert!(result.results.is_empty());
        assert!(result.related.is_empty());
        assert!(serde_json::to_vec(&result).unwrap().len() <= 44);
    }

    #[test]
    fn primary_fallback_preserves_ranked_evidence_without_claiming_a_complete_symbol() {
        for path in ["worker.rs", "worker.ts", "worker.py"] {
            let source = format!(
                "    // shipment routing destination selection\n{}\n    return selected_destination;",
                ["    record_step();"; 100].join("\n")
            );
            let winner = Chunk {
                path: path.into(),
                start_line: 101,
                end_line: 100 + source.lines().count(),
                text: source.clone(),
                lexical_score: 1.0,
            };
            let packet = build_packet(
                std::slice::from_ref(&winner),
                &[(winner.clone(), 1.0)],
                "shipment routing destination selection",
            );
            let result = &packet.results[0].excerpt;
            assert_eq!(result.start_line, winner.start_line);
            assert_eq!(result.end_line, winner.end_line);
            assert_eq!(result.text, source);
            assert!(result.symbol.is_none());
            assert!(result.truncated);
            assert!(packet.truncated);
        }
    }

    #[test]
    fn unbounded_caller_supplied_winner_keeps_the_focused_fallback() {
        let source = format!(
            "    select_destination();\n{}",
            ["    record_step();"; 300].join("\n")
        );
        let winner = Chunk {
            path: "worker.rs".into(),
            start_line: 1,
            end_line: source.lines().count(),
            text: source,
            lexical_score: 1.0,
        };
        let packet = build_packet(
            std::slice::from_ref(&winner),
            &[(winner.clone(), 1.0)],
            "select destination",
        );
        assert_eq!(
            packet.results[0].excerpt.text.lines().count(),
            EXCERPT_LINES
        );
        assert!(
            packet.results[0]
                .excerpt
                .text
                .contains("select_destination")
        );
        assert!(packet.results[0].excerpt.truncated);
    }

    #[test]
    fn oversized_primary_keeps_focus_and_exact_lines_when_shortened() {
        let source = format!(
            "fn process() {{\n{}\n    important_branch();\n{}\n}}",
            ["    work();"; 100].join("\n"),
            ["    other_work();"; 100].join("\n")
        );
        let corpus = chunk_text("process.rs", &source);
        let winner = corpus
            .iter()
            .find(|chunk| chunk.text.contains("important_branch"))
            .unwrap()
            .clone();
        let mut result = build_packet(&corpus, &[(winner, 1.0)], "important_branch");
        result.fit_to_budget(900);
        let excerpt = &result.results[0].excerpt;
        assert!(excerpt.truncated);
        assert!(excerpt.text.contains("important_branch"));
        assert_eq!(
            excerpt.text,
            source
                .lines()
                .skip(excerpt.start_line - 1)
                .take(excerpt.end_line - excerpt.start_line + 1)
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(serde_json::to_vec(&result).unwrap().len() <= 900);
    }

    #[test]
    fn same_named_implementations_in_different_files_remain_when_they_fit() {
        let mut corpus = chunk_text("first.rs", "fn process() { first(); }");
        corpus.extend(chunk_text("second.rs", "fn process() { second(); }"));
        let winners = corpus
            .iter()
            .cloned()
            .map(|chunk| (chunk, 1.0))
            .collect::<Vec<_>>();
        let result = build_packet(&corpus, &winners, "process");
        assert_eq!(result.results.len(), 2);
        assert!(result.results[0].excerpt.text.contains("first();"));
        assert!(result.results[1].excerpt.text.contains("second();"));
    }
    #[test]
    fn gaps_conflicts_and_missing_winners_do_not_fabricate_source() {
        let mut corpus = chunk_text("one.rs", "fn one() {\n do_work();\n}\n");
        let winner = corpus[0].clone();
        corpus.push(Chunk {
            path: "one.rs".into(),
            start_line: 2,
            end_line: 2,
            text: "different();".into(),
            lexical_score: 0.0,
        });
        let packet = build_packet(&corpus, &[(winner.clone(), 1.0)], "work");
        assert!(packet.results.is_empty());
        assert!(packet.truncated);
        assert!(
            build_packet(&[], &[(winner, 1.0)], "one")
                .results
                .is_empty()
        );
    }
    #[test]
    fn quoted_calls_raw_strings_and_rust_lifetimes_are_not_false_relations() {
        for (path, source) in [
            (
                "main.ts",
                "function real() {\n const x = 'prefix { false_helper() }';\n actual();\n}\n",
            ),
            (
                "main.py",
                "def real():\n    x = 'false_helper()'\n    actual()\n",
            ),
            (
                "main.rs",
                "fn real<'a>(x: &'a str, y: &'_ str) {\n let x = r###\"text \" fn false_helper() {\n } false_helper()\n \"###;\n actual();\n}\n",
            ),
        ] {
            let mut corpus = chunk_text(path, source);
            let winner = corpus[0].clone();
            let (other_path, other_source) = match path.rsplit('.').next().unwrap() {
                "ts" => (
                    "other.ts",
                    "function false_helper() {}\nfunction actual() {}\n",
                ),
                "py" => (
                    "other.py",
                    "def false_helper():\n    pass\ndef actual():\n    pass\n",
                ),
                _ => ("other.rs", "fn false_helper() {}\nfn actual() {}\n"),
            };
            corpus.extend(chunk_text(other_path, other_source));
            let packet = build_packet(&corpus, &[(winner, 1.0)], "actual");
            assert_eq!(
                packet.results[0].excerpt.symbol.as_ref().unwrap().name,
                "real",
                "{path}"
            );
            assert_eq!(packet.related.len(), 1, "{path}");
            assert_eq!(
                packet.related[0].excerpt.symbol.as_ref().unwrap().name,
                "actual",
                "{path}"
            );
        }
    }
    fn related_for_files(files: &[(&str, &str)], question: &str) -> ContextPacket {
        let mut corpus = chunk_text(files[0].0, files[0].1);
        let winner = corpus[0].clone();
        for (path, text) in &files[1..] {
            corpus.extend(chunk_text(path, text));
        }
        build_packet(&corpus, &[(winner, 1.0)], question)
    }
    #[test]
    fn qualified_and_receiver_calls_never_degrade_into_bare_names() {
        for call in [
            "urlencoding::encode()",
            "urlencoding\n :: encode()",
            "client.encode()",
            "client\n .encode()",
            "client->encode()",
            "foreign::nested::encode()",
            "foreign::Type::<T>::encode()",
        ] {
            let source = format!("fn login() {{\n {call};\n}}\n");
            let packet = related_for_files(
                &[
                    ("src/auth.rs", &source),
                    ("src/graphics.rs", "pub fn encode() {}\n"),
                ],
                "encode",
            );
            assert!(packet.related.is_empty(), "{call}");
        }
        let packet = related_for_files(
            &[
                ("src/auth.rs", "fn login() {\n urlencoding::encode();\n}\n"),
                ("src/urlencoding.rs", "pub fn encode() {}\n"),
            ],
            "encode",
        );
        assert!(
            packet.related.is_empty(),
            "a filename alone is not a local module declaration"
        );
    }
    #[test]
    fn bare_calls_do_not_choose_methods_or_other_languages() {
        for (path, source) in [
            (
                "main.rs",
                "fn main() {\n encode();\n}\nimpl Encoder {\n fn encode() {}\n}\n",
            ),
            (
                "main.py",
                "def main():\n    encode()\nclass Encoder:\n    def encode():\n        pass\n",
            ),
            (
                "main.ts",
                "function main() {\n encode();\n}\nclass Encoder {\n function encode() {}\n}\n",
            ),
        ] {
            let packet = related_for_files(&[(path, source)], "encode");
            assert!(packet.related.is_empty(), "{path}");
        }
        let packet = related_for_files(
            &[
                ("main.py", "def main():\n    encode()\n"),
                ("graphics.rs", "fn encode() {}\n"),
            ],
            "encode",
        );
        assert!(packet.related.is_empty());
    }
    #[test]
    fn local_bindings_and_parameters_block_free_function_expansion() {
        for source in [
            "fn main(encode: fn()) {\n encode();\n}\n",
            "fn main() {\n let encode = callback;\n encode();\n}\n",
            "fn main() {\n let (encode, other) = callbacks;\n encode();\n}\n",
        ] {
            let packet = related_for_files(
                &[("main.rs", source), ("encode.rs", "fn encode() {}\n")],
                "encode",
            );
            assert!(packet.related.is_empty(), "{source}");
        }
    }
    #[test]
    fn explicit_rust_paths_and_simple_import_aliases_select_only_their_module() {
        for source in [
            "fn main() {\n crate::encoding::encode();\n}\n",
            "mod encoding;\nfn main() {\n encoding::encode();\n}\n",
            "use crate::encoding::encode;\nfn main() {\n encode();\n}\n",
            "use crate::encoding::encode as write;\nfn main() {\n write();\n}\n",
            "use crate::encoding as codec;\nfn main() {\n codec::encode();\n}\n",
        ] {
            let packet = related_for_files(
                &[
                    ("src/main.rs", source),
                    ("src/encoding.rs", "pub fn encode() {}\n"),
                    ("src/graphics.rs", "pub fn encode() {}\n"),
                ],
                "encode",
            );
            assert_eq!(packet.related.len(), 1, "{source}");
            assert_eq!(
                packet.related[0].excerpt.path, "src/encoding.rs",
                "{source}"
            );
            assert!(!packet.related[0].ambiguous);
            assert_eq!(packet.related[0].candidate_count, 1);
        }
    }
    #[test]
    fn parent_module_import_retains_export_helper_and_same_file_function() {
        let source = "use super::overlay::run_overlay_only_export;\nfn run_export_job() {\n run_overlay_only_export();\n probe_source_video();\n}\n\nfn probe_source_video() {}\n";
        let mut corpus = chunk_text("src/export/job.rs", source);
        let winner = corpus
            .iter()
            .find(|chunk| chunk.text.contains("fn run_export_job"))
            .unwrap()
            .clone();
        corpus.extend(chunk_text(
            "src/export/overlay.rs",
            "pub fn run_overlay_only_export() {}\n",
        ));
        corpus.extend(chunk_text("src/other.rs", "fn probe_source_video() {}\n"));
        let packet = build_packet(&corpus, &[(winner, 1.0)], "run_export_job");
        assert_eq!(packet.related.len(), 2);
        assert!(
            packet
                .related
                .iter()
                .any(|r| r.excerpt.path == "src/export/overlay.rs")
        );
        assert!(
            packet
                .related
                .iter()
                .any(|r| r.excerpt.path == "src/export/job.rs"
                    && r.excerpt.symbol.as_ref().unwrap().name == "probe_source_video")
        );
    }
    #[test]
    fn uncertain_imports_do_not_fall_back_to_unrelated_free_functions() {
        for source in [
            "use external::encode;\nfn main() {\n encode();\n}\n",
            "use external::{encode};\nfn main() {\n encode();\n}\n",
            "use external::*;\nfn main() {\n encode();\n}\n",
        ] {
            let packet = related_for_files(
                &[("main.rs", source), ("graphics.rs", "fn encode() {}\n")],
                "encode",
            );
            assert!(packet.related.is_empty(), "{source}");
        }
    }
    #[test]
    fn same_file_sibling_modules_need_an_explicit_path() {
        for (call, expected) in [("encode()", 0), ("crate::graphics::encode()", 1)] {
            let source =
                format!("mod graphics {{\n pub fn encode() {{}}\n}}\nfn main() {{\n {call};\n}}\n");
            let corpus = chunk_text("src/main.rs", &source);
            let winner = corpus
                .iter()
                .find(|chunk| chunk.text.contains("fn main"))
                .unwrap()
                .clone();
            let packet = build_packet(&corpus, &[(winner, 1.0)], "main");
            assert_eq!(packet.related.len(), expected, "{call}");
        }
    }
    #[test]
    fn multiline_imports_wildcards_and_global_bindings_block_external_names() {
        for (path, source, helper_path, helper) in [
            (
                "main.ts",
                "import {\n encode\n} from 'external';\nfunction main() {\n encode();\n}\n",
                "local.ts",
                "function encode() {}\n",
            ),
            (
                "main.ts",
                "const encode = require('external');\nfunction main() {\n encode();\n}\n",
                "local.ts",
                "function encode() {}\n",
            ),
            (
                "main.py",
                "from external import (\n    encode\n)\ndef main():\n    encode()\n",
                "local.py",
                "def encode():\n    pass\n",
            ),
            (
                "main.py",
                "from external import *\ndef main():\n    encode()\n",
                "local.py",
                "def encode():\n    pass\n",
            ),
            (
                "main.py",
                "encode = external()\ndef main():\n    encode()\n",
                "local.py",
                "def encode():\n    pass\n",
            ),
        ] {
            let packet = related_for_files(&[(path, source), (helper_path, helper)], "main");
            assert!(packet.related.is_empty(), "{source}");
        }
    }
    #[test]
    fn same_file_arrow_function_is_still_eligible() {
        let corpus = chunk_text(
            "main.ts",
            "const encode = () => {\n return 1;\n};\nfunction main() {\n encode();\n}\n",
        );
        let winner = corpus
            .iter()
            .find(|chunk| chunk.text.contains("function main"))
            .unwrap()
            .clone();
        let packet = build_packet(&corpus, &[(winner, 1.0)], "main");
        assert_eq!(packet.related.len(), 1);
        assert_eq!(
            packet.related[0].excerpt.symbol.as_ref().unwrap().name,
            "encode"
        );
    }
    #[test]
    fn query_matching_references_outrank_incidental_helpers() {
        let mut corpus = chunk_text(
            "main.rs",
            "fn export() {\n initialize();\n log_state();\n encode_video();\n}\n",
        );
        let winner = corpus[0].clone();
        for name in ["initialize", "log_state", "encode_video"] {
            corpus.extend(chunk_text(
                &format!("{name}.rs"),
                &format!("fn {name}() {{}}\n"),
            ));
        }
        let packet = build_packet(&corpus, &[(winner, 1.0)], "video encoding");
        assert_eq!(packet.related.len(), 2);
        assert_eq!(
            packet.related[0].excerpt.symbol.as_ref().unwrap().name,
            "encode_video"
        );
    }
}
