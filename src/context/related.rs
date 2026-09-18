//! Conservative lexical matching, deliberately short of import/type resolution.
//! Unknown qualification and ambiguous bare names are grounds to abstain.
use super::{Declaration, Language, Snapshot, SourceExcerpt, SourceLocation, language};
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub(super) struct Reference {
    pub name: String,
    parts: Vec<String>,
    pub source: SourceLocation,
}

fn calls() -> &'static Regex {
    static CALLS: OnceLock<Regex> = OnceLock::new();
    CALLS.get_or_init(|| Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*(?:\s*(?:::|\?\.|\.|->)\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*(?:::<[^;{}\n]*>)?\s*\(").unwrap())
}

pub(super) fn references(snapshot: &Snapshot<'_>, excerpt: &SourceExcerpt) -> Vec<Reference> {
    let mut offsets = Vec::new();
    let mut text = String::new();
    // Include preceding source so a qualifier just outside the excerpt is not
    // mistaken for a bare call. Filter the returned references to the excerpt.
    for (&line, code) in snapshot.code.range(..=excerpt.end_line) {
        offsets.push((text.len(), line));
        text.push_str(code);
        text.push('\n');
    }
    let mut references = Vec::new();
    for capture in calls().captures_iter(&text) {
        let matched = capture.get(1).unwrap();
        let line = offsets[offsets.partition_point(|(offset, _)| *offset <= matched.start()) - 1].1;
        if line < excerpt.start_line || references.len() >= 64 {
            continue;
        }
        let previous = text[..matched.start()]
            .chars()
            .rev()
            .find(|c| !c.is_whitespace());
        // A failed partial match after an unsupported receiver, generic path,
        // escaped/Unicode identifier, or attribute must never become bare.
        if previous.is_some_and(|c| !c.is_ascii() || matches!(c, '.' | ':' | '>' | '$' | '#' | '@'))
        {
            continue;
        }
        let line_prefix = &text[offsets
            [offsets.partition_point(|(offset, _)| *offset <= matched.start()) - 1]
            .0..matched.start()];
        if excerpt.path.ends_with(".rs") && line_prefix.trim_start().starts_with("#[") {
            continue;
        }
        let chain = matched.as_str();
        if chain.contains('.') || chain.contains("->") {
            continue; // Receiver/type information is required for methods.
        }
        let parts = chain
            .split("::")
            .map(|p| p.trim().to_owned())
            .collect::<Vec<_>>();
        let name = parts.last().unwrap();
        if name.len() > 128
            || snapshot
                .declarations
                .iter()
                .any(|d| d.line == line && d.name == *name)
        {
            continue;
        }
        references.push(Reference {
            name: name.clone(),
            parts,
            source: SourceLocation {
                path: excerpt.path.clone(),
                line,
            },
        });
    }
    references
}

#[derive(Clone, Default)]
struct Scope {
    modules: Vec<String>,
    free: bool,
}
struct FileIndex {
    scopes: BTreeMap<usize, Scope>,
    custom_module_paths: bool,
}
impl FileIndex {
    fn new(path: &str, snapshot: &Snapshot<'_>) -> Self {
        static MODULE: OnceLock<Regex> = OnceLock::new();
        let module =
            MODULE.get_or_init(|| Regex::new(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*$").unwrap());
        let mut scopes = BTreeMap::new();
        let mut stack: Vec<Option<String>> = vec![];
        let mut previous = 0;
        for (&line, text) in &snapshot.code {
            if line != previous + 1 {
                // A gap can hide a container opening. Do not assert top level.
                stack = vec![None];
            }
            previous = line;
            let scope = match language(path) {
                Language::Braces => Scope {
                    modules: stack.iter().filter_map(Clone::clone).collect(),
                    free: stack.iter().all(Option::is_some),
                },
                Language::Python | Language::Ruby => Scope {
                    modules: vec![],
                    free: text.len() == text.trim_start().len(),
                },
                Language::Other => Scope::default(),
            };
            scopes.insert(line, scope);
            if language(path) == Language::Braces {
                for (offset, byte) in text.bytes().enumerate() {
                    if byte == b'{' {
                        let name = path
                            .ends_with(".rs")
                            .then(|| module.captures(&text[..offset]))
                            .flatten()
                            .map(|capture| capture[1].to_owned());
                        stack.push(name);
                    } else if byte == b'}' {
                        stack.pop();
                    }
                }
            }
        }
        let compact = snapshot
            .code
            .values()
            .flat_map(|line| line.chars())
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        Self {
            scopes,
            custom_module_paths: compact.contains("#[path="),
        }
    }
    fn scope(&self, snapshot: &Snapshot<'_>, line: usize, for_call: bool) -> Option<&Scope> {
        let line = if for_call {
            snapshot.containing(line).map_or(line, |d| d.line)
        } else {
            line
        };
        self.scopes.get(&line)
    }
}

fn family(path: &str) -> &str {
    match path.rsplit('.').next().unwrap_or("") {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => "js",
        "py" | "pyi" => "py",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        extension => extension,
    }
}

fn has_word(text: &str, name: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| word == name)
}

fn shadowed(
    reference: &Reference,
    snapshot: &Snapshot<'_>,
    index: &FileIndex,
    scope: &Scope,
) -> bool {
    let parent = snapshot.containing(reference.source.line);
    // Parameters and local bindings are not free-function references. Binding
    // analysis is conservative: a same-name binding anywhere earlier in this
    // body prevents expansion even if its nested scope has already ended.
    let header = snapshot
        .code
        .range(parent.map_or(reference.source.line, |d| d.line)..=reference.source.line)
        .take(12)
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let header = header.split('{').next().unwrap_or(&header);
    if let Some((_, parameters)) = header.split_once('(')
        && has_word(
            parameters.split(')').next().unwrap_or(parameters),
            &reference.name,
        )
    {
        return true;
    }
    for (&line, text) in snapshot.code.range(..=reference.source.line) {
        let global = index
            .scopes
            .get(&line)
            .is_some_and(|other| other.free && other.modules == scope.modules);
        let local = parent.is_some_and(|d| line >= d.line && line <= d.end);
        if !global && !local {
            continue;
        }
        if global
            && snapshot
                .declarations
                .iter()
                .any(|d| d.line == line && d.name == reference.name)
        {
            continue;
        }
        if text.trim_start().starts_with("use ") || text.trim_start().starts_with("pub use ") {
            continue;
        }
        let words = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .collect::<Vec<_>>();
        for (index, word) in words.iter().enumerate() {
            if matches!(*word, "let" | "const" | "var" | "for" | "as") {
                let next = words[index + 1..]
                    .iter()
                    .filter(|word| !word.is_empty() && **word != "mut")
                    .take(1)
                    .next();
                if next.is_some_and(|word| **word == reference.name) {
                    return true;
                }
            }
        }
        // Python assignments, Go short declarations, and destructuring are
        // deliberately treated as uncertain rather than traced dynamically.
        if let Some((left, _)) = text.split_once('=')
            && has_word(left, &reference.name)
            && !left.contains("fn ")
            && !left.contains("function ")
        {
            return true;
        }
    }
    false
}

enum Import {
    None,
    Path(Vec<String>),
    Uncertain,
}
fn imported(
    name: &str,
    path: &str,
    snapshot: &Snapshot<'_>,
    scope: &Scope,
    index: &FileIndex,
) -> Import {
    static SIMPLE_USE: OnceLock<Regex> = OnceLock::new();
    let simple_use = SIMPLE_USE.get_or_init(|| Regex::new(r"^(?:pub(?:\([^)]*\))?\s+)?use\s+((?:[A-Za-z_][A-Za-z0-9_]*::)*[A-Za-z_][A-Za-z0-9_]*)(?:\s+as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;$").unwrap());
    let mut found = None;
    let mut statement = String::new();
    for (&line, text) in &snapshot.code {
        let text = text.trim();
        if path.ends_with(".rs") {
            if statement.is_empty() {
                if !(text.starts_with("use ")
                    || text.starts_with("pub use ")
                    || text.starts_with("pub(") && text.contains(" use "))
                {
                    continue;
                }
                if index
                    .scopes
                    .get(&line)
                    .is_none_or(|other| other.modules != scope.modules)
                {
                    continue;
                }
                if index.scopes.get(&line).is_none_or(|other| !other.free) {
                    return Import::Uncertain;
                }
            }
            statement.push_str(text);
            statement.push(' ');
            if !text.contains(';') {
                continue;
            }
            let candidate = statement.trim();
            if let Some(capture) = simple_use.captures(candidate) {
                let parts = capture[1]
                    .split("::")
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let alias = capture
                    .get(2)
                    .map_or(parts.last().unwrap().as_str(), |m| m.as_str());
                if alias == name {
                    if found.is_some() {
                        return Import::Uncertain;
                    }
                    found = Some(parts);
                }
            } else if has_word(candidate, name) || candidate.contains('*') {
                return Import::Uncertain;
            }
            statement.clear();
        } else {
            if statement.is_empty()
                && !(text.starts_with("import ")
                    || text.starts_with("from ")
                    || text.starts_with("using "))
            {
                continue;
            }
            statement.push_str(text);
            statement.push(' ');
            if has_word(&statement, name) || statement.contains('*') {
                return Import::Uncertain;
            }
            if text.contains(';')
                || text.contains(" from ")
                || (!statement.contains('{') && !statement.contains('('))
                || text.ends_with(')')
            {
                statement.clear();
            }
        }
    }
    if !statement.is_empty() && (has_word(&statement, name) || statement.contains('*')) {
        return Import::Uncertain;
    }
    found.map_or(Import::None, Import::Path)
}

/// Conventional Rust module identity within a source root. A qualified call
/// still needs an explicit crate/self/super path or a local mod/import below.
fn rust_module(path: &str, modules: &[String]) -> (String, Vec<String>) {
    let (root, relative) = if let Some((prefix, relative)) = path.rsplit_once("/src/") {
        (format!("{prefix}/src/"), relative)
    } else if let Some(relative) = path.strip_prefix("src/") {
        ("src/".into(), relative)
    } else {
        (String::new(), path)
    };
    let mut parts = relative
        .trim_end_matches(".rs")
        .split('/')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if parts
        .last()
        .is_some_and(|part| matches!(part.as_str(), "mod" | "lib" | "main"))
    {
        parts.pop();
    }
    parts.extend_from_slice(modules);
    (root, parts)
}

fn qualified_target(
    parts: &[String],
    reference: &Reference,
    scope: &Scope,
    snapshot: &Snapshot<'_>,
    index: &FileIndex,
) -> Option<(String, Vec<String>)> {
    if !reference.source.path.ends_with(".rs") {
        return None;
    }
    if index.custom_module_paths {
        return None;
    }
    let (root, mut target) = rust_module(&reference.source.path, &scope.modules);
    let mut parts = parts.to_vec();
    match parts.first()?.as_str() {
        "crate" => {
            target.clear();
            parts.remove(0);
        }
        "self" => {
            parts.remove(0);
        }
        "super" => {
            while parts.first().is_some_and(|part| part == "super") {
                target.pop()?;
                parts.remove(0);
            }
        }
        first => {
            match imported(first, &reference.source.path, snapshot, scope, index) {
                Import::Path(mut import) => {
                    import.extend(parts.into_iter().skip(1));
                    // Only anchored imports are supported; aliases are never
                    // repeatedly rewritten or retried by their last name.
                    if !matches!(import.first()?.as_str(), "crate" | "self" | "super") {
                        return None;
                    }
                    return qualified_target(&import, reference, scope, snapshot, index);
                }
                Import::Uncertain => return None,
                Import::None => {}
            }
            let declared = snapshot.code.iter().any(|(&line, text)| {
                index
                    .scopes
                    .get(&line)
                    .is_some_and(|other| other.modules == scope.modules && other.free)
                    && text
                        .split_whitespace()
                        .skip_while(|word| word.starts_with("pub"))
                        .collect::<Vec<_>>()
                        .join(" ")
                        .strip_prefix("mod ")
                        .is_some_and(|suffix| {
                            suffix
                                .strip_prefix(first)
                                .is_some_and(|rest| rest.trim_start().starts_with([';', '{']))
                        })
            });
            if !declared {
                return None;
            }
        }
    }
    target.extend(parts);
    Some((root, target))
}

pub(super) struct Matcher {
    indices: HashMap<String, FileIndex>,
}
impl Matcher {
    pub fn new() -> Self {
        Self {
            indices: HashMap::new(),
        }
    }
    pub fn definition_name(
        &mut self,
        reference: &Reference,
        snapshot: &Snapshot<'_>,
    ) -> Option<String> {
        let index = self
            .indices
            .entry(reference.source.path.clone())
            .or_insert_with(|| FileIndex::new(&reference.source.path, snapshot));
        let scope = index.scope(snapshot, reference.source.line, true)?;
        let mut parts = reference.parts.clone();
        if parts.len() == 1 {
            if shadowed(reference, snapshot, index, scope) {
                return None;
            }
            match imported(
                &reference.name,
                &reference.source.path,
                snapshot,
                scope,
                index,
            ) {
                Import::Path(path) => parts = path,
                Import::Uncertain => return None,
                Import::None => {}
            }
        }
        if parts.len() > 1 {
            qualified_target(&parts, reference, scope, snapshot, index)?;
        }
        parts.last().cloned()
    }
    pub fn select<'a>(
        &mut self,
        reference: &Reference,
        candidates: &'a [(String, Declaration)],
        snapshots: &HashMap<&str, Snapshot<'_>>,
    ) -> Option<&'a (String, Declaration)> {
        let source = &snapshots[reference.source.path.as_str()];
        self.indices
            .entry(reference.source.path.clone())
            .or_insert_with(|| FileIndex::new(&reference.source.path, source));
        for (path, _) in candidates {
            self.indices
                .entry(path.clone())
                .or_insert_with(|| FileIndex::new(path, &snapshots[path.as_str()]));
        }
        let index = &self.indices[&reference.source.path];
        let scope = index.scope(source, reference.source.line, true)?;
        let mut parts = reference.parts.clone();
        if parts.len() == 1 {
            if shadowed(reference, source, index, scope) {
                return None;
            }
            match imported(
                &reference.name,
                &reference.source.path,
                source,
                scope,
                index,
            ) {
                Import::Path(path) => parts = path,
                Import::Uncertain => return None,
                Import::None => {}
            }
        }
        let qualified = if parts.len() > 1 {
            Some(qualified_target(&parts, reference, scope, source, index)?)
        } else {
            None
        };
        let eligible = candidates
            .iter()
            .filter(|(path, declaration)| {
                if family(path) != family(&reference.source.path) {
                    return false;
                }
                let candidate = &snapshots[path.as_str()];
                let Some(candidate_scope) =
                    self.indices[path].scope(candidate, declaration.line, false)
                else {
                    return false;
                };
                if !candidate_scope.free {
                    return false;
                }
                if path.ends_with(".go")
                    && candidate.code[&declaration.line]
                        .trim_start()
                        .starts_with("func (")
                {
                    return false;
                }
                if let Some((root, target)) = &qualified {
                    let (candidate_root, mut module) = rust_module(path, &candidate_scope.modules);
                    module.push(declaration.name.clone());
                    &candidate_root == root && &module == target
                } else {
                    if path == &reference.source.path {
                        candidate_scope.modules == scope.modules
                    } else {
                        candidate_scope.modules.is_empty()
                    }
                }
            })
            .collect::<Vec<_>>();
        if qualified.is_none() {
            let local = eligible
                .iter()
                .copied()
                .filter(|(path, declaration)| {
                    path == &reference.source.path
                        && self.indices[path]
                            .scope(&snapshots[path.as_str()], declaration.line, false)
                            .is_some_and(|other| other.modules == scope.modules)
                })
                .collect::<Vec<_>>();
            if local.len() == 1 {
                return Some(local[0]);
            }
            if !local.is_empty() {
                return None;
            }
        }
        (eligible.len() == 1).then(|| eligible[0])
    }
}
