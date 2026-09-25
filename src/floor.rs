//! The exact-name floor: a definition named in the question is always a
//! candidate, and is shown even when the relevance ranker rejects it.
//!
//! Agents ask for code by name far more often than by behaviour, and a
//! one-line definition loses a keyword shortlist to files that repeat the
//! name. The index knows every definition; this module finds the ones the
//! question names, chooses among same-named definitions with the question's
//! own hints, and turns each into a whole-definition chunk to pin.
use crate::navigation::{DefinitionKind, DefinitionRef, NavigationIndex};
use crate::search::{self, Chunk};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

/// At most this many definitions are pinned per search.
pub const MAX_PINS: usize = 3;
/// A leaf name defined this many times outside tests is common (`render`,
/// `execute`, `Page`): pinning one needs a container or path hint.
pub const COMMON_NAME_DEFINITIONS: usize = 5;
/// Identifiers considered per question, in order of appearance.
const MAX_IDENTIFIERS: usize = 8;
/// Same-named definitions listed beside the pinned one.
const ALSO_DEFINED: usize = 3;

/// A definition the question names, as a chunk to rank and show.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub name: String,
    pub qualified: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub kind: DefinitionKind,
    #[serde(skip)]
    pub chunk: Chunk,
}

impl Pin {
    pub fn is(&self, chunk: &Chunk) -> bool {
        chunk.path == self.path
            && chunk.start_line == self.start_line
            && chunk.end_line == self.end_line
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Floor {
    /// Identifier-shaped words found in the question.
    pub identifiers: Vec<String>,
    pub pins: Vec<Pin>,
    /// What the agent cannot see from the excerpts: other definitions of a
    /// pinned name, and common names that needed a hint.
    pub notes: Vec<String>,
}

struct Patterns {
    token: Regex,
    backticked: Regex,
    camel: Regex,
    pascal: Regex,
}
fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        token: Regex::new(r"[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\.|#)[A-Za-z_][A-Za-z0-9_]*)*").unwrap(),
        backticked: Regex::new(r"`([^`\n]{1,80})`").unwrap(),
        camel: Regex::new(r"[a-z0-9][A-Z]").unwrap(),
        pascal: Regex::new(r"^[A-Z][a-z0-9]+[A-Z]").unwrap(),
    })
}

