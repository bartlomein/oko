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
// Five was measured on 169 replayed agent questions: Jev rarely accepts more
// than three candidates, so coverage did not move, while keyword-ranked
// responses grew by half. Further candidates are named by path instead.
const RESULT_LIMIT: usize = 3;
const RELATED_LIMIT: usize = 2;
const EXCERPT_LINES: usize = 60;
// Return a proven implementation whole: a window that stops short of the
// relevant line costs the agent a follow-up read, or a wrong answer if it
// trusts the window. Lower-ranked matches give this up first under the byte
// budget. Uncertain declarations can retain the bounded primary winning chunk
// without claiming a complete function. Larger spans use a focused source window.
const PRIMARY_IMPLEMENTATION_LINES: usize = 256;
const RELATED_LINES: usize = 32;
// On replayed agent questions, over a quarter of the expected code missing from
// a response began one or two lines after a shown definition, in the next short
// one: the predicate a parser calls, the sibling method the question names.
// Agents read on regardless, at a model turn each. A neighbour must be tied to
// the match, by a reference or by the question; larger ones are left to ranking.
const SIBLING_LINES: usize = 20;
const SIBLING_BUDGET: usize = 30;
// Candidates the ranker rated just below its cutoff held as much of the missing
// code again, while most responses used a fraction of the budget and one or two
// of three slots. They are shown only in a spare slot of a small response, as a
// focused window, and labelled, because most of them are not what was asked for.
// The label states what they are and gives no instruction: agents that follow
// instructions literally turn "check this" into extra reads.
const RUNNER_UP_BELOW_BYTES: usize = 6_000;
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
    /// Whole matches or related definitions were dropped, as opposed to an
    /// included excerpt merely being partial.
    #[serde(skip)]
    pub omitted: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextMatch {
    #[serde(flatten)]
    pub excerpt: SourceExcerpt,
    pub score: f64,
    /// Rated below the relevance cutoff; shown because the response had room.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub lower_confidence: bool,
    /// The focused window that replaces a lower-ranked complete definition
    /// before any match is dropped to fit the budget.
    #[serde(skip)]
    compact: Option<SourceExcerpt>,
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
    /// A whole file is never incomplete, even without a proven declaration boundary.
    pub truncated: bool,
    /// The excerpt is the entire file, so rereading it adds nothing.
    pub whole_file: bool,
    /// True only when the complete enclosing definition is proven and included.
    /// This says nothing about whether all surrounding dependencies were returned.
    pub definition_complete: bool,
    /// Proven complete definitions in the excerpt: the matched one and any short
    /// definitions that directly follow it. Zero unless `definition_complete`.
    pub definitions: usize,
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
    /// Chunks tile a file from line 1, so a gapless range ending at the last
    /// known line is the entire file.
    fn covers_whole_file(&self, start: usize, end: usize) -> bool {
        start == 1
            && self
                .lines
                .last_key_value()
                .is_some_and(|(last, _)| *last == end && self.lines.len() == end)
    }
    /// Decorators, attributes, and comments directly above a declaration belong
    /// to it: `@classmethod` changes what the function is, and an edit to the
    /// function usually touches its comment. A blank line, other code, or
    /// another declaration ends the attachment.
    fn attached_header_start(&self, declaration: usize) -> usize {
        const ATTACHED_LINES: usize = 24;
        let mut start = declaration;
        while start > 1 && declaration - start < ATTACHED_LINES {
            let line = start - 1;
            let (Some(raw), Some(code)) = (self.lines.get(&line), self.code.get(&line)) else {
                break;
            };
            let code = code.trim();
            let attached = !raw.trim().is_empty()
                && (code.is_empty() || code.starts_with('@') || code.starts_with("#["));
            if !attached
                || self
                    .declarations
                    .iter()
                    .any(|d| d.line <= line && d.end >= line)
            {
                break;
            }
            start = line;
        }
        start
    }
    /// The end of a short, proven definition that directly follows line `end`:
    /// only blank lines may separate them, and everything attached above the
    /// sibling comes with it.
    fn following_sibling(
        &self,
        (start, end): (usize, usize),
        terms: &HashSet<String>,
    ) -> Option<usize> {
        let next = self.declarations.iter().find(|d| d.line > end)?;
        // The shown code uses it, or the question asks about it by name.
        let referenced = (start..=end).any(|n| {
            self.code.get(&n).is_some_and(|code| {
                code.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                    .any(|word| word == next.name)
            })
        });
        let asked = tokenize(&next.name).iter().any(|part| {
            part.len() >= 4
                && terms.iter().any(|term| {
                    term.len() >= 4
                        && (term.starts_with(part.as_str()) || part.starts_with(term.as_str()))
                })
        });
        if !referenced && !asked {
            return None;
        }
        let header = self.attached_header_start(next.line);
        let adjoins = (end + 1..header).all(|n| {
            self.lines
                .get(&n)
                .is_some_and(|line| line.trim().is_empty())
        });
        let present = (header..=next.end).all(|n| self.lines.contains_key(&n));
        (next.complete && adjoins && present && header > end && next.end - header < SIBLING_LINES)
            .then_some(next.end)
    }
    /// A doc comment often repeats the question better than the code it
    /// documents, and chunks begin at that comment. Only comments and blank
    /// lines may separate the line from the declaration it introduces.
    fn documented_declaration(&self, line: usize, last: usize) -> Option<usize> {
        if self.containing(line).is_some() {
            return None;
        }
        let declaration = self
            .declarations
            .iter()
            .find(|d| d.line > line && d.line <= last)?;
        (line..declaration.line)
            .all(|n| self.code.get(&n).is_some_and(|code| code.trim().is_empty()))
            .then_some(declaration.line)
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
    // The question's words, for choosing which following definitions belong.
    terms: &HashSet<String>,
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
    let symbol = parent.map(|d| symbol_header(snapshot, d));
    // Without a proven declaration boundary, source context must not
    // advertise a complete implementation (even when it reaches EOF).
    let unproven = start > low
        || end < high
        || (bounded_parent.is_none() && language(path) != Language::Other)
        || symbol.as_ref().is_some_and(|s| s.truncated);
    // A definition begins with what is attached to it, not with its keyword.
    if parent.is_some_and(|d| d.line == start) {
        start = snapshot.attached_header_start(start);
    }
    let mut definitions = usize::from(!unproven && bounded_parent.is_some());
    if definitions == 1 {
        let mut added = 0;
        while let Some(sibling_end) = snapshot.following_sibling((start, end), terms) {
            let size = sibling_end - end;
            if added + size > SIBLING_BUDGET || end - start + 1 + size > limit {
                break;
            }
            end = sibling_end;
            added += size;
            definitions += 1;
        }
    }
    let text = (start..=end)
        .map(|n| snapshot.lines[&n])
        .collect::<Vec<_>>()
        .join("\n");
    // Nothing is missing from a whole file, so it is not an incomplete
    // excerpt; that still proves no declaration boundary.
    let whole_file = snapshot.covers_whole_file(start, end);
    Some(SourceExcerpt {
        path: path.into(),
        start_line: start,
        end_line: end,
        text,
        symbol,
        truncated: unproven && !whole_file,
        whole_file,
        definition_complete: definitions > 0,
        definitions,
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
        self.whole_file = false;
        self.definition_complete = false;
        self.definitions = 0;
        true
    }
}
impl SourceExcerpt {
    /// `path:start-end (label)` followed by the exact source in a fence that
    /// the source itself cannot close. Every line carries its file line number
    /// and a tab: models count lines unreliably, so an agent asked for a
    /// location would otherwise cite a line or two off.
    fn render(&self, lower_confidence: bool, out: &mut String) {
        let mut label = if self.whole_file {
            "whole file".to_owned()
        } else if self.definitions > 1 {
            format!("{} complete definitions", self.definitions)
        } else if self.definition_complete {
            "complete definition".to_owned()
        } else {
            "partial excerpt".to_owned()
        };
        if lower_confidence {
            label.push_str(", possible match");
        }
        let longest_run = self
            .text
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0);
        let fence = "`".repeat(longest_run.max(2) + 1);
        out.push_str(&format!(
            "{}:{}-{} ({label})\n{fence}\n",
            self.path, self.start_line, self.end_line
        ));
        for (offset, line) in self.text.split('\n').enumerate() {
            out.push_str(&format!("{}\t{line}\n", self.start_line + offset));
        }
        out.push_str(&fence);
        out.push('\n');
    }
}
impl ContextPacket {
    /// Compact agent-facing rendering: exact source without JSON escaping,
    /// scores, or serving metadata. Matches are in ranked order.
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        for result in &self.results {
            if !out.is_empty() {
                out.push('\n');
            }
            result.excerpt.render(result.lower_confidence, &mut out);
        }
        for related in &self.related {
            let relation = match related.relation {
                "resolved_definition" => "Definition",
                "resolved_caller" => "Caller",
                _ => "Possible definition",
            };
            let anchor = related.target.as_ref().map_or_else(
                || {
                    related
                        .referenced_from
                        .iter()
                        .map(|source| format!("{}:{}", source.path, source.line))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                |target| format!("{}:{}", target.path, target.line),
            );
            let verb = if related.target.is_some() {
                "of"
            } else {
                "referenced from"
            };
            out.push_str(&format!("\n{relation} {verb} {anchor}:\n"));
            related.excerpt.render(false, &mut out);
        }
        if self.omitted {
            out.push_str("\nLower-ranked evidence was omitted to fit the response limit.\n");
        }
        out
    }

    fn prune_related(&mut self) {
        for related in &mut self.related {
            if let Some(target) = &related.target {
                if !self.results.iter().any(|result| {
                    result.excerpt.path == target.path
                        && (result.excerpt.start_line..=result.excerpt.end_line)
                            .contains(&target.line)
                }) {
                    self.truncated = true;
                    self.omitted = true;
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
            let pruned = related.referenced_from.len() != before;
            self.truncated |= pruned;
            self.omitted |= pruned;
        }
        self.related
            .retain(|related| !related.referenced_from.is_empty());
    }

    /// Fit a serialized packet, including escaping and metadata, to a byte budget.
    /// Narrow lower-ranked complete definitions to their focused window, then
    /// drop lower-ranked matches, then related definitions, before shortening
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
            if let Some(result) = self
                .results
                .iter_mut()
                .skip(1)
                .rev()
                .find(|result| result.compact.is_some())
            {
                result.excerpt = result.compact.take().expect("checked above");
                self.prune_related();
            } else if self.results.len() > 1 {
                self.results.pop();
                self.omitted = true;
                self.prune_related();
            } else if self.related.pop().is_some() {
                // Related definitions are already ordered by query relevance.
                self.omitted = true;
            } else if let Some(primary) = self.results.first_mut() {
                if !primary.excerpt.shrink() {
                    self.results.pop();
                    self.omitted = true;
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
    build_packet_inner(corpus, winners, &[], question, None)
}

/// Use syntax facts from the same captured workspace snapshot as `corpus`.
/// No filesystem reads or model calls occur during expansion.
pub fn build_packet_with_navigation(
    corpus: &[Chunk],
    winners: &[(Chunk, f64)],
    question: &str,
    navigation: &NavigationIndex,
) -> ContextPacket {
    build_packet_inner(corpus, winners, &[], question, Some(navigation))
}

/// As `build_packet_with_navigation`, offering candidates rated just below the
/// relevance cutoff for slots the accepted winners leave free. They appear only
/// while the packet is small, and are labelled as possible matches.
pub fn build_packet_with_runners_up(
    corpus: &[Chunk],
    winners: &[(Chunk, f64)],
    runners_up: &[(Chunk, f64)],
    question: &str,
    navigation: &NavigationIndex,
) -> ContextPacket {
    build_packet_inner(corpus, winners, runners_up, question, Some(navigation))
}

fn build_packet_inner(
    corpus: &[Chunk],
    winners: &[(Chunk, f64)],
    runners_up: &[(Chunk, f64)],
    question: &str,
    navigation: Option<&NavigationIndex>,
) -> ContextPacket {
    let mut packet = ContextPacket {
        results: vec![],
        related: vec![],
        truncated: false,
        omitted: false,
    };
    // Nothing accepted is an answer in itself; runners-up only accompany a match.
    if winners.is_empty() {
        return packet;
    }
    let mut by_path: BTreeMap<&str, Vec<&Chunk>> = BTreeMap::new();
    for chunk in corpus {
        by_path.entry(&chunk.path).or_default().push(chunk);
    }
    let terms = query_terms(question);
    let mut snapshots = HashMap::new();
    let ranked = winners
        .iter()
        .map(|winner| (winner, false))
        .chain(runners_up.iter().map(|runner_up| (runner_up, true)));
    for ((chunk, score), lower_confidence) in ranked {
        if packet.results.len() == RESULT_LIMIT {
            // Further accepted matches are named by path after the excerpts;
            // nothing was cut for size.
            packet.truncated |= !lower_confidence;
            break;
        }
        if lower_confidence
            && serde_json::to_vec(&packet)
                .map_or(true, |bytes| bytes.len() >= RUNNER_UP_BELOW_BYTES)
        {
            break;
        }
        let Some(chunks) = by_path.get(chunk.path.as_str()) else {
            packet.truncated = true;
            packet.omitted = true;
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
            packet.omitted = true;
            continue;
        }
        let focus = focus_line(chunk, &terms, snapshot);
        let focus = snapshot
            .documented_declaration(focus, chunk.end_line)
            .unwrap_or(focus);
        let primary = packet.results.is_empty();
        // A runner-up earns a focused window, not a long implementation.
        let whole = if lower_confidence {
            EXCERPT_LINES
        } else {
            PRIMARY_IMPLEMENTATION_LINES
        };
        let proven = snapshot.containing(focus).is_some_and(|declaration| {
            declaration.complete && declaration.end - declaration.line < whole
        });
        let (limit, preserve_range) = if proven {
            (whole, false)
        } else if primary
            && !lower_confidence
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
            &terms,
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
        let compact = (!primary && proven)
            .then(|| {
                excerpt(
                    &chunk.path,
                    snapshot,
                    focus,
                    (chunk.start_line, chunk.end_line),
                    EXCERPT_LINES,
                    false,
                    &terms,
                )
            })
            .flatten()
            .filter(|window| {
                (window.start_line, window.end_line) != (context.start_line, context.end_line)
            });
        packet.results.push(ContextMatch {
            excerpt: context,
            score: if score.is_finite() { *score } else { 0.0 },
            lower_confidence,
            compact,
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
                // A supporting definition stays as small as it is.
                &HashSet::new(),
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
            let whole_file = snapshot.covers_whole_file(start, end);
            let context = SourceExcerpt {
                path: candidate.path.clone(),
                start_line: start,
                end_line: end,
                text: (start..=end)
                    .map(|line| snapshot.lines[&line])
                    .collect::<Vec<_>>()
                    .join("\n"),
                symbol: None,
                truncated: !complete && !whole_file,
                whole_file,
                definition_complete: complete,
                definitions: usize::from(complete),
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
            // The whole file is present, so nothing is missing to reread,
            // but an unclosed body is never a proven definition.
            assert!(excerpt.whole_file);
            assert!(!excerpt.truncated);
            assert!(!excerpt.definition_complete);
            if question == "produce_output" {
                assert!(excerpt.symbol.is_none());
            }
        }
    }
    #[test]
    fn whole_file_without_a_declaration_is_not_reported_as_incomplete() {
        let source = "export const RETRY_BACKOFF = {\n  strategy: 'exponential',\n  initialDelayMilliseconds: 5_000,\n} as const;";
        let result = packet("retry.constant.ts", source, "retry backoff initial delay");
        let excerpt = &result.results[0].excerpt;
        assert_eq!((excerpt.start_line, excerpt.end_line), (1, 4));
        assert!(excerpt.whole_file);
        assert!(!excerpt.truncated);
        assert!(!excerpt.definition_complete);
        assert!(!result.truncated && !result.omitted);
        assert_eq!(
            result.render_text(),
            format!(
                "retry.constant.ts:1-4 (whole file)\n```\n{}\n```\n",
                source
                    .lines()
                    .enumerate()
                    .map(|(index, line)| format!("{}\t{line}", index + 1))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        );
    }
    #[test]
    fn rendered_text_labels_completeness_and_survives_embedded_fences() {
        let source = format!(
            "fn transform() {{\n    // ```\n{}\n}}\nfn unrelated() {{}}",
            ["    step();"; 3].join("\n")
        );
        let text = packet("worker.rs", &source, "transform").render_text();
        assert!(
            text.starts_with("worker.rs:1-6 (complete definition)\n````\n1\tfn transform() {"),
            "{text}"
        );
        assert!(text.ends_with("6\t}\n````\n"), "{text}");
        assert!(!text.contains("score"));
        let long = format!("fn transform() {{\n{}\n}}", ["    step();"; 400].join("\n"));
        let packet = packet("worker.rs", &long, "transform");
        assert!(packet.results[0].excerpt.truncated);
        assert!(packet.render_text().contains("(partial excerpt)\n"));
    }
    #[test]
    fn lower_ranked_definitions_are_whole_and_narrow_before_any_match_is_dropped() {
        let function = |name: &str, marker: &str| {
            let mut lines = vec![format!("fn {name}() {{")];
            lines.extend((0..100).map(|line| format!("    step_{line}();")));
            lines[5] = format!("    {marker}_start();");
            lines[95] = format!("    {marker}_late_decision();");
            lines.push("}".into());
            lines.join("\n")
        };
        let first = function("probe", "probe");
        let second = function("validate", "validate");
        let mut corpus = chunk_text("probe.rs", &first);
        corpus.extend(chunk_text("validate.rs", &second));
        let winners: Vec<_> = ["probe.rs", "validate.rs"]
            .iter()
            .map(|path| {
                let chunk = corpus.iter().find(|chunk| chunk.path == *path).unwrap();
                (chunk.clone(), 0.9)
            })
            .collect();
        let packet = build_packet(&corpus, &winners, "validate start");
        let lower = &packet.results[1].excerpt;
        assert_eq!((lower.start_line, lower.end_line), (1, 102));
        assert!(lower.definition_complete && !lower.truncated);
        assert!(lower.text.contains("validate_late_decision"));
        assert!(!packet.truncated && !packet.omitted);

        // Too small for both definitions, large enough for a focused window.
        let mut fitted = packet.clone();
        fitted.fit_to_budget(serde_json::to_vec(&packet).unwrap().len() - 600);
        assert_eq!(fitted.results.len(), 2, "narrowed instead of dropped");
        assert_eq!(fitted.results[0].excerpt.text, first);
        let narrowed = &fitted.results[1].excerpt;
        assert_eq!(narrowed.end_line - narrowed.start_line + 1, EXCERPT_LINES);
        assert!(narrowed.truncated && !narrowed.definition_complete);
        assert!(narrowed.text.contains("validate_start"));
        assert!(fitted.truncated && !fitted.omitted);
        assert!(
            fitted
                .render_text()
                .contains("validate.rs:1-60 (partial excerpt)")
        );

        // The previous policy still applies once narrowing is exhausted.
        fitted.fit_to_budget(serde_json::to_vec(&fitted).unwrap().len() - 600);
        assert_eq!(fitted.results.len(), 1);
        assert_eq!(fitted.results[0].excerpt.text, first);
        assert!(fitted.omitted);
    }
    #[test]
    fn excerpts_begin_with_the_decorators_and_comments_attached_to_a_definition() {
        // The winner is the chunk holding `needle`, wherever it is in the file.
        let packet = |path: &str, source: &str, needle: &str| {
            let corpus = chunk_text(path, source);
            let winner = corpus
                .iter()
                .find(|chunk| chunk.text.contains(needle))
                .unwrap()
                .clone();
            build_packet(&corpus, &[(winner, 0.9)], needle)
        };
        let python = "def first():\n    return 1\n\n@cache\n@validate(strict=True)\ndef get_reason_phrase(value):\n    try:\n        return codes(value).phrase\n    except ValueError:\n        return \"\"\n\n@cache\ndef other():\n    return 2";
        let result = packet("status.py", python, "get_reason_phrase");
        let excerpt = &result.results[0].excerpt;
        assert_eq!(excerpt.start_line, 4, "{}", excerpt.text);
        assert!(
            excerpt
                .text
                .starts_with("@cache\n@validate(strict=True)\ndef get_reason_phrase")
        );
        assert!(
            !excerpt.text.contains("return 1"),
            "the previous function is not attached"
        );
        assert!(
            !excerpt.text.contains("def other"),
            "nor is the next one's decorator"
        );

        let rust = "fn before() {}\n\n/// Parses the header.\n#[inline]\npub fn first_forwarded_value() -> u32 {\n    1\n}";
        let result = packet("request.rs", rust, "first_forwarded_value");
        let excerpt = &result.results[0].excerpt;
        assert_eq!(
            (excerpt.start_line, excerpt.end_line),
            (3, 7),
            "{}",
            excerpt.text
        );
        assert!(excerpt.definition_complete && !excerpt.truncated);

        // A blank line ends the attachment: that comment describes something else.
        let detached =
            "// Section heading\n\nfn first_forwarded_value() -> u32 {\n    1\n}\nfn after() {}";
        let result = packet("plain.rs", detached, "first_forwarded_value");
        assert_eq!(result.results[0].excerpt.start_line, 3);
    }
    #[test]
    fn a_short_following_definition_comes_along_only_when_tied_to_the_match() {
        let pick = |source: &str, needle: &str, question: &str| {
            let corpus = chunk_text("src/interpolate.rs", source);
            let winner = corpus
                .iter()
                .find(|chunk| chunk.text.contains(needle))
                .unwrap()
                .clone();
            build_packet(&corpus, &[(winner, 0.9)], question)
        };
        // The shown function calls it.
        let called = "fn find_cap_ref(bytes: &[u8]) -> usize {\n    bytes.iter().take_while(|b| is_valid_cap_letter(b)).count()\n}\n\n/// Whether the byte may appear in a capture name.\n#[inline]\nfn is_valid_cap_letter(b: &u8) -> bool {\n    b.is_ascii_alphanumeric()\n}\n\nfn unrelated() {}";
        let packet = pick(called, "fn find_cap_ref", "find capture reference");
        let excerpt = &packet.results[0].excerpt;
        assert_eq!((excerpt.start_line, excerpt.end_line), (1, 9));
        assert_eq!(excerpt.definitions, 2);
        assert!(excerpt.definition_complete && !excerpt.truncated);
        assert!(
            packet
                .render_text()
                .starts_with("src/interpolate.rs:1-9 (2 complete definitions)\n")
        );
        assert!(
            !excerpt.text.contains("fn unrelated"),
            "nothing ties it to the match"
        );

        // The question asks about it by name.
        let asked = "class MultiDecoder:\n    def __init__(self, children):\n        self.children = list(reversed(children))\n\n    def decode(self, data):\n        return data\n\n    def flush(self):\n        return b\"\"";
        let corpus = chunk_text("httpx/_decoders.py", asked);
        let winner = corpus
            .iter()
            .find(|chunk| chunk.text.contains("def __init__"))
            .unwrap()
            .clone();
        let packet = build_packet(
            &corpus,
            &[(winner, 0.9)],
            "how the decoder chain is applied",
        );
        let excerpt = &packet.results[0].excerpt;
        assert!(
            excerpt.text.contains("def decode(self, data):"),
            "{}",
            excerpt.text
        );
        assert!(!excerpt.text.contains("def flush"), "{}", excerpt.text);

        // Neither: the match stays as small as it is.
        let alone = "fn find_cap_ref() -> usize {\n    1\n}\n\nfn unrelated() -> usize {\n    2\n}";
        let excerpt = &pick(alone, "fn find_cap_ref", "find capture reference").results[0].excerpt;
        assert_eq!(
            (excerpt.start_line, excerpt.end_line, excerpt.definitions),
            (1, 3, 1)
        );

        // Code between them, or a long neighbour, is left to ranking.
        let apart = "fn find_cap_ref() -> bool {\n    is_valid()\n}\nconst LIMIT: usize = 3;\nfn is_valid() -> bool {\n    true\n}";
        assert_eq!(
            pick(apart, "fn find_cap_ref", "find").results[0]
                .excerpt
                .end_line,
            3
        );
        let long = format!(
            "fn find_cap_ref() -> bool {{\n    is_valid()\n}}\n\nfn is_valid() -> bool {{\n{}    true\n}}",
            "    step();\n".repeat(SIBLING_LINES)
        );
        assert_eq!(
            pick(&long, "fn find_cap_ref", "find").results[0]
                .excerpt
                .end_line,
            3
        );
    }
    #[test]
    fn runners_up_fill_spare_slots_of_a_small_packet_and_are_labelled() {
        let mut corpus = chunk_text(
            "accepted.rs",
            "pub fn parcel_dispatch() {\n    deliver();\n}",
        );
        corpus.extend(chunk_text(
            "close.rs",
            "pub fn parcel_route() {\n    plan();\n}",
        ));
        corpus.extend(chunk_text(
            "second.rs",
            "pub fn parcel_label() {\n    print();\n}",
        ));
        corpus.extend(chunk_text(
            "third.rs",
            "pub fn parcel_weigh() {\n    scale();\n}",
        ));
        let chunk = |path: &str| corpus.iter().find(|c| c.path == path).unwrap().clone();
        let navigation = NavigationIndex::new_shared(std::iter::empty());
        let packet = build_packet_with_runners_up(
            &corpus,
            &[(chunk("accepted.rs"), 0.9)],
            &[
                (chunk("close.rs"), 0.45),
                (chunk("second.rs"), 0.4),
                (chunk("third.rs"), 0.36),
            ],
            "parcel dispatch",
            &navigation,
        );
        let shown: Vec<_> = packet
            .results
            .iter()
            .map(|r| (r.excerpt.path.as_str(), r.lower_confidence))
            .collect();
        assert_eq!(
            shown,
            [
                ("accepted.rs", false),
                ("close.rs", true),
                ("second.rs", true)
            ]
        );
        assert!(!packet.omitted, "nothing was cut for size");
        let text = packet.render_text();
        assert!(text.contains("accepted.rs:1-3 (whole file)\n"), "{text}");
        assert!(
            text.contains("close.rs:1-3 (whole file, possible match)\n"),
            "{text}"
        );
        assert!(!text.contains("omitted"), "{text}");

        // Accepted matches always come first and are never displaced.
        let full = build_packet_with_runners_up(
            &corpus,
            &[
                (chunk("accepted.rs"), 0.9),
                (chunk("second.rs"), 0.8),
                (chunk("third.rs"), 0.7),
            ],
            &[(chunk("close.rs"), 0.45)],
            "parcel dispatch",
            &navigation,
        );
        assert!(full.results.iter().all(|r| !r.lower_confidence));

        // A response that is already large keeps to what was accepted.
        let big = format!(
            "pub fn parcel_dispatch() {{\n{}}}",
            "    deliver_the_parcel_to_its_destination();\n".repeat(200)
        );
        let mut corpus = chunk_text("accepted.rs", &big);
        corpus.extend(chunk_text("close.rs", "pub fn parcel_route() {}"));
        let accepted = corpus[0].clone();
        let close = corpus
            .iter()
            .find(|c| c.path == "close.rs")
            .unwrap()
            .clone();
        let packet = build_packet_with_runners_up(
            &corpus,
            &[(accepted, 0.9)],
            &[(close, 0.45)],
            "parcel dispatch",
            &navigation,
        );
        assert_eq!(packet.results.len(), 1);
    }
    #[test]
    fn more_accepted_matches_than_slots_is_not_reported_as_cut_for_size() {
        let mut corpus = Vec::new();
        for index in 0..5 {
            corpus.extend(chunk_text(
                &format!("parcel{index}.rs"),
                "pub fn parcel_dispatch() {\n    deliver();\n}",
            ));
        }
        let winners: Vec<_> = corpus.iter().map(|chunk| (chunk.clone(), 0.9)).collect();
        let packet = build_packet(&corpus, &winners, "parcel dispatch");
        assert_eq!(packet.results.len(), RESULT_LIMIT);
        assert!(packet.truncated, "more matches exist than are shown");
        assert!(!packet.omitted);
        assert!(!packet.render_text().contains("omitted"));
    }
    #[test]
    fn a_match_in_the_doc_comment_returns_the_function_it_documents() {
        let mut lines = vec![
            "import { isRemoteAllowed } from './remote';".to_owned(),
            "".into(),
            "/**".into(),
            " * Infers the dimensions of a remote image after URL authorization.".into(),
            " */".into(),
            "export function inferRemoteSize(url: string): number {".into(),
        ];
        lines.extend((0..90).map(|line| format!("    step_{line}();")));
        lines.push("    if (!isRemoteAllowed(finalUrl)) throw new Error('blocked');".into());
        lines.push("}".into());
        let source = lines.join("\n");
        let mut corpus = chunk_text(
            "first.ts",
            "export function first(): number {\n    return 1;\n}",
        );
        corpus.extend(chunk_text("probe.ts", &source));
        let documented = corpus
            .iter()
            .find(|chunk| chunk.path == "probe.ts" && chunk.text.contains("Infers the dimensions"))
            .unwrap()
            .clone();
        assert!(documented.start_line < 6, "the chunk begins at the comment");
        let winners = [(corpus[0].clone(), 0.9), (documented, 0.8)];
        let packet = build_packet(
            &corpus,
            &winners,
            "infers dimensions of a remote image URL authorization",
        );
        let excerpt = &packet.results[1].excerpt;
        // The function it documents, beginning with that comment.
        assert_eq!((excerpt.start_line, excerpt.end_line), (3, 98));
        assert!(excerpt.text.starts_with("/**\n * Infers the dimensions"));
        assert!(excerpt.definition_complete && !excerpt.truncated);
        assert!(excerpt.text.contains("isRemoteAllowed(finalUrl)"));

        // Code between the matching line and a later declaration is its own evidence.
        let unrelated = "const remoteImageDimensions = 1;\nconsole.log(remoteImageDimensions);\nexport function other(): number {\n    return 2;\n}";
        let corpus = chunk_text("plain.ts", unrelated);
        let packet = build_packet(
            &corpus,
            &[(corpus[0].clone(), 0.9)],
            "remote image dimensions",
        );
        assert_eq!(packet.results[0].excerpt.start_line, 1);
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
            omitted: true,
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
        // The short same-file helper it calls directly follows it, so it comes
        // with the match instead of taking a supporting slot.
        let primary = &packet.results[0].excerpt;
        assert_eq!((primary.start_line, primary.end_line), (2, 7));
        assert_eq!(primary.definitions, 2);
        assert!(primary.text.ends_with("fn probe_source_video() {}"));
        assert_eq!(packet.related.len(), 1);
        assert_eq!(packet.related[0].excerpt.path, "src/export/overlay.rs");
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
