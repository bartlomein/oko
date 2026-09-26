//! Every use of a name, and the tests that exercise it: deterministic answers
//! from the snapshot, with no ranker involved.
//!
//! "Who calls X" and "tests for X" were answered with definitions and left
//! the agent to grep. Here the snapshot's text is scanned for the name as a
//! whole word, each hit is attributed to the definition enclosing it and
//! classified, test paths are set aside and counted, and test files are
//! paired by naming convention and by mention.
use crate::floor::{Pin, contains_word};
use crate::navigation::{DefinitionKind, NavigationIndex};
use crate::rails;
use crate::search::{self, Chunk};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

/// Rows shown before the rest is summarised as a count.
pub const MAX_ROWS: usize = 40;
pub const MAX_FILES: usize = 12;
const ROW_TEXT_BYTES: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UseKind {
    /// The name is invoked: `name(`.
    Call,
    /// An import, use, require or include line.
    Import,
    /// The name appears in a comment.
    Mention,
    /// Any other appearance: a type position, an argument, a property.
    Reference,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Use {
    pub path: String,
    pub line: usize,
    pub kind: UseKind,
    /// The definition the line sits in, qualified, when the parser knows it.
    pub enclosing: Option<String>,
    pub text: String,
    pub test: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usages {
    pub name: String,
    pub qualified: String,
    pub definition: Option<(String, usize)>,
    pub calls: usize,
    pub imports: usize,
    pub references: usize,
    pub mentions: usize,
    pub in_tests: usize,
    /// Hits in documentation files (Markdown, reStructuredText, plain text).
    pub in_docs: usize,
    /// Shown rows: non-test calls, imports and references, grouped by file.
    pub shown: Vec<Use>,
    pub omitted_rows: usize,
    pub omitted_files: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestMatch {
    pub path: String,
    /// `high` for a file named after the definition's file or name, `medium`
    /// for a file that mentions the name.
    pub confidence: &'static str,
    pub how: &'static str,
    /// Lines mentioning the name, with the enclosing test when known.
    pub lines: Vec<(usize, Option<String>)>,
}

struct Patterns {
    callers: Regex,
    import: Regex,
    comment: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        callers: Regex::new(
            r"(?i)\b(?:callers?|usages?|usage|call ?sites?|consumers|invocations?|uses of|calls to|references to)\b|\bwho\s+(?:calls|uses|invokes|references)\b|\bwhere\b[^\n]{0,60}\b(?:is|are|gets?|get)\s+(?:called|used|invoked|referenced|consumed)\b|\bwhere\s+(?:is|are)\b[^\n]{0,60}\b(?:called|used|invoked|referenced|consumed)\b|\b(?:all|every|each)\s+(?:the\s+)?(?:places?|sites?|locations?)\s+(?:that|which|where)\b[^.\n]{0,40}\b(?:calls?|uses?|invokes?)\b",
        )
        .unwrap(),
        import: Regex::new(r"^\s*(?:import\b|from\s+\S+\s+import\b|use\s+[A-Za-z_:]|require\s*\(|require\s+'|include\s+[A-Z]|extend\s+[A-Z]|using\s+|#include\b|export\s+\{|export\s+\*)").unwrap(),
        comment: Regex::new(r"^\s*(?://|#|/\*|\*|--|<!--|///|//!)").unwrap(),
    })
}

/// The question asks which definitions nothing uses.
pub fn asks_for_unused(question: &str) -> bool {
    static UNUSED: OnceLock<Regex> = OnceLock::new();
    UNUSED
        .get_or_init(|| {
            Regex::new(r"(?i)\b(?:dead\s+code|unused\s+(?:code|functions?|methods?|helpers?|definitions?|symbols?|types?|classes|exports?|private\s+\w+)|never\s+(?:called|used|referenced|invoked)|not\s+(?:used|called|referenced)\s+anywhere|(?:no|zero)\s+(?:callers|references|usages?|call\s+sites)|unreferenced|orphan(?:ed)?\s+(?:code|functions?|methods?|helpers?))\b")
                .unwrap()
        })
        .is_match(question)
}

/// The question asks who uses a name rather than what the name does.
pub fn asks_for_callers(question: &str) -> bool {
    patterns().callers.is_match(question)
}

/// A listing may stand alone only for a short question ("callers of X").
/// An issue-length text that mentions "references to" or "usage" wants
/// ranked code; the listing then accompanies it.
pub fn listing_can_stand_alone(question: &str) -> bool {
    question.len() <= 120
        && question.split_whitespace().count() <= 16
        && !asks_for_code_too(question)
}

/// The question asks for the definition or behaviour as well ("definition
/// and callers", "implementation and usage"): the listing then accompanies
/// the ranked code instead of replacing it.
pub fn asks_for_code_too(question: &str) -> bool {
    static CODE: OnceLock<Regex> = OnceLock::new();
    CODE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:definitions?|implementations?|implemented|implement|how\b|what\b|why\b|logic|explain|body|source)\b")
            .unwrap()
    })
    .is_match(question)
}

/// Lines of every file that mentions `name` somewhere: the substring test
/// over chunk text is the cheap part, and it rules out most files.
fn lines_by_path<'a>(
    corpus: &'a [Chunk],
    name: &str,
) -> BTreeMap<&'a str, BTreeMap<usize, &'a str>> {
    let mentioning: std::collections::HashSet<&str> = corpus
        .iter()
        .filter(|chunk| chunk.text.contains(name))
        .map(|chunk| chunk.path.as_str())
        .collect();
    let mut files: BTreeMap<&str, BTreeMap<usize, &str>> = BTreeMap::new();
    for chunk in corpus {
        if !mentioning.contains(chunk.path.as_str())
            || chunk.start_line == 0
            || chunk.text.split('\n').count() != chunk.end_line - chunk.start_line + 1
        {
            continue;
        }
        let lines = files.entry(chunk.path.as_str()).or_default();
        for (offset, text) in chunk.text.split('\n').enumerate() {
            lines.entry(chunk.start_line + offset).or_insert(text);
        }
    }
    files
}

/// The lines to scan for a pin: every line naming it, plus, for a Ruby
/// class, the association lines that refer to it without its constant.
fn lines_for<'a>(
    pin: &Pin,
    corpus: &'a [Chunk],
) -> (
    BTreeMap<&'a str, BTreeMap<usize, &'a str>>,
    Option<rails::Associations>,
) {
    let mut files = lines_by_path(corpus, &pin.name);
    let associations = (pin.path.ends_with(".rb") && pin.kind == DefinitionKind::Class)
        .then(|| rails::associations(&pin.name))
        .flatten();
    if let Some(associations) = &associations {
        for (path, lines) in lines_by_path(corpus, &associations.needle) {
            if !path.ends_with(".rb") {
                continue;
            }
            files.entry(path).or_default().extend(lines);
        }
    }
    (files, associations)
}