/// Words that follow a name when the name is a symbol: "Upload model".
const CODE_NOUNS: &[&str] = &[
    "class",
    "trait",
    "struct",
    "enum",
    "interface",
    "module",
    "model",
    "type",
    "component",
    "hook",
    "middleware",
    "handler",
    "service",
    "controller",
    "helper",
    "mixin",
    "decorator",
    "macro",
    "function",
    "method",
    "object",
    "impl",
    "subclass",
    "superclass",
    "constructor",
    "record",
    "error",
    "exception",
    "job",
    "serializer",
    "validator",
    "mailer",
    "worker",
];
/// Frameworks, languages and formats named in questions: not code to pin.
const STOP: &[&str] = &[
    "ActiveRecord",
    "GitHub",
    "GitLab",
    "JavaScript",
    "TypeScript",
    "CoffeeScript",
    "WebSocket",
    "WebSockets",
    "GraphQL",
    "PostgreSQL",
    "MySQL",
    "SQLite",
    "MongoDB",
    "OpenAPI",
    "OAuth",
    "JWT",
    "JSON",
    "YAML",
    "TOML",
    "HTML",
    "CSS",
    "HTTP",
    "HTTPS",
    "URL",
    "URI",
    "API",
    "REST",
    "ReactDOM",
    "NextJS",
    "NodeJS",
    "DevTools",
    "iOS",
    "macOS",
    "WebAssembly",
    "WebKit",
    "PostCSS",
    "TailwindCSS",
    "ESLint",
    "TypeORM",
    "SQLAlchemy",
    "ActiveJob",
    "ActiveSupport",
    "ActiveModel",
    "ActionController",
    "ActionView",
    "ActionMailer",
    "ActiveStorage",
    "RSpec",
    "MiniTest",
    "PyTest",
    "JUnit",
    "GoLang",
    "RustLang",
    "StackOverflow",
    "README",
    "TODO",
];
/// Sentence words agents capitalize; never a symbol on their own.
const PROSE: &[&str] = &[
    "The",
    "This",
    "That",
    "These",
    "Those",
    "For",
    "From",
    "With",
    "Without",
    "And",
    "Or",
    "But",
    "In",
    "On",
    "At",
    "To",
    "Of",
    "If",
    "Is",
    "Are",
    "Does",
    "Do",
    "Can",
    "Should",
    "Must",
    "Will",
    "Need",
    "Please",
    "Trace",
    "Locate",
    "Return",
    "Find",
    "Show",
    "Explain",
    "List",
    "How",
    "What",
    "Where",
    "Which",
    "Who",
    "Why",
    "When",
    "Describe",
    "Identify",
    "Compare",
    "Check",
    "Look",
    "Get",
    "Give",
    "Include",
    "Read",
    "Search",
    "Follow",
    "Map",
    "Walk",
    "Note",
    "Also",
    "Then",
    "Provide",
    "Focus",
    "Start",
    "Determine",
    "Implementation",
    "Definition",
    "Code",
    "File",
    "Files",
    "Test",
    "Tests",
    "Data",
    "Method",
    "Methods",
    "Function",
    "Functions",
    "Class",
    "Classes",
    "Module",
    "Modules",
    "Config",
    "Default",
    "Main",
    "Base",
    "Core",
    "Client",
    "Request",
    "Response",
    "Route",
    "Routes",
    "Router",
    "Handler",
    "Handlers",
    "Model",
    "Models",
    "View",
    "Views",
    "Controller",
    "Controllers",
    "Helper",
    "Helpers",
    "Service",
    "Services",
    "Component",
    "Components",
    "Middleware",
    "Plugin",
    "Plugins",
    "Session",
    "Token",
    "Tokens",
    "Key",
    "Keys",
    "Cache",
    "Store",
    "Context",
    "Engine",
    "Server",
    "Client",
    "Build",
    "Dev",
    "Static",
    "Dynamic",
    "Remote",
    "Local",
    "Public",
    "Private",
    "Content",
    "Encoding",
    "Type",
    "Types",
    "Error",
    "Errors",
    "Value",
    "Values",
    "Name",
    "Names",
    "Path",
    "Paths",
];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `needle` occurs in `hay` as a whole word.
pub fn contains_word(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(offset) = hay[from..].find(needle) {
        let start = from + offset;
        let end = start + needle.len();
        let before = hay[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c));
        let after = hay[end..].chars().next().is_none_or(|c| !is_word_char(c));
        if before && after {
            return true;
        }
        from = start + needle.len().max(1);
        if from > hay.len() {
            break;
        }
    }
    false
}

/// Identifier-shaped words of a question, in order: qualified names
/// (`Flask.wsgi_app`, `Foo::bar`), snake_case, camelCase, PascalCase with two
/// humps, anything in backticks, and a Capitalized word next to a code noun
/// ("Upload model", "class Upload").
pub fn identifiers(question: &str) -> Vec<String> {
    let p = patterns();
    let mut found: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        let name = name.trim_end_matches("()");
        if name.len() >= 2
            && found.len() < MAX_IDENTIFIERS
            && !STOP.contains(&name)
            && !found.iter().any(|f| f == name)
        {
            found.push(name.to_owned());
        }
    };
    for capture in p.backticked.captures_iter(question) {
        let inner = capture[1].trim();
        if p.token
            .find(inner)
            .is_some_and(|m| m.as_str().len() == inner.trim_end_matches("()").len())
        {
            push(inner);
        }
    }
    let matches: Vec<_> = p.token.find_iter(question).collect();
    for (index, found_token) in matches.iter().enumerate() {
        let token = found_token.as_str();
        let qualified = token.contains("::") || token.contains('.') || token.contains('#');
        if qualified {
            // Not `e.g.` or a file name: every segment is a plausible name.
            let segments: Vec<&str> = token
                .split(['.', '#'])
                .flat_map(|s| s.split("::"))
                .collect();
            if segments
                .iter()
                .all(|s| s.len() >= 2 && s.chars().any(|c| c.is_alphabetic()))
                && !segments
                    .last()
                    .is_some_and(|ext| FILE_EXTENSIONS.contains(ext))
            {
                push(token);
            }
            continue;
        }
        let has_lower = token.chars().any(|c| c.is_lowercase());
        let snake = token.contains('_') && token.chars().any(|c| c.is_alphabetic());
        let camel = has_lower && p.camel.is_match(token);
        let pascal = p.pascal.is_match(token);
        if snake || camel || pascal {
            push(token);
            continue;
        }
        if token.starts_with(|c: char| c.is_uppercase()) && has_lower {
            let after = question[found_token.end()..].trim_start();
            // "Upload ActiveRecord model": the framework name in between does
            // not hide the noun.
            let next = matches[index + 1..]
                .iter()
                .map(|m| m.as_str())
                .find(|word| !STOP.contains(word))
                .map(str::to_ascii_lowercase);
            let previous = index
                .checked_sub(1)
                .and_then(|i| matches.get(i))
                .map(|m| m.as_str().to_ascii_lowercase());
            let noun_after = after.starts_with(|c: char| c.is_alphabetic())
                && next.as_deref().is_some_and(|w| CODE_NOUNS.contains(&w));
            let noun_before = previous.as_deref().is_some_and(|w| CODE_NOUNS.contains(&w));
            let repeated = question.matches(token).count() >= 2;
            let prose = PROSE.contains(&token);
            let sentence_start = question[..found_token.start()]
                .trim_end()
                .chars()
                .next_back()
                .is_none_or(|c| matches!(c, '.' | ':' | ';' | '!' | '?'));
            if noun_after || noun_before || (repeated && !prose) || (!prose && !sentence_start) {
                push(token);
            }
        }
    }
    found
}

