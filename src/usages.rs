//! Every use of a name, and the tests that exercise it: deterministic answers
//! from the snapshot, with no ranker involved.
//!
//! "Who calls X" and "tests for X" were answered with definitions and left
//! the agent to grep. Here the snapshot's text is scanned for the name as a
//! whole word, each hit is attributed to the definition enclosing it and
//! classified, test paths are set aside and counted, and test files are
//! paired by naming convention and by mention.
use crate::floor::{Pin, contains_word};
use crate::navigation::NavigationIndex;
use crate::search::{self, Chunk};
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;
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
            r"(?i)\b(?:callers?|usages?|call ?sites?|references?|uses|users|consumers|invocations?)\s+(?:of|to|for)\b|\bwho\s+(?:calls|uses|invokes|references)\b|\bwhere\s+(?:is|are|it's|its)\b[^\n]{0,60}\b(?:called|used|invoked|referenced|consumed)\b|\b(?:all|every|each)\s+(?:the\s+)?(?:places?|sites?|locations?)\s+(?:that|which|where)\b[^.\n]{0,40}\b(?:calls?|uses?|invokes?)\b",
        )
        .unwrap(),
        import: Regex::new(r"^\s*(?:import\b|from\s+\S+\s+import\b|use\s+[A-Za-z_:]|require\s*\(|require\s+'|include\s+[A-Z]|extend\s+[A-Z]|using\s+|#include\b|export\s+\{|export\s+\*)").unwrap(),
        comment: Regex::new(r"^\s*(?://|#|/\*|\*|--|<!--|///|//!)").unwrap(),
    })
}

/// The question asks who uses a name rather than what the name does.
pub fn asks_for_callers(question: &str) -> bool {
    patterns().callers.is_match(question)
}

fn lines_by_path(corpus: &[Chunk]) -> BTreeMap<&str, BTreeMap<usize, &str>> {
    let mut files: BTreeMap<&str, BTreeMap<usize, &str>> = BTreeMap::new();
    for chunk in corpus {
        if chunk.start_line == 0
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
    let files = lines_by_path(corpus);
    let mut per_file: Vec<(&str, Vec<Use>)> = Vec::new();
    for (path, lines) in &files {
        if !lines.values().any(|line| line.contains(name)) {
            continue;
        }
        let test = search::is_test_path(path);
        let docs = !test && is_docs_path(path);
        let mut uses = Vec::new();
        for (number, text) in lines {
            if *path == pin.path
                && (pin.start_line..=pin.start_line.max(pin.start_line)).contains(number)
            {
                continue;
            }
            if !contains_word(text, name) {
                continue;
            }
            if test {
                result.in_tests += 1;
                continue;
            }
            if docs {
                result.in_docs += 1;
                continue;
            }
            let kind = classify(text, name);
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
                text: trim_row(text),
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
    let files = lines_by_path(corpus);
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
        out.push_str(&format!(
            "  {} — {}, {}{}\n",
            found.path,
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
    fn callers_questions_are_recognised() {
        for q in [
            "callers of wsgi_app",
            "who calls dispatch_request",
            "where is Context.Next called from",
            "all the places that use inferRemoteSize",
            "usages of `Upload`",
        ] {
            assert!(asks_for_callers(q), "{q}");
        }
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
            text.contains("tests/test_basic.py — mentions it, medium: test_wsgi_app (L2)"),
            "{text}"
        );
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