/// A line names the pin, directly or, in a Ruby file, through a Rails
/// association rule.
fn refers(
    path: &str,
    text: &str,
    name: &str,
    associations: Option<&rails::Associations>,
) -> Option<Option<rails::AssociationRule>> {
    if contains_word(text, name) {
        return Some(None);
    }
    if !path.ends_with(".rb") {
        return None;
    }
    associations.and_then(|a| a.matches(text)).map(Some)
}

/// Prose files: a name there is documentation, not a use.
fn is_docs_path(path: &str) -> bool {
    matches!(
        path.rsplit('.')
            .next()
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "mdx" | "markdown" | "rst" | "txt" | "adoc" | "asciidoc" | "html" | "htm")
    )
}

fn classify(line: &str, name: &str) -> UseKind {
    let p = patterns();
    if p.comment.is_match(line) {
        return UseKind::Mention;
    }
    if p.import.is_match(line) {
        return UseKind::Import;
    }
    // `name(`, `name!(`, `name (` and `name::<T>(`.
    let mut from = 0;
    while let Some(offset) = line[from..].find(name) {
        let start = from + offset;
        let end = start + name.len();
        let before_ok = line[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let rest = line[end..].trim_start_matches(['!', ' ']);
        let rest = rest
            .strip_prefix("::<")
            .map_or(rest, |r| r.split_once('>').map_or(r, |(_, r)| r));
        if before_ok && rest.starts_with('(') {
            return UseKind::Call;
        }
        from = end;
    }
    UseKind::Reference
}

fn trim_row(text: &str) -> String {
    let text = text.trim();
    if text.len() <= ROW_TEXT_BYTES {
        return text.to_owned();
    }
    let mut end = ROW_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Every whole-word use of `pin`'s name across the snapshot, except the
/// definition's own span.
pub fn usages(pin: &Pin, navigation: &NavigationIndex, corpus: &[Chunk]) -> Usages {
    let name = pin.name.as_str();
    let mut result = Usages {
        name: pin.name.clone(),
        qualified: pin.qualified.clone(),
        definition: Some((pin.path.clone(), pin.start_line)),
        ..Usages::default()
    };
    let (files, associations) = lines_for(pin, corpus);
    let mut per_file: Vec<(&str, Vec<Use>)> = Vec::new();
    for (path, lines) in &files {
        let test = search::is_test_path(path);
        let docs = !test && is_docs_path(path);
        let mut uses = Vec::new();
        for (number, text) in lines {
            if *path == pin.path
                && (pin.start_line..=pin.start_line.max(pin.start_line)).contains(number)
            {
                continue;
            }
            let Some(rule) = refers(path, text, name, associations.as_ref()) else {
                continue;
            };
            if test {
                result.in_tests += 1;
                continue;
            }
            if docs {
                result.in_docs += 1;
                continue;
            }
            let kind = if rule.is_some() {
                UseKind::Reference
            } else {
                classify(text, name)
            };
            match kind {
                UseKind::Call => result.calls += 1,
                UseKind::Import => result.imports += 1,
                UseKind::Reference => result.references += 1,
                UseKind::Mention => result.mentions += 1,
            }
            if kind == UseKind::Mention {
                continue;
            }
            uses.push(Use {
                path: (*path).to_owned(),
                line: *number,
                kind,
                enclosing: navigation
                    .definition(path, *number)
                    .map(|d| d.qualified.clone()),
                text: match rule {
                    Some(rule) => format!("{}  [{}]", trim_row(text), rule.label),
                    None => trim_row(text),
                },
                test,
            });
        }
        if !uses.is_empty() {
            per_file.push((path, uses));
        }
    }
    // Files with the most uses first; the definition's own file leads.
    per_file.sort_by(|a, b| {
        (b.0 == pin.path)
            .cmp(&(a.0 == pin.path))
            .then_with(|| b.1.len().cmp(&a.1.len()))
            .then_with(|| a.0.cmp(b.0))
    });
    let total_rows: usize = per_file.iter().map(|(_, uses)| uses.len()).sum();
    let mut shown = Vec::new();
    let mut files_shown = 0;
    for (_, uses) in per_file {
        if files_shown == MAX_FILES || shown.len() >= MAX_ROWS {
            result.omitted_files += 1;
            continue;
        }
        files_shown += 1;
        for item in uses {
            if shown.len() == MAX_ROWS {
                break;
            }
            shown.push(item);
        }
    }
    result.omitted_rows = total_rows - shown.len();
    result.shown = shown;
    result
}

/// One reference to a name inside one definition of one file: the row an
/// agent cites as `path:line` with the enclosing method's name.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependentRow {
    pub line: usize,
    /// The enclosing definition, qualified; `None` at file top level.
    pub enclosing: Option<String>,
    pub kind: UseKind,
    pub text: String,
    /// Uses inside this enclosing definition beyond the row shown.
    pub more: usize,
    /// The Rails association rule that matched, when the line does not
    /// spell the constant: `belongs_to / has_one`.
    pub rule: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependentFile {
    pub path: String,
    pub area: String,
    pub uses: usize,
    pub rows: Vec<DependentRow>,
}

/// Every production file that uses a name, one row per enclosing
/// definition, grouped by area: the answer to "what depends on X" that an
/// agent can cite file by file.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependents {
    pub name: String,
    pub qualified: String,
    pub definition: Option<(String, usize)>,
    /// Code files by area (first two path segments), areas in path order.
    pub areas: Vec<(String, Vec<DependentFile>)>,
    pub files: usize,
    pub uses: usize,
    /// Task, data and generated files: `(path, uses)`, counted, not listed.
    pub data_files: Vec<(String, usize)>,
    pub own_file_uses: usize,
    pub test_files: usize,
    pub tests: usize,
    /// Test files: `(path, uses, first line)`, files named after the
    /// definition first, then most uses.
    pub test_rows: Vec<(String, usize, usize)>,
    pub in_comments: usize,
}

/// Task, data, generated and script files: their uses are counted after the
/// code so the code rows are never displaced by a locale file or a rake task.
fn is_data_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let extension = lower.rsplit('.').next().unwrap_or("");
    matches!(
        extension,
        "yml" | "yaml" | "json" | "xml" | "csv" | "toml" | "lock" | "rake" | "sql" | "txt" | "svg"
    ) || lower.starts_with("script/")
        || lower.starts_with("scripts/")
        || lower.starts_with("db/")
        || lower.contains("/fixtures/")
        || lower.contains("/locales/")
        || lower.contains("/generated/")
        || lower.ends_with(".min.js")
}