const FILE_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rb", "rs", "go", "java", "kt", "cs", "php",
    "swift", "scala", "json", "md", "yml", "yaml", "toml", "html", "css", "scss", "vue", "svelte",
];

/// Everything the question names that the index defines, as pins.
pub fn floor(question: &str, navigation: &NavigationIndex, corpus: &[Chunk]) -> Floor {
    resolve(
        identifiers(question),
        question,
        navigation,
        corpus,
        MAX_PINS,
    )
}

/// Names given explicitly (`symbols`): each is looked up like a question's
/// identifier, with the names themselves as the hint text and room for all.
pub fn pins_for_names(names: &[String], navigation: &NavigationIndex, corpus: &[Chunk]) -> Floor {
    let hint = names.join(" ");
    resolve(
        names.to_vec(),
        &hint,
        navigation,
        corpus,
        names.len().max(1),
    )
}

fn resolve(
    identifiers: Vec<String>,
    question: &str,
    navigation: &NavigationIndex,
    corpus: &[Chunk],
    max_pins: usize,
) -> Floor {
    let mut result = Floor {
        identifiers: identifiers.clone(),
        ..Floor::default()
    };
    let mut lines_by_path: BTreeMap<&str, BTreeMap<usize, &str>> = BTreeMap::new();
    let mut seen: HashSet<(String, usize, usize)> = HashSet::new();
    for identifier in &identifiers {
        if result.pins.len() == max_pins {
            break;
        }
        let mut candidates = lookup(navigation, identifier);
        if candidates.is_empty() {
            continue;
        }
        let leaf = leaf_of(identifier);
        let hinted = |reference: &DefinitionRef| {
            container_hint(navigation, *reference, question)
                || path_hint(navigation.path(*reference), leaf, question)
        };
        let non_test = candidates
            .iter()
            .filter(|r| !search::is_test_path(navigation.path(**r)))
            .count();
        if non_test >= COMMON_NAME_DEFINITIONS {
            // Nesting disambiguates on its own: `Upload` the model is the one
            // top-level `Upload`; `SiteSetting.Upload` and the plugin modules
            // are not what a bare `Upload` means.
            let top_level: Vec<DefinitionRef> = candidates
                .iter()
                .copied()
                .filter(|r| {
                    navigation.get(*r).qualified == leaf
                        && !search::is_test_path(navigation.path(*r))
                })
                .collect();
            if top_level.len() == 1 && !candidates.iter().any(&hinted) {
                let others: Vec<DefinitionRef> = candidates
                    .iter()
                    .copied()
                    .filter(|r| *r != top_level[0])
                    .collect();
                candidates = top_level;
                candidates.extend(others);
            } else {
                candidates.retain(|r| hinted(r));
            }
            if candidates.is_empty() {
                result.notes.push(format!(
                    "`{leaf}` is defined in {non_test} places; name its class or file to pin one."
                ));
                continue;
            }
        }
        candidates.sort_by_cached_key(|r| {
            let definition = navigation.get(*r);
            let path = navigation.path(*r);
            (
                !container_hint(navigation, *r, question),
                !path_hint(path, leaf, question),
                search::is_test_path(path),
                !definition.exported(),
                path.len(),
                path.to_owned(),
                definition.start_line,
            )
        });
        let Some(chosen) = candidates.first().copied() else {
            continue;
        };
        let definition = navigation.get(chosen);
        let path = navigation.path(chosen);
        let key = (path.to_owned(), definition.start_line, definition.end_line);
        if seen.contains(&key) {
            continue;
        }
        let lines = lines_by_path
            .entry(path)
            .or_insert_with(|| file_lines(corpus, path));
        let Some(text) = span_text(lines, definition.start_line, definition.end_line) else {
            continue;
        };
        seen.insert(key);
        result.pins.push(Pin {
            name: definition.name.clone(),
            qualified: definition.qualified.clone(),
            path: path.to_owned(),
            start_line: definition.start_line,
            end_line: definition.end_line,
            kind: definition.kind,
            chunk: Chunk {
                path: path.to_owned(),
                start_line: definition.start_line,
                end_line: definition.end_line,
                text,
                lexical_score: 0.0,
            },
        });
        let others: Vec<String> = candidates
            .iter()
            .skip(1)
            .filter(|r| !search::is_test_path(navigation.path(**r)))
            .take(ALSO_DEFINED)
            .map(|r| format!("{}:{}", navigation.path(*r), navigation.get(*r).start_line))
            .collect();
        if !others.is_empty() {
            let more = candidates.len().saturating_sub(1 + others.len());
            result.notes.push(format!(
                "`{}` is also defined in {}{}.",
                definition.name,
                others.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ));
        }
    }
    result
}