fn area_of(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    match segments.len() {
        0 | 1 => ".".to_owned(),
        2 => segments[0].to_owned(),
        _ => format!("{}/{}", segments[0], segments[1]),
    }
}

pub fn dependents(pin: &Pin, navigation: &NavigationIndex, corpus: &[Chunk]) -> Dependents {
    let name = pin.name.as_str();
    let mut result = Dependents {
        name: pin.name.clone(),
        qualified: pin.qualified.clone(),
        definition: Some((pin.path.clone(), pin.start_line)),
        ..Dependents::default()
    };
    let mut by_area: BTreeMap<String, Vec<DependentFile>> = BTreeMap::new();
    let (files, associations) = lines_for(pin, corpus);
    for (path, lines) in &files {
        if is_docs_path(path) {
            continue;
        }
        let test = search::is_test_path(path);
        let own = *path == pin.path;
        let data = !test && is_data_path(path);
        let mut count = 0;
        let mut first = 0;
        // Rows keyed by the enclosing definition's start line, so one method
        // is one row however many times it names the symbol.
        let mut rows: BTreeMap<usize, DependentRow> = BTreeMap::new();
        for (number, text) in lines {
            if own && *number == pin.start_line {
                continue;
            }
            let Some(rule) = refers(path, text, name, associations.as_ref()) else {
                continue;
            };
            if patterns().comment.is_match(text) {
                result.in_comments += 1;
                continue;
            }
            count += 1;
            if first == 0 {
                first = *number;
            }
            if test || own || data {
                continue;
            }
            let enclosing = navigation.definition(path, *number);
            let key = enclosing.map_or(0, |d| d.start_line);
            match rows.get_mut(&key) {
                Some(row) => row.more += 1,
                None => {
                    rows.insert(
                        key,
                        DependentRow {
                            line: *number,
                            enclosing: enclosing.map(|d| d.qualified.clone()),
                            kind: if rule.is_some() {
                                UseKind::Reference
                            } else {
                                classify(text, name)
                            },
                            text: {
                                let mut row = trim_row(text);
                                if row.len() > DEPENDENT_TEXT_BYTES {
                                    let mut end = DEPENDENT_TEXT_BYTES;
                                    while !row.is_char_boundary(end) {
                                        end -= 1;
                                    }
                                    row.truncate(end);
                                    row.push('…');
                                }
                                row
                            },
                            more: 0,
                            rule: rule.map(|r| r.label),
                        },
                    );
                }
            }
        }
        if count == 0 {
            continue;
        }
        if test {
            result.test_files += 1;
            result.tests += count;
            result.test_rows.push(((*path).to_owned(), count, first));
        } else if own {
            result.own_file_uses = count;
        } else if data {
            result.data_files.push(((*path).to_owned(), count));
        } else {
            result.files += 1;
            result.uses += count;
            let area = area_of(path);
            by_area
                .entry(area.clone())
                .or_default()
                .push(DependentFile {
                    path: (*path).to_owned(),
                    area,
                    uses: count,
                    rows: rows.into_values().collect(),
                });
        }
    }
    result.areas = by_area.into_iter().collect();
    result
        .data_files
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let file_stem = stem(&pin.path);
    result.test_rows.sort_by(|a, b| {
        named_after(&b.0, file_stem, name)
            .cmp(&named_after(&a.0, file_stem, name))
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.0.cmp(&b.0))
    });
    result
}

/// The listing stays under this many bytes: every file keeps at least its
/// first row; extra rows go first, then whole areas are summarised.
const DEPENDENTS_BYTES: usize = 14_000;
/// Dependents rows show less of the line than a callers row: the path and
/// the enclosing definition are the point.
const DEPENDENT_TEXT_BYTES: usize = 90;
const DATA_FILES_SHOWN: usize = 6;
const TEST_FILES_SHOWN: usize = 12;

/// `mode: enumerate` and long callers lists: `path:line<TAB>Enclosing<TAB>text`
/// per enclosing definition, grouped by area with counts, code before data.
pub fn render_dependents(summary: &Dependents) -> String {
    let mut out = format!(
        "Files using {} — {} files, {} uses in code",
        summary.qualified, summary.files, summary.uses
    );
    if summary.own_file_uses > 0 {
        out.push_str(&format!("; {} in its own file", summary.own_file_uses));
    }
    if summary.test_files > 0 {
        out.push_str(&format!(
            "; {} test files ({} uses)",
            summary.test_files, summary.tests
        ));
    }
    out.push_str(". One row per enclosing definition: path:line, definition, line.\n");
    let mut rows_per_file = usize::MAX;
    let mut with_text = true;
    let mut body = String::new();
    // Shrink until the body fits: all rows, then three per file, then one,
    // then one without the line text (path:line and the definition are what
    // a citation needs); still over, the areas with the most files are
    // summarised first, since those are worth a `directory` search of their
    // own.
    loop {
        let mut sections: Vec<(&str, usize, String)> = Vec::new();
        for (area, files) in &summary.areas {
            let mut section = format!("{area} ({} files)\n", files.len());
            for file in files {
                let shown_rows = rows_per_file.min(file.rows.len());
                for (index, row) in file.rows.iter().take(shown_rows).enumerate() {
                    section.push_str(&format!("  {}:{}\t", file.path, row.line));
                    match &row.enclosing {
                        Some(name) => section.push_str(name),
                        None => section.push_str("(top level)"),
                    }
                    if with_text {
                        section.push('\t');
                        section.push_str(&row.text);
                    }
                    if let Some(rule) = row.rule {
                        section.push_str(&format!("  [{rule}]"));
                    }
                    // Every use beyond the rows shown, in one count on the
                    // file's last row.
                    let hidden = file.uses.saturating_sub(shown_rows);
                    if index + 1 == shown_rows && hidden > 0 {
                        section.push_str(&format!("  (+{hidden} more in this file)"));
                    }
                    section.push('\n');
                }
            }
            sections.push((area, files.len(), section));
        }
        let total: usize = sections.iter().map(|(_, _, s)| s.len()).sum();
        if out.len() + total <= DEPENDENTS_BYTES || (rows_per_file == 1 && !with_text) {
            let mut summarised: Vec<(&str, usize)> = Vec::new();
            let mut kept = total;
            let mut by_size: Vec<usize> = (0..sections.len()).collect();
            by_size.sort_by_key(|i| std::cmp::Reverse(sections[*i].1));
            let mut dropped = vec![false; sections.len()];
            for i in by_size {
                if out.len() + kept <= DEPENDENTS_BYTES {
                    break;
                }
                kept -= sections[i].2.len();
                dropped[i] = true;
                summarised.push((sections[i].0, sections[i].1));
            }
            body.clear();
            for (i, (_, _, section)) in sections.iter().enumerate() {
                if !dropped[i] {
                    body.push_str(section);
                }
            }
            if !summarised.is_empty() {
                summarised.sort();
                let list: Vec<String> = summarised
                    .iter()
                    .map(|(area, n)| format!("{area} ({n})"))
                    .collect();
                body.push_str(&format!(
                    "… {} more files under {}; pass `directory` for one of them.\n",
                    summarised.iter().map(|(_, n)| n).sum::<usize>(),
                    list.join(", ")
                ));
            }
            break;
        }
        match rows_per_file {
            usize::MAX => rows_per_file = 3,
            3 => rows_per_file = 1,
            _ => with_text = false,
        }
    }
    out.push_str(&body);
    if !summary.data_files.is_empty() {
        let shown: Vec<String> = summary
            .data_files
            .iter()
            .take(DATA_FILES_SHOWN)
            .map(|(path, n)| format!("{path} ({n})"))
            .collect();
        let uses: usize = summary.data_files.iter().map(|(_, n)| n).sum();
        out.push_str(&format!(
            "Task, data and script files ({} files, {} uses): {}",
            summary.data_files.len(),
            uses,
            shown.join(", ")
        ));
        if summary.data_files.len() > DATA_FILES_SHOWN {
            out.push_str(&format!(
                ", +{} more",
                summary.data_files.len() - DATA_FILES_SHOWN
            ));
        }
        out.push('\n');
    }
    if !summary.test_rows.is_empty() {
        out.push_str(&format!(
            "Specs and tests using {} ({} files, {} uses), named after it first:\n",
            summary.name, summary.test_files, summary.tests
        ));
        for (path, uses, first) in summary.test_rows.iter().take(TEST_FILES_SHOWN) {
            out.push_str(&format!("  {path}:{first}\t{uses}\n"));
        }
        if summary.test_rows.len() > TEST_FILES_SHOWN {
            out.push_str(&format!(
                "  … {} more test files; ask \"tests for {}\" for the ones named after it.\n",
                summary.test_rows.len() - TEST_FILES_SHOWN,
                summary.name
            ));
        }
    }
    out
}

/// Files that use a name, most first, with counts: the answer to "what
/// depends on X" in one line, and the shape of `mode: enumerate`.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedBy {
    pub name: String,
    pub qualified: String,
    /// Non-test files, most uses first: `(path, uses, first line)`.
    pub files: Vec<(String, usize, usize)>,
    pub uses: usize,
    pub test_files: usize,
    pub tests: usize,
}

/// A summary is offered on its own only when this many files use the name.
pub const USED_BY_MIN_FILES: usize = 4;
const USED_BY_SHOWN: usize = 10;
const ENUMERATE_FILES: usize = 40;

pub fn used_by(pin: &Pin, corpus: &[Chunk]) -> UsedBy {
    let name = pin.name.as_str();
    let mut result = UsedBy {
        name: pin.name.clone(),
        qualified: pin.qualified.clone(),
        ..UsedBy::default()
    };
    let (files, associations) = lines_for(pin, corpus);
    for (path, lines) in &files {
        // Dependents are other code files: not the definition's own file, not
        // documentation.
        if *path == pin.path || is_docs_path(path) {
            continue;
        }
        let mut count = 0;
        let mut first = 0;
        for (number, text) in lines {
            if patterns().comment.is_match(text)
                || refers(path, text, name, associations.as_ref()).is_none()
            {
                continue;
            }
            count += 1;
            if first == 0 {
                first = *number;
            }
        }
        if count == 0 {
            continue;
        }
        if search::is_test_path(path) {
            result.test_files += 1;
            result.tests += count;
        } else {
            result.uses += count;
            result.files.push(((*path).to_owned(), count, first));
        }
    }
    result
        .files
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    result
}

/// One line under a pinned definition: `Used by 41 files (386 uses): a.rb
/// (12), b.rb (9), … +33 more; 19 test files`. Nothing when few files use it.
pub fn render_used_by(summary: &UsedBy) -> Option<String> {
    if summary.files.len() < USED_BY_MIN_FILES {
        return None;
    }
    // `path:line (uses)`: the first use is a location an agent can cite.
    let shown: Vec<String> = summary
        .files
        .iter()
        .take(USED_BY_SHOWN)
        .map(|(path, count, first)| format!("{path}:{first} ({count})"))
        .collect();
    let more = summary.files.len().saturating_sub(USED_BY_SHOWN);
    let mut line = format!(
        "`{}` is used by {} files ({} uses): {}",
        summary.qualified,
        summary.files.len(),
        summary.uses,
        shown.join(", ")
    );
    if more > 0 {
        line.push_str(&format!(", +{more} more"));
    }
    if summary.test_files > 0 {
        line.push_str(&format!("; {} test files", summary.test_files));
    }
    line.push_str(". Ask \"who uses ");
    line.push_str(&summary.name);
    line.push_str("\" for every file with path:line and the enclosing definition.\n");
    Some(line)
}

/// `mode: enumerate`: every file that uses the name, one row each with the
/// count and first line, up to 40, then a count of the rest.
pub fn render_enumerate(summary: &UsedBy) -> String {
    let mut out = format!(
        "Files using {} — {} files, {} uses",
        summary.qualified,
        summary.files.len(),
        summary.uses
    );
    if summary.test_files > 0 {
        out.push_str(&format!(
            "; {} test files ({} uses) hidden",
            summary.test_files, summary.tests
        ));
    }
    out.push('\n');
    for (path, count, first) in summary.files.iter().take(ENUMERATE_FILES) {
        out.push_str(&format!("  {path}:{first}\t{count}\n"));
    }
    let more = summary.files.len().saturating_sub(ENUMERATE_FILES);
    if more > 0 {
        out.push_str(&format!("  … {more} more files\n"));
    }
    out
}