/// The pin for one definition: its whole span as a chunk, when the snapshot
/// holds every line of it.
pub fn pin_for(
    navigation: &NavigationIndex,
    corpus: &[Chunk],
    reference: DefinitionRef,
) -> Option<Pin> {
    let definition = navigation.get(reference);
    let path = navigation.path(reference);
    let lines = file_lines(corpus, path);
    let text = span_text(&lines, definition.start_line, definition.end_line)?;
    Some(Pin {
        name: definition.name.clone(),
        qualified: definition.qualified.clone(),
        path: path.to_owned(),
        start_line: definition.start_line,
        end_line: definition.end_line,
        kind: definition.kind,
        chunk: Chunk {
            path: path.to_owned(),
            start_line: definition.start_line,
            end_line: definition.end_line,
            text,
            lexical_score: 0.0,
        },
    })
}

/// For "who calls X" and "tests for X": the name may be a plain word
/// (`authenticate`), so when the identifier rules found nothing, every word
/// of the question that the index defines is a candidate; the last one wins,
/// preferring a top-level, non-test definition.
pub fn named_target(question: &str, navigation: &NavigationIndex, corpus: &[Chunk]) -> Option<Pin> {
    let found = floor(question, navigation, corpus);
    if let Some(pin) = found.pins.into_iter().next() {
        return Some(pin);
    }
    let words: Vec<&str> = patterns()
        .token
        .find_iter(question)
        .map(|m| m.as_str())
        .filter(|w| w.len() >= 3 && !CALLERS_WORDS.contains(&w.to_ascii_lowercase().as_str()))
        .collect();
    for word in words.iter().rev() {
        let mut candidates = navigation.lookup(word).to_vec();
        if candidates.is_empty() {
            continue;
        }
        candidates.sort_by_cached_key(|r| {
            let d = navigation.get(*r);
            (
                search::is_test_path(navigation.path(*r)),
                d.qualified != d.name,
                !d.exported(),
                navigation.path(*r).len(),
            )
        });
        if let Some(pin) = pin_for(navigation, corpus, candidates[0]) {
            return Some(pin);
        }
    }
    None
}

/// Words of a callers or tests question that are never the target.
const CALLERS_WORDS: &[&str] = &[
    "who",
    "calls",
    "call",
    "called",
    "caller",
    "callers",
    "uses",
    "use",
    "used",
    "usage",
    "usages",
    "where",
    "what",
    "which",
    "all",
    "the",
    "every",
    "each",
    "places",
    "sites",
    "site",
    "from",
    "for",
    "and",
    "are",
    "that",
    "this",
    "with",
    "invokes",
    "invoked",
    "references",
    "referenced",
    "reference",
    "tests",
    "test",
    "specs",
    "spec",
    "find",
    "list",
    "show",
    "locate",
    "existing",
    "related",
    "relevant",
    "unit",
    "integration",
    "covering",
    "cover",
    "exercising",
    "function",
    "method",
    "class",
    "definition",
    "implementation",
    "code",
    "file",
    "files",
    "module",
    "internal",
    "non",
];