/// The rendered usages answer.
pub fn render_usages(usages: &Usages) -> String {
    let mut out = format!("Callers of {} — ", usages.qualified);
    let mut counts = Vec::new();
    for (count, what) in [
        (usages.calls, "call"),
        (usages.imports, "import"),
        (usages.references, "reference"),
    ] {
        if count > 0 {
            counts.push(format!(
                "{count} {what}{}",
                if count == 1 { "" } else { "s" }
            ));
        }
    }
    if counts.is_empty() {
        out.push_str("no uses found outside the definition");
    } else {
        out.push_str(&counts.join(", "));
    }
    let mut hidden = Vec::new();
    if usages.mentions > 0 {
        hidden.push(format!("{} in comments", usages.mentions));
    }
    if usages.in_tests > 0 {
        hidden.push(format!("{} in tests", usages.in_tests));
    }
    if usages.in_docs > 0 {
        hidden.push(format!("{} in docs", usages.in_docs));
    }
    if !hidden.is_empty() {
        out.push_str(&format!("; {} hidden", hidden.join(", ")));
    }
    out.push('\n');
    if let Some((path, line)) = &usages.definition {
        out.push_str(&format!("Defined at {path}:{line}\n"));
    }
    let mut last_path = "";
    for item in &usages.shown {
        if item.path != last_path {
            out.push_str(&format!("\n{}\n", item.path));
            last_path = &item.path;
        }
        let kind = match item.kind {
            UseKind::Call => "call",
            UseKind::Import => "import",
            UseKind::Reference => "ref",
            UseKind::Mention => "mention",
        };
        out.push_str(&format!(
            "  {}\t{kind}\t{}\t{}\n",
            item.line,
            item.enclosing.as_deref().unwrap_or("-"),
            item.text
        ));
    }
    if usages.omitted_rows > 0 {
        out.push_str(&format!(
            "\n{} more use{} in {} more file{} not shown.\n",
            usages.omitted_rows,
            if usages.omitted_rows == 1 { "" } else { "s" },
            usages.omitted_files,
            if usages.omitted_files == 1 { "" } else { "s" }
        ));
    }
    out
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split_once('.').map_or(name, |(stem, _)| stem)
}

/// A test path named after `stem` or `name`: `test_x.py`, `x_test.go`,
/// `x.test.ts`, `x_spec.rb`, `XTest.java`, `__tests__/x.ts`.
fn named_after(test_path: &str, file_stem: &str, name: &str) -> bool {
    let test_stem = stem(test_path).to_ascii_lowercase();
    let mut candidates = vec![file_stem.to_ascii_lowercase()];
    let lower = name.to_ascii_lowercase();
    if !candidates.contains(&lower) {
        candidates.push(lower);
    }
    candidates.iter().any(|c| {
        c.len() >= 3
            && (test_stem == *c
                || test_stem == format!("test_{c}")
                || test_stem == format!("{c}_test")
                || test_stem == format!("{c}_spec")
                || test_stem == format!("{c}test")
                || test_stem == format!("{c}tests")
                || test_stem == format!("{c}.test")
                || test_stem == format!("{c}.spec")
                || test_stem == format!("test{c}"))
    })
}

/// Test files for `pin`: named after its file or its name (high), or
/// mentioning the name (medium), each with the mentioning lines and their
/// enclosing test function.
pub fn tests_for(pin: &Pin, navigation: &NavigationIndex, corpus: &[Chunk]) -> Vec<TestMatch> {
    let files = lines_by_path(corpus, &pin.name);
    let file_stem = stem(&pin.path);
    let mut matches = Vec::new();
    for (path, lines) in &files {
        if !search::is_test_path(path) || *path == pin.path {
            continue;
        }
        let named = named_after(path, file_stem, &pin.name);
        let mentions: Vec<(usize, Option<String>)> = lines
            .iter()
            .filter(|(_, text)| contains_word(text, &pin.name))
            .map(|(number, _)| {
                (
                    *number,
                    navigation
                        .definition(path, *number)
                        .map(|d| d.qualified.clone()),
                )
            })
            .collect();
        if !named && mentions.is_empty() {
            continue;
        }
        matches.push(TestMatch {
            path: (*path).to_owned(),
            confidence: if named { "high" } else { "medium" },
            how: match (named, mentions.is_empty()) {
                (true, false) => "named after it, mentions it",
                (true, true) => "named after it",
                _ => "mentions it",
            },
            lines: mentions,
        });
    }
    matches.sort_by(|a, b| {
        (b.confidence == "high")
            .cmp(&(a.confidence == "high"))
            .then_with(|| b.lines.len().cmp(&a.lines.len()))
            .then_with(|| a.path.cmp(&b.path))
    });
    matches.truncate(8);
    matches
}