fn leaf_of(identifier: &str) -> &str {
    identifier
        .rsplit(['.', '#', ':'])
        .next()
        .unwrap_or(identifier)
}

fn lookup(navigation: &NavigationIndex, identifier: &str) -> Vec<DefinitionRef> {
    let qualified = identifier.replace("::", ".").replace('#', ".");
    if qualified.contains('.') {
        let found = navigation.lookup_qualified(&qualified);
        if !found.is_empty() {
            return found;
        }
    }
    let leaf = leaf_of(identifier);
    let found = navigation.lookup(leaf);
    if !found.is_empty() {
        return found.to_vec();
    }
    // A case slip inside a name (`getrequestMeta`) is a typo; a different
    // first letter is a different kind of name (`Upload` the model is not
    // `upload` the helper).
    let first = leaf.chars().next().map(|c| c.is_uppercase());
    navigation
        .lookup_insensitive(leaf)
        .into_iter()
        .filter(|r| {
            navigation
                .get(*r)
                .name
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                == first
        })
        .collect()
}

/// The definition's container (or any qualifier segment) is a word of the question.
fn container_hint(navigation: &NavigationIndex, reference: DefinitionRef, question: &str) -> bool {
    let definition = navigation.get(reference);
    let Some((prefix, _)) = definition.qualified.rsplit_once('.') else {
        return false;
    };
    prefix
        .split('.')
        .any(|segment| segment.len() >= 2 && contains_word(question, segment))
}

/// A path segment (file stem or directory) is a word of the question. The
/// name itself does not count: `page.tsx` says nothing about which `Page`.
fn path_hint(path: &str, name: &str, question: &str) -> bool {
    let lower = question.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    path.split('/')
        .map(|segment| segment.rsplit_once('.').map_or(segment, |(stem, _)| stem))
        .map(|segment| segment.to_ascii_lowercase())
        .filter(|segment| {
            segment.len() >= 3
                && *segment != name
                && !matches!(
                    segment.as_str(),
                    "src" | "lib" | "app" | "index" | "main" | "mod"
                )
        })
        .any(|segment| contains_word(&lower, &segment))
}

fn file_lines<'a>(corpus: &'a [Chunk], path: &str) -> BTreeMap<usize, &'a str> {
    let mut lines = BTreeMap::new();
    for chunk in corpus.iter().filter(|chunk| chunk.path == path) {
        if chunk.start_line == 0
            || chunk.text.split('\n').count() != chunk.end_line - chunk.start_line + 1
        {
            continue;
        }
        for (offset, text) in chunk.text.split('\n').enumerate() {
            lines.entry(chunk.start_line + offset).or_insert(text);
        }
    }
    lines
}

fn span_text(lines: &BTreeMap<usize, &str>, start: usize, end: usize) -> Option<String> {
    if start == 0 || end < start {
        return None;
    }
    (start..=end)
        .map(|line| lines.get(&line).copied())
        .collect::<Option<Vec<_>>>()
        .map(|lines| lines.join("\n"))
}