pub fn render_tests(pin: &Pin, matches: &[TestMatch]) -> String {
    if matches.is_empty() {
        return format!(
            "Tests for {}: no test file is named after it or mentions it.\n",
            pin.qualified
        );
    }
    let mut out = format!("Tests for {}:\n", pin.qualified);
    for found in matches {
        let mut lines: Vec<String> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (number, enclosing) in found.lines.iter().take(6) {
            match enclosing {
                Some(name) if seen.insert(name.clone()) => {
                    lines.push(format!("{name} (L{number})"))
                }
                Some(_) => {}
                None => lines.push(format!("L{number}")),
            }
        }
        // The row leads with `path:line` of the first mention, so the file
        // can be cited as a location; a file only named after the
        // definition has no line to give.
        let location = match found.lines.first() {
            Some((number, _)) => format!("{}:{}", found.path, number),
            None => found.path.clone(),
        };
        out.push_str(&format!(
            "  {} — {}, {}{}\n",
            location,
            found.how,
            found.confidence,
            if lines.is_empty() {
                String::new()
            } else {
                format!(": {}", lines.join(", "))
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::floor;
    use crate::navigation::NavigationPreparer;
    use std::sync::Arc;

    fn corpus(files: &[(&str, &str)]) -> (Vec<Chunk>, NavigationIndex) {
        let mut preparer = NavigationPreparer::default();
        let mut chunks = vec![];
        let mut facts = vec![];
        for (path, text) in files {
            chunks.extend(search::chunk_text(path, text));
            facts.push((*path, Arc::new(preparer.prepare(path, text))));
        }
        (chunks, NavigationIndex::new_shared(facts))
    }

    #[test]
    fn dependents_keep_every_code_file_and_demote_data_files() {
        let (chunks, index) = corpus(&[
            (
                "app/models/upload.rb",
                "class Upload < ActiveRecord::Base\n  def self.find_one\n    Upload.first\n  end\nend\n",
            ),
            (
                "app/controllers/metadata_controller.rb",
                "class MetadataController\n  def default_manifest\n    icon = Upload.find_by(id: 1)\n    Upload.count\n  end\n  def other\n    Upload.last\n  end\nend\n",
            ),
            (
                "lib/email/styles.rb",
                "module Email\n  class Styles\n    def stripped_secure_image_uploads\n      # Upload is mentioned here\n      Upload.secure\n    end\n  end\nend\n",
            ),
            (
                "app/models/user_profile.rb",
                "class UserProfile < ActiveRecord::Base\n  belongs_to :card_background_upload, class_name: \"Upload\"\n  has_many :uploads\n  def bg\n    upload_id\n  end\nend\n",
            ),
            (
                "lib/tasks/uploads.rake",
                "task :x do\n  Upload.find_each { }\n  Upload.count\nend\n",
            ),
            ("config/locales/client.en.yml", "en:\n  upload: Upload\n"),
            (
                "spec/models/upload_spec.rb",
                "describe Upload do\n  Upload.new\nend\n",
            ),
        ]);
        let pin = floor::pins_for_names(&["Upload".into()], &index, &chunks)
            .pins
            .remove(0);
        let summary = dependents(&pin, &index, &chunks);
        assert_eq!((summary.files, summary.uses), (3, 6));
        assert_eq!(summary.own_file_uses, 1);
        assert_eq!((summary.test_files, summary.tests), (1, 2));
        assert_eq!(summary.in_comments, 1);
        assert_eq!(
            summary.data_files,
            vec![
                ("lib/tasks/uploads.rake".to_owned(), 2),
                ("config/locales/client.en.yml".to_owned(), 1)
            ]
        );
        let text = render_dependents(&summary);
        assert!(
            text.starts_with("Files using Upload — 3 files, 6 uses in code; 1 in its own file; 1 test files (2 uses). One row per enclosing definition: path:line, definition, line.\napp/controllers (1 files)\n  app/controllers/metadata_controller.rb:3\tMetadataController.default_manifest\ticon = Upload.find_by(id: 1)\n  app/controllers/metadata_controller.rb:7\tMetadataController.other\tUpload.last  (+1 more in this file)\napp/models (1 files)\n  app/models/user_profile.rb:2\tUserProfile\tbelongs_to :card_background_upload, class_name: \"Upload\"  (+1 more in this file)\nlib/email (1 files)\n  lib/email/styles.rb:5\tEmail.Styles.stripped_secure_image_uploads\tUpload.secure\nTask, data and script files (2 files, 3 uses): lib/tasks/uploads.rake (2), config/locales/client.en.yml (1)\nSpecs and tests using Upload (1 files, 2 uses), named after it first:\n  spec/models/upload_spec.rb:1\t2\n"),
            "{text}"
        );
    }

    #[test]
    fn unused_definitions_are_the_names_no_other_code_uses() {
        let (chunks, index) = corpus(&[
            (
                "gin.go",
                "package gin\n\nfunc New() *Engine {\n    return build()\n}\n\nfunc build() *Engine { return nil }\n\n// readNthLine is a helper.\nfunc readNthLine(n int) string { return \"\" }\n\nfunc parseIP(s string) string { return s }\n\nfunc Exported() {}\n\nfunc main() {}\n",
            ),
            (
                "gin_test.go",
                "package gin\n\nfunc TestParse(t *testing.T) { parseIP(\"x\") }\n",
            ),
        ]);
        let summary = unused(&index, &chunks, &[], "");
        let names =
            |list: &[UnusedDefinition]| list.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&summary.private), ["readNthLine"]);
        assert_eq!(names(&summary.exported), ["New", "Exported"]);
        assert_eq!(names(&summary.tests_only), ["parseIP"]);
        assert_eq!(summary.checked, 5);
        let text = render_unused(&summary, "the workspace", "");
        assert!(
            text.starts_with("Unused definitions in the workspace — 3 of 5 checked have no use in non-test code of the workspace outside their own definition; 1 used only by tests\nprivate (1):\n  gin.go:10\tfunction readNthLine\nexported (other repositories may use them) (2):\n  gin.go:3\tfunction New\n  gin.go:14\tfunction Exported\nused only by tests, private first (1):\n  gin.go:12\tfunction parseIP (1 test uses)\n"),
            "{text}"
        );
        // Named checks look at those names only.
        let summary = unused(&index, &chunks, &["build".into(), "readNthLine".into()], "");
        assert_eq!(summary.checked, 2);
        assert_eq!(names(&summary.private), ["readNthLine"]);
    }

    #[test]
    fn unused_questions_are_recognised() {
        for q in [
            "dead code in the gin package",
            "unused functions in src/",
            "which helpers are never called",
            "unreferenced private methods",
            "functions with no callers",
        ] {
            assert!(asks_for_unused(q), "{q}");
        }
        for q in [
            "callers of parseIP",
            "how does the router work",
            "unused import warning in build",
        ] {
            assert!(!asks_for_unused(q), "{q}");
        }
    }

    #[test]
    fn callers_questions_are_recognised() {
        for q in [
            "callers of wsgi_app",
            "who calls dispatch_request",
            "where is Context.Next called from",
            "where Context.Next is called from internal callers non-test",
            "all the places that use inferRemoteSize",
            "usages of `Upload`",
            "sanitizePathChars callers",
            "calls to debugPrintError function usages",
            "mockFileSystem type usage in gin",
        ] {
            assert!(asks_for_callers(q), "{q}");
        }
        // "Referenced by posts" and "used by Upload" describe the code, not a listing.
        for q in [
            "Upload model that represents an uploaded file referenced by posts",
            "HasUrl concern module used by Upload",
        ] {
            assert!(!asks_for_callers(q), "{q}");
        }
        assert!(asks_for_code_too(
            "renderToHTMLOrFlight app render function definition and callers"
        ));
        assert!(!asks_for_code_too("sanitizePathChars callers"));
        assert!(listing_can_stand_alone("sanitizePathChars callers"));
        assert!(!listing_can_stand_alone(
            "When I run the migration the references to the old column are not updated and the usage of `rename_field` in the admin breaks with a KeyError; see the traceback below and the model definition"
        ));
        for q in [
            "wsgi_app method definition",
            "how does the router match dynamic segments",
            "renderToHTMLOrFlight in app-render",
        ] {
            assert!(!asks_for_callers(q), "{q}");
        }
    }

    #[test]
    fn usages_are_attributed_classified_and_tests_hidden() {
        let (chunks, index) = corpus(&[
            (
                "src/flask/app.py",
                "class Flask:\n    def wsgi_app(self, environ):\n        return 1\n\n    def __call__(self, environ):\n        # wsgi_app does the work\n        return self.wsgi_app(environ)\n",
            ),
            (
                "src/flask/testing.py",
                "from .app import wsgi_app\n\ndef run(app):\n    fn = app.wsgi_app\n    return fn\n",
            ),
            (
                "tests/test_basic.py",
                "def test_wsgi_app(app):\n    assert app.wsgi_app(None) == 1\n",
            ),
            (
                "docs/quickstart.rst",
                "Wrap ``app.wsgi_app`` to add middleware.\n",
            ),
        ]);
        let found = floor::floor("callers of wsgi_app", &index, &chunks);
        let pin = &found.pins[0];
        let uses = usages(pin, &index, &chunks);
        assert_eq!(
            (
                uses.calls,
                uses.imports,
                uses.references,
                uses.mentions,
                uses.in_tests
            ),
            (1, 1, 1, 1, 1)
        );
        let rows: Vec<_> = uses
            .shown
            .iter()
            .map(|u| (u.path.as_str(), u.line, u.kind, u.enclosing.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                ("src/flask/app.py", 7, UseKind::Call, Some("Flask.__call__")),
                ("src/flask/testing.py", 1, UseKind::Import, None),
                ("src/flask/testing.py", 4, UseKind::Reference, Some("run")),
            ]
        );
        let text = render_usages(&uses);
        assert_eq!(uses.in_docs, 1);
        assert!(text.starts_with("Callers of Flask.wsgi_app — 1 call, 1 import, 1 reference; 1 in comments, 1 in tests, 1 in docs hidden\nDefined at src/flask/app.py:2\n"), "{text}");
        assert!(
            text.contains("  7\tcall\tFlask.__call__\treturn self.wsgi_app(environ)\n"),
            "{text}"
        );

        let tests = tests_for(pin, &index, &chunks);
        assert_eq!(tests.len(), 1);
        assert_eq!(
            (tests[0].path.as_str(), tests[0].confidence, tests[0].how),
            ("tests/test_basic.py", "medium", "mentions it")
        );
        let text = render_tests(pin, &tests);
        assert!(
            text.contains("tests/test_basic.py:2 — mentions it, medium: test_wsgi_app (L2)"),
            "{text}"
        );
    }

    #[test]
    fn used_by_counts_files_and_renders_a_summary_only_when_widely_used() {
        let mut files: Vec<(String, String)> = vec![(
            "app/models/upload.rb".into(),
            "class Upload < ActiveRecord::Base\n  def url; end\nend\n".into(),
        )];
        for i in 0..5 {
            files.push((format!("app/models/user{i}.rb"), format!("class User{i}\n  belongs_to :avatar, class_name: 'Upload'\n  def avatar_upload; Upload.find(1); end\nend\n")));
        }
        files.push((
            "spec/models/upload_spec.rb".into(),
            "describe Upload do\n  it { Upload.new }\nend\n".into(),
        ));
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        let (chunks, index) = corpus(&refs);
        let found = floor::floor("Upload model", &index, &chunks);
        let summary = used_by(&found.pins[0], &chunks);
        assert_eq!(
            (
                summary.files.len(),
                summary.uses,
                summary.test_files,
                summary.tests
            ),
            (5, 10, 1, 2)
        );
        let line = render_used_by(&summary).unwrap();
        assert!(
            line.starts_with("`Upload` is used by 5 files (10 uses): app/models/user0.rb:2 (2), "),
            "{line}"
        );
        assert!(
            line.contains("; 1 test files. Ask \"who uses Upload\" for every file with path:line and the enclosing definition."),
            "{line}"
        );
        let listing = render_enumerate(&summary);
        assert!(listing.starts_with("Files using Upload — 5 files, 10 uses; 1 test files (2 uses) hidden\n  app/models/user0.rb:2\t2\n"), "{listing}");
        // Few users: no summary line.
        let (chunks, index) = corpus(&refs[..2]);
        let found = floor::floor("Upload model", &index, &chunks);
        assert!(render_used_by(&used_by(&found.pins[0], &chunks)).is_none());
    }

    #[test]
    fn test_files_named_after_the_source_file_rank_high() {
        let (chunks, index) = corpus(&[
            (
                "src/probe.ts",
                "export function inferRemoteSize() { return 1; }\n",
            ),
            (
                "src/probe.test.ts",
                "import { inferRemoteSize } from './probe';\ndescribe('probe', () => { it('sizes', () => inferRemoteSize()); });\n",
            ),
            ("test/other.test.ts", "it('x', () => 1);\n"),
        ]);
        let found = floor::floor("tests for inferRemoteSize", &index, &chunks);
        let tests = tests_for(&found.pins[0], &index, &chunks);
        assert_eq!(tests.len(), 1);
        assert_eq!(
            (tests[0].path.as_str(), tests[0].confidence),
            ("src/probe.test.ts", "high")
        );
        assert!(named_after("tests/test_app.py", "app", "Flask"));
        assert!(named_after(
            "src/test/java/io/javalin/RoutingTest.java",
            "routing",
            "Routing"
        ));
        assert!(!named_after("tests/test_basic.py", "app", "Flask"));
    }
}

/// A definition nothing else uses.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnusedDefinition {
    pub name: String,
    pub qualified: String,
    pub path: String,
    pub line: usize,
    pub kind: DefinitionKind,
    pub exported: bool,
    /// Uses in test files only.
    pub tests: usize,
}

/// `mode: unused`: the definitions of the searched directory whose name
/// appears nowhere in non-test code outside the definition itself.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unused {
    pub checked: usize,
    /// Not exported: nothing in the repository uses them.
    pub private: Vec<UnusedDefinition>,
    /// Exported: nothing in this repository uses them; other repositories may.
    pub exported: Vec<UnusedDefinition>,
    /// Used by test files only.
    pub tests_only: Vec<UnusedDefinition>,
}

/// Names a language runtime or framework calls without a reference in the
/// repository, so their absence from the code means nothing.
const ENTRY_POINTS: &[&str] = &[
    "main",
    "init",
    "new",
    "String",
    "Error",
    "ServeHTTP",
    "MarshalJSON",
    "UnmarshalJSON",
    "MarshalText",
    "UnmarshalText",
    "Close",
    "Read",
    "Write",
    "Len",
    "Less",
    "Swap",
    "Reset",
    "setUp",
    "tearDown",
    "setup",
    "teardown",
    "__init__",
    "__str__",
    "__repr__",
    "__eq__",
    "__hash__",
    "__enter__",
    "__exit__",
    "__call__",
    "__iter__",
    "__next__",
    "__len__",
    "__getitem__",
    "__setitem__",
    "initialize",
    "to_s",
    "inspect",
    "call",
    "perform",
    "fmt",
    "drop",
    "default",
    "from",
    "into",
    "clone",
    "eq",
    "hash",
    "deref",
    "index",
    "run",
    "handle",
    "toString",
    "equals",
    "hashCode",
    "compareTo",
    "invoke",
    "apply",
    "accept",
    "get",
    "set",
];

/// Occurrences of a name beyond this are not recorded: it is plainly used.
const USE_CAP: usize = 96;
const UNUSED_SHOWN: usize = 60;
/// Exported names only tests use are a footnote: a library's public API
/// looks like this, so a few rows and a count are enough.
const EXPORTED_TESTS_ONLY_SHOWN: usize = 12;