/// The shortlist with the pins at its head, within the shortlist limit.
pub fn pinned_shortlist(pins: &[Pin], shortlist: Vec<Chunk>) -> Vec<Chunk> {
    if pins.is_empty() {
        return shortlist;
    }
    let mut out: Vec<Chunk> = pins.iter().map(|pin| pin.chunk.clone()).collect();
    out.extend(
        shortlist
            .into_iter()
            .filter(|chunk| !pins.iter().any(|pin| pin.is(chunk))),
    );
    out.truncate(search::SHORTLIST_LIMIT.max(pins.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::NavigationPreparer;
    use std::sync::Arc;

    #[test]
    fn identifiers_follow_the_shapes_agents_use() {
        let cases: &[(&str, &[&str])] = &[
            (
                "Flask.wsgi_app method definition and what it calls",
                &["Flask.wsgi_app"],
            ),
            (
                "renderToHTMLOrFlight function definition in app-render",
                &["renderToHTMLOrFlight"],
            ),
            (
                "Upload ActiveRecord model definition class Upload",
                &["Upload"],
            ),
            (
                "Upload ActiveRecord model that represents an uploaded file",
                &["Upload"],
            ),
            ("Post model where uploads are validated", &["Post"]),
            (
                "NextNodeServer subclass extending base Server",
                &["NextNodeServer"],
            ),
            (
                "Trace remote image dimension probing: locate the initial URL",
                &[],
            ),
            ("where is `Context` used in the router", &["Context"]),
            (
                "replacement expansion: Captures trait interpolate",
                &["Captures"],
            ),
            ("how does the router match dynamic segments", &[]),
            (
                "Foo::bar and Baz#qux in index.ts, e.g. the handler",
                &["Foo::bar", "Baz#qux"],
            ),
            (
                "getRequestMeta() and add_request_meta",
                &["getRequestMeta", "add_request_meta"],
            ),
        ];
        for (question, expected) in cases {
            assert_eq!(identifiers(question), *expected, "{question}");
        }
    }

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
    fn a_named_definition_is_pinned_whole_and_others_are_listed() {
        let (chunks, index) = corpus(&[
            (
                "src/server/next-server.ts",
                "import { BaseServer } from './base-server';\nexport default class NextNodeServer extends BaseServer<Options> {\n  handle() {\n    return 1;\n  }\n}\n",
            ),
            (
                "src/server/base-server.ts",
                "export class BaseServer {\n  handle() { return 0; }\n}\nexport function handle() {}\n",
            ),
            ("test/server.test.ts", "export function handle() {}\n"),
        ]);
        let found = floor(
            "NextNodeServer subclass extending base Server",
            &index,
            &chunks,
        );
        assert_eq!(found.pins.len(), 1);
        let pin = &found.pins[0];
        assert_eq!(
            (pin.path.as_str(), pin.start_line, pin.end_line),
            ("src/server/next-server.ts", 2, 6)
        );
        assert!(
            pin.chunk
                .text
                .starts_with("export default class NextNodeServer")
        );
        assert!(found.notes.is_empty());

        // `handle` is defined three times: the question's class picks the method.
        let found = floor("NextNodeServer `handle` method", &index, &chunks);
        assert_eq!(found.pins.len(), 2);
        assert!(
            found
                .pins
                .iter()
                .any(|p| p.qualified == "NextNodeServer.handle"),
            "{:?}",
            found.pins
        );
        assert_eq!(
            found.notes,
            [
                "`handle` is also defined in src/server/base-server.ts:4, src/server/base-server.ts:2 and 1 more."
            ]
        );

        // Without a hint the exported function wins over the method and the test.
        let found = floor("where is `handle` defined", &index, &chunks);
        assert_eq!(found.pins[0].qualified, "handle");
        assert_eq!(found.pins[0].path, "src/server/base-server.ts");
    }

    #[test]
    fn common_names_need_a_hint_and_pins_lead_the_shortlist() {
        let files: Vec<(String, String)> = ["home", "shop", "blog", "docs", "auth", "admin"]
            .iter()
            .map(|area| {
                (
                    format!("src/pages/{area}/page.tsx"),
                    "export default function Page() { return null; }\n".to_owned(),
                )
            })
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        let (chunks, index) = corpus(&refs);
        let floor_ = floor("the Page component", &index, &chunks);
        assert!(floor_.pins.is_empty());
        assert_eq!(
            floor_.notes,
            ["`Page` is defined in 6 places; name its class or file to pin one."]
        );
        // One top-level definition among nested ones needs no hint.
        let mut nested: Vec<(String, String)> = files.clone();
        nested.push((
            "src/models/page.tsx".into(),
            "export class Page {}\n".into(),
        ));
        for (_, text) in nested.iter_mut().take(6) {
            // Six methods named `Page` on other classes: the name is common.
            *text = "export class Admin { Page() { return 1; } }\n".to_owned();
        }
        let refs2: Vec<(&str, &str)> = nested
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        let (chunks2, index2) = corpus(&refs2);
        let found = floor("the Page class", &index2, &chunks2);
        assert_eq!(found.pins.len(), 1, "{:?}", found.notes);
        assert_eq!(found.pins[0].path, "src/models/page.tsx");
        let floor_ = floor("the Page component in admin", &index, &chunks);
        assert_eq!(floor_.pins.len(), 1);
        assert_eq!(floor_.pins[0].path, "src/pages/admin/page.tsx");
        let shortlist = pinned_shortlist(&floor_.pins, chunks.clone());
        assert!(floor_.pins[0].is(&shortlist[0]));
        assert_eq!(shortlist.len(), chunks.len());
    }
}