/// `prefix` limits the candidates to one directory (`app/models/`), with a
/// trailing slash; uses are counted over the whole corpus given.
pub fn unused(
    navigation: &NavigationIndex,
    corpus: &[Chunk],
    only: &[String],
    prefix: &str,
) -> Unused {
    let only: HashSet<&str> = only
        .iter()
        .map(|n| n.rsplit(['.', ':', '#']).next().unwrap_or(n))
        .collect();
    // Candidates: definitions in non-test code with a name worth checking.
    let candidates: Vec<(&str, &crate::navigation::Definition)> = navigation
        .all_definitions()
        .filter(|(path, d)| {
            path.starts_with(prefix)
                && !search::is_test_path(path)
                && d.name.len() >= 3
                && !ENTRY_POINTS.contains(&d.name.as_str())
                && !d.name.starts_with("Test")
                && !d.name.starts_with("test_")
                && (only.is_empty() || only.contains(d.name.as_str()))
        })
        .collect();
    let names: HashSet<&str> = candidates.iter().map(|(_, d)| d.name.as_str()).collect();
    // One pass over the code: where each candidate name occurs, capped.
    let mut occurrences: HashMap<&str, Vec<(&str, usize)>> = HashMap::new();
    for chunk in corpus {
        if is_docs_path(&chunk.path)
            || chunk.start_line == 0
            || chunk.text.split('\n').count() != chunk.end_line - chunk.start_line + 1
        {
            continue;
        }
        for (offset, line) in chunk.text.split('\n').enumerate() {
            if patterns().comment.is_match(line) {
                continue;
            }
            for word in line
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|w| !w.is_empty() && !w.starts_with(|c: char| c.is_ascii_digit()))
            {
                if let Some(name) = names.get(word) {
                    let seen = occurrences.entry(name).or_default();
                    if seen.len() < USE_CAP {
                        seen.push((chunk.path.as_str(), chunk.start_line + offset));
                    }
                }
            }
        }
    }
    let mut result = Unused {
        checked: candidates.len(),
        ..Unused::default()
    };
    for (path, d) in candidates {
        let Some(seen) = occurrences.get(d.name.as_str()) else {
            continue;
        };
        if seen.len() >= USE_CAP {
            continue;
        }
        let mut seen = seen.clone();
        seen.sort_unstable();
        seen.dedup();
        let (mut code, mut tests) = (0, 0);
        for (at, line) in seen {
            // The definition's own lines, including its doc comment, do not count.
            if at == path && (d.start_line..=d.end_line).contains(&line) {
                continue;
            }
            if search::is_test_path(at) {
                tests += 1;
            } else {
                code += 1;
            }
        }
        if code > 0 {
            continue;
        }
        let entry = UnusedDefinition {
            name: d.name.clone(),
            qualified: d.qualified.clone(),
            path: path.to_owned(),
            line: d.start_line,
            kind: d.kind,
            exported: d.exported(),
            tests,
        };
        if tests > 0 {
            result.tests_only.push(entry);
        } else if d.exported() {
            result.exported.push(entry);
        } else {
            result.private.push(entry);
        }
    }
    for list in [&mut result.private, &mut result.exported] {
        list.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    }
    // A private name only tests use is dead in production; a public one is
    // an API without internal callers, listed after.
    result.tests_only.sort_by(|a, b| {
        a.exported
            .cmp(&b.exported)
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });
    result
}

fn kind_word(kind: DefinitionKind) -> &'static str {
    match kind {
        DefinitionKind::Function => "function",
        DefinitionKind::Constant => "constant",
        DefinitionKind::Type => "type",
        DefinitionKind::Class => "class",
        DefinitionKind::Method => "method",
        DefinitionKind::Module => "module",
    }
}

/// The unused answer: private names first, then exported, then names only
/// tests use; each row `path:line<TAB>kind qualified`.
pub fn render_unused(summary: &Unused, scope: &str, prefix: &str) -> String {
    let total = summary.private.len() + summary.exported.len();
    let mut out = format!(
        "Unused definitions in {scope} — {total} of {} checked have no use in non-test code of the workspace outside their own definition",
        summary.checked
    );
    if !summary.tests_only.is_empty() {
        out.push_str(&format!(
            "; {} used only by tests",
            summary.tests_only.len()
        ));
    }
    out.push('\n');
    let mut shown = 0;
    for (label, list) in [
        ("private", &summary.private),
        (
            "exported (other repositories may use them)",
            &summary.exported,
        ),
        ("used only by tests, private first", &summary.tests_only),
    ] {
        if list.is_empty() {
            continue;
        }
        out.push_str(&format!("{label} ({}):\n", list.len()));
        let mut exported_tests_only = 0;
        for entry in list {
            if entry.tests > 0 && entry.exported {
                exported_tests_only += 1;
            }
            if shown >= UNUSED_SHOWN || exported_tests_only > EXPORTED_TESTS_ONLY_SHOWN {
                break;
            }
            shown += 1;
            out.push_str(&format!(
                "  {}:{}\t{} {}",
                entry.path.strip_prefix(prefix).unwrap_or(&entry.path),
                entry.line,
                kind_word(entry.kind),
                entry.qualified
            ));
            if entry.tests > 0 {
                out.push_str(&format!(" ({} test uses)", entry.tests));
            }
            out.push('\n');
        }
    }
    let more = (total + summary.tests_only.len()).saturating_sub(shown);
    if more > 0 {
        out.push_str(&format!("  … {more} more\n"));
    }
    out.push_str("Checked by name over the indexed code: a method that implements an interface, a name used through reflection, a route or a template, or a public API can look unused. Confirm before deleting.\n");
    out
}
