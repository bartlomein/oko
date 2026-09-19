//! Cached syntax facts and conservative, snapshot-local JavaScript navigation.
//!
//! Parsing establishes syntax, not runtime types. Relationships require a local
//! binding or an explicit import whose target is unambiguous in this snapshot.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tree_sitter::{Node, ParseOptions, Parser};

const MAX_FACTS: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionKind {
    Function,
    Constant,
    Type,
    Class,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Definition {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub complete: bool,
    pub kind: DefinitionKind,
    exports: Vec<String>,
    start_byte: usize,
    end_byte: usize,
    namespace: Namespace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Namespace {
    Value,
    Type,
    Both,
}
impl Namespace {
    fn accepts(self, other: Self) -> bool {
        self == Self::Both || other == Self::Both || self == other
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum BindingTarget {
    Definition(usize),
    Import(usize),
    Opaque,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reference {
    pub name: String,
    pub line: usize,
    pub is_call: bool,
    namespace: Namespace,
    target: BindingTarget,
    caller: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Import {
    pub local: String,
    pub imported: String,
    pub specifier: String,
    pub type_only: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModuleConfig {
    pub base_url: Option<String>,
    pub paths: BTreeMap<String, Vec<String>>,
    pub extends: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileFacts {
    pub definitions: Vec<Definition>,
    pub references: Vec<Reference>,
    pub imports: Vec<Import>,
    pub config: Option<ModuleConfig>,
    path: String,
    digest: Vec<u8>,
    lines: usize,
    bytes: usize,
    valid: bool,
}

impl FileFacts {
    pub fn matches_source(&self, path: &str, text: &str) -> bool {
        self.matches_captured_source(path, text.len(), &Sha256::digest(text.as_bytes()).into())
            && self.lines == text.bytes().filter(|b| *b == b'\n').count() + 1
    }

    /// Reuse a digest computed from the exact captured text during discovery.
    /// The caller must pass the digest of BOM-stripped text, matching prepare().
    /// Cache-envelope integrity protects these derived facts; content identity
    /// still comes from fresh file bytes, never timestamps or cached digests.
    pub(crate) fn matches_captured_source(
        &self,
        path: &str,
        bytes: usize,
        digest: &[u8; 32],
    ) -> bool {
        self.path == path
            && self.bytes == bytes
            && self.digest.as_slice() == digest
            && self.lines > 0
            && self.lines <= bytes.saturating_add(1)
            && self.definitions.len() <= MAX_FACTS
            && self.references.len() <= MAX_FACTS
            && self.imports.len() <= MAX_FACTS
            && (self.valid
                || (self.definitions.is_empty()
                    && self.references.is_empty()
                    && self.imports.is_empty()))
            && self.definitions.iter().all(|d| {
                d.start_line > 0
                    && d.start_line <= d.end_line
                    && d.end_line <= self.lines
                    && d.start_byte <= d.end_byte
                    && d.end_byte <= self.bytes
            })
            && self.references.iter().all(|r| {
                r.line > 0
                    && r.line <= self.lines
                    && r.caller.is_none_or(|i| i < self.definitions.len())
                    && match r.target {
                        BindingTarget::Definition(i) => i < self.definitions.len(),
                        BindingTarget::Import(i) => i < self.imports.len(),
                        BindingTarget::Opaque => false,
                    }
            })
    }
}

pub fn supports(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts")
    )
}

#[derive(Default)]
pub struct NavigationPreparer {
    js: Option<Parser>,
    ts: Option<Parser>,
    tsx: Option<Parser>,
}
impl NavigationPreparer {
    pub fn prepare(&mut self, path: &str, text: &str) -> FileFacts {
        let mut facts = FileFacts {
            path: path.into(),
            digest: Sha256::digest(text.as_bytes()).to_vec(),
            lines: text.bytes().filter(|b| *b == b'\n').count() + 1,
            bytes: text.len(),
            ..FileFacts::default()
        };
        if path.rsplit('/').next() == Some("tsconfig.json") {
            facts.config = parse_config(text);
            facts.valid = facts.config.is_some();
            return facts;
        }
        if !supports(path) || text.len() > 256 * 1024 {
            return facts;
        }
        let (slot, language) = match path.rsplit('.').next() {
            Some("tsx") => (&mut self.tsx, tree_sitter_typescript::LANGUAGE_TSX.into()),
            Some("ts" | "mts" | "cts") => (
                &mut self.ts,
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            ),
            _ => (&mut self.js, tree_sitter_javascript::LANGUAGE.into()),
        };
        let parser = slot.get_or_insert_with(|| {
            let mut parser = Parser::new();
            parser.set_language(&language).expect("bundled parser ABI");
            parser
        });
        let started = Instant::now();
        let mut cancel =
            |_: &tree_sitter::ParseState| started.elapsed() > Duration::from_millis(250);
        let mut read = |offset: usize, _: tree_sitter::Point| &text.as_bytes()[offset..];
        let Some(tree) = parser.parse_with_options(
            &mut read,
            None,
            Some(ParseOptions::new().progress_callback(&mut cancel)),
        ) else {
            parser.reset();
            return facts;
        };
        // A partial syntax tree can misidentify scopes. Keep the existing lexical
        // fallback for the whole file rather than claiming precise relationships.
        if tree.root_node().has_error() {
            return facts;
        }
        let mut collector = Collector::new(text, facts);
        collector.visit(tree.root_node(), 0);
        collector.finish()
    }
}

#[derive(Clone)]
struct Scope {
    parent: Option<usize>,
    function: bool,
}
struct Binding {
    name: String,
    scope: usize,
    namespace: Namespace,
    target: BindingTarget,
}
struct Pending {
    name: String,
    scope: usize,
    line: usize,
    namespace: Namespace,
    is_call: bool,
    caller: Option<usize>,
}
struct Collector<'a> {
    text: &'a str,
    facts: FileFacts,
    scopes: Vec<Scope>,
    bindings: Vec<Binding>,
    pending: Vec<Pending>,
    depth: usize,
    overflow: bool,
    current_caller: Option<usize>,
}
impl<'a> Collector<'a> {
    fn new(text: &'a str, facts: FileFacts) -> Self {
        Self {
            text,
            facts,
            scopes: vec![Scope {
                parent: None,
                function: true,
            }],
            bindings: vec![],
            pending: vec![],
            depth: 0,
            overflow: false,
            current_caller: None,
        }
    }
    fn text(&self, node: Node<'_>) -> &'a str {
        &self.text[node.byte_range()]
    }
    fn scope(&mut self, parent: usize, function: bool) -> usize {
        self.scopes.push(Scope {
            parent: Some(parent),
            function,
        });
        self.scopes.len() - 1
    }
    fn bind(&mut self, name: &str, scope: usize, namespace: Namespace, target: BindingTarget) {
        self.bindings.push(Binding {
            name: name.into(),
            scope,
            namespace,
            target,
        });
    }
    fn pattern(&mut self, node: Node<'_>, scope: usize) {
        if self.depth >= 256 {
            self.overflow = true;
            return;
        }
        self.depth += 1;
        self.pattern_inner(node, scope);
        self.depth -= 1;
    }
    fn pattern_inner(&mut self, node: Node<'_>, scope: usize) {
        match node.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => self.bind(
                self.text(node),
                scope,
                Namespace::Value,
                BindingTarget::Opaque,
            ),
            "type_identifier" => self.bind(
                self.text(node),
                scope,
                Namespace::Type,
                BindingTarget::Opaque,
            ),
            "type_annotation" => {}
            "pair_pattern" => {
                if let Some(value) = node.child_by_field_name("value") {
                    self.pattern(value, scope);
                }
            }
            "assignment_pattern" | "object_assignment_pattern" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.pattern(left, scope);
                }
            }
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = node
                    .child_by_field_name("pattern")
                    .or_else(|| node.child_by_field_name("name"))
                {
                    self.pattern(pattern, scope);
                }
            }
            _ => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.pattern(child, scope);
                }
            }
        }
    }
    fn parameter_types(&mut self, node: Node<'_>, scope: usize) {
        if self.depth >= 256 {
            self.overflow = true;
            return;
        }
        if node.kind() == "type_annotation" {
            self.visit(node, scope);
            return;
        }
        self.depth += 1;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.parameter_types(child, scope);
        }
        self.depth -= 1;
    }
    fn definition(
        &mut self,
        name: Node<'_>,
        node: Node<'_>,
        kind: DefinitionKind,
        scope: usize,
        namespace: Namespace,
    ) -> usize {
        let mut span = node;
        if let Some(parent) = node.parent()
            && matches!(
                parent.kind(),
                "lexical_declaration" | "variable_declaration"
            )
        {
            span = parent;
        }
        let mut exports = vec![];
        if let Some(parent) = span.parent().filter(|p| p.kind() == "export_statement") {
            let prefix = &self.text[parent.start_byte()..span.start_byte()];
            exports.push(if prefix.split_whitespace().any(|w| w == "default") {
                "default".into()
            } else {
                self.text(name).into()
            });
            span = parent;
        }
        // The bundled TS grammar accepts a bare `export` followed by a newline
        // as an identifier expression plus a declaration, without an ERROR node.
        // This keyword cannot be an identifier expression in JS/TS. Recognize
        // only that exact adjacent syntax; a semicolon or any other token stops
        // the correction. Original byte/line locations remain authoritative.
        let export_prefix = span.prev_named_sibling().filter(|previous| {
            exports.is_empty()
                && span
                    .parent()
                    .is_some_and(|parent| parent.kind() == "program")
                && previous.kind() == "expression_statement"
                && self.text(*previous) == "export"
                && self.text[previous.end_byte()..span.start_byte()]
                    .chars()
                    .all(char::is_whitespace)
        });
        if export_prefix.is_some() {
            exports.push(self.text(name).into());
        }
        let start = export_prefix.unwrap_or(span);
        let id = self.facts.definitions.len();
        self.facts.definitions.push(Definition {
            name: self.text(name).into(),
            start_line: start.start_position().row + 1,
            end_line: end_line(span),
            complete: !span.has_error(),
            kind,
            exports,
            start_byte: start.start_byte(),
            end_byte: span.end_byte(),
            namespace,
        });
        self.bind(
            self.text(name),
            scope,
            namespace,
            BindingTarget::Definition(id),
        );
        id
    }
    fn visit(&mut self, node: Node<'_>, scope: usize) {
        if self.depth >= 256 {
            self.overflow = true;
            return;
        }
        self.depth += 1;
        self.visit_inner(node, scope);
        self.depth -= 1;
    }
    fn visit_inner(&mut self, node: Node<'_>, scope: usize) {
        if self.facts.definitions.len() > MAX_FACTS
            || self.pending.len() > MAX_FACTS
            || self.bindings.len() > MAX_FACTS
        {
            return;
        }
        match node.kind() {
            "comment" | "string" | "regex" | "import_attribute" => return,
            "import_statement" => {
                self.imports(node, scope);
                return;
            }
            "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition" => {
                let owner = if matches!(
                    node.kind(),
                    "function_declaration" | "generator_function_declaration"
                ) {
                    node.child_by_field_name("name").map(|name| {
                        self.definition(
                            name,
                            node,
                            DefinitionKind::Function,
                            scope,
                            Namespace::Value,
                        )
                    })
                } else if node.parent().is_some_and(|parent| {
                    parent.kind() == "variable_declarator"
                        && parent.child_by_field_name("value") == Some(node)
                }) {
                    self.facts
                        .definitions
                        .last()
                        .filter(|d| {
                            d.kind == DefinitionKind::Function
                                && d.start_byte <= node.start_byte()
                                && d.end_byte >= node.end_byte()
                        })
                        .map(|_| self.facts.definitions.len() - 1)
                } else {
                    None
                };
                let previous_caller = self.current_caller;
                self.current_caller = owner;
                let inner = self.scope(scope, true);
                if !matches!(
                    node.kind(),
                    "function_declaration" | "generator_function_declaration" | "arrow_function"
                ) && let Some(name) = node
                    .child_by_field_name("name")
                    .filter(|n| n.kind() == "identifier")
                {
                    self.bind(
                        self.text(name),
                        inner,
                        Namespace::Value,
                        BindingTarget::Opaque,
                    );
                }
                for field in ["parameters", "parameter", "type_parameters"] {
                    if let Some(parameters) = node.child_by_field_name(field) {
                        self.pattern(parameters, inner);
                    }
                }
                if let Some(parameters) = node.child_by_field_name("parameters") {
                    self.parameter_types(parameters, inner);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.visit(body, inner);
                }
                if let Some(return_type) = node.child_by_field_name("return_type") {
                    self.visit(return_type, inner);
                }
                self.current_caller = previous_caller;
                return;
            }
            "statement_block" | "class_body" | "catch_clause" | "for_statement"
            | "for_in_statement" => {
                let inner = self.scope(scope, false);
                if node.kind() == "catch_clause"
                    && let Some(parameter) = node.child_by_field_name("parameter")
                {
                    self.pattern(parameter, inner);
                }
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    self.visit(child, inner);
                }
                return;
            }
            "variable_declarator" => {
                let Some(name) = node.child_by_field_name("name") else {
                    return;
                };
                let parent = node.parent();
                let is_const =
                    parent.is_some_and(|p| self.text(p).trim_start().starts_with("const "));
                let mut binding_scope = scope;
                if parent.is_some_and(|p| p.kind() == "variable_declaration") {
                    while !self.scopes[binding_scope].function {
                        binding_scope = self.scopes[binding_scope].parent.unwrap_or(0);
                    }
                }
                if name.kind() == "identifier" && is_const {
                    let kind = if node.child_by_field_name("value").is_some_and(|n| {
                        matches!(
                            n.kind(),
                            "arrow_function" | "function_expression" | "generator_function"
                        )
                    }) {
                        DefinitionKind::Function
                    } else {
                        DefinitionKind::Constant
                    };
                    self.definition(name, node, kind, binding_scope, Namespace::Value);
                } else {
                    self.pattern(name, binding_scope);
                }
                if let Some(value) = node.child_by_field_name("value") {
                    self.visit(value, scope);
                }
                if let Some(annotation) = node.child_by_field_name("type") {
                    self.visit(annotation, scope);
                }
                return;
            }
            "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration" => {
                let kind = if matches!(
                    node.kind(),
                    "class_declaration" | "abstract_class_declaration"
                ) {
                    DefinitionKind::Class
                } else {
                    DefinitionKind::Type
                };
                let namespace = if matches!(
                    node.kind(),
                    "interface_declaration" | "type_alias_declaration"
                ) {
                    Namespace::Type
                } else {
                    Namespace::Both
                };
                if let Some(name) = node.child_by_field_name("name") {
                    self.definition(name, node, kind, scope, namespace);
                }
                let inner = self.scope(scope, false);
                if let Some(parameters) = node.child_by_field_name("type_parameters") {
                    self.pattern(parameters, inner);
                }
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if Some(child) != node.child_by_field_name("name")
                        && Some(child) != node.child_by_field_name("type_parameters")
                    {
                        self.visit(child, inner);
                    }
                }
                return;
            }
            "identifier" | "type_identifier" | "shorthand_property_identifier" => {
                if reference_identifier(node) {
                    let namespace = if node.kind() == "type_identifier" {
                        Namespace::Type
                    } else {
                        Namespace::Value
                    };
                    let is_call = node.parent().is_some_and(|p| {
                        p.kind() == "call_expression"
                            && p.child_by_field_name("function") == Some(node)
                    });
                    self.pending.push(Pending {
                        name: self.text(node).into(),
                        scope,
                        line: node.start_position().row + 1,
                        namespace,
                        is_call,
                        caller: self.current_caller,
                    });
                }
                return;
            }
            // Namespace declarations and ambient declarations require additional
            // merge rules; never let their internal names become global bindings.
            "internal_module" | "module" | "ambient_declaration" => return,
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, scope);
        }
    }
    fn imports(&mut self, node: Node<'_>, scope: usize) {
        let Some(source) = node.child_by_field_name("source") else {
            return;
        };
        let Some(specifier) = string_value(self.text(source)) else {
            return;
        };
        let type_only = self.text(node).trim_start().starts_with("import type ");
        let mut cursor = node.walk();
        for clause in node
            .named_children(&mut cursor)
            .filter(|n| n.kind() == "import_clause")
        {
            let mut children = clause.walk();
            for child in clause.named_children(&mut children) {
                match child.kind() {
                    "identifier" => {
                        self.add_import(self.text(child), "default", &specifier, type_only, scope)
                    }
                    "namespace_import" => {
                        let mut nested = child.walk();
                        for name in child
                            .named_children(&mut nested)
                            .filter(|n| n.kind() == "identifier")
                        {
                            self.bind(
                                self.text(name),
                                scope,
                                Namespace::Both,
                                BindingTarget::Opaque,
                            );
                        }
                    }
                    "named_imports" => {
                        let mut nested = child.walk();
                        for item in child
                            .named_children(&mut nested)
                            .filter(|n| n.kind() == "import_specifier")
                        {
                            let Some(name) = item
                                .child_by_field_name("name")
                                .filter(|n| n.kind() == "identifier")
                            else {
                                continue;
                            };
                            let alias = item.child_by_field_name("alias").unwrap_or(name);
                            let only =
                                type_only || self.text(item).trim_start().starts_with("type ");
                            self.add_import(
                                self.text(alias),
                                self.text(name),
                                &specifier,
                                only,
                                scope,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    fn add_import(
        &mut self,
        local: &str,
        imported: &str,
        specifier: &str,
        type_only: bool,
        scope: usize,
    ) {
        let index = self.facts.imports.len();
        self.facts.imports.push(Import {
            local: local.into(),
            imported: imported.into(),
            specifier: specifier.into(),
            type_only,
        });
        self.bind(
            local,
            scope,
            if type_only {
                Namespace::Type
            } else {
                Namespace::Both
            },
            BindingTarget::Import(index),
        );
    }
    fn finish(mut self) -> FileFacts {
        if self.overflow
            || self.facts.definitions.len() > MAX_FACTS
            || self.pending.len() > MAX_FACTS
            || self.bindings.len() > MAX_FACTS
        {
            self.facts.definitions.clear();
            self.facts.imports.clear();
            return self.facts;
        }
        let mut bindings: HashMap<(usize, &str), Vec<&Binding>> = HashMap::new();
        for binding in &self.bindings {
            bindings
                .entry((binding.scope, binding.name.as_str()))
                .or_default()
                .push(binding);
        }
        for pending in &self.pending {
            let mut scope = Some(pending.scope);
            let mut target = None;
            while let Some(id) = scope {
                if let Some(candidates) = bindings.get(&(id, pending.name.as_str())) {
                    let mut matching = candidates
                        .iter()
                        .filter(|b| b.namespace.accepts(pending.namespace));
                    if let Some(first) = matching.next() {
                        if matching.next().is_none() {
                            target = Some(first.target.clone());
                        }
                        break;
                    }
                }
                scope = self.scopes[id].parent;
            }
            let Some(target) = target.filter(|t| !matches!(t, BindingTarget::Opaque)) else {
                continue;
            };
            self.facts.references.push(Reference {
                name: pending.name.clone(),
                line: pending.line,
                is_call: pending.is_call,
                namespace: pending.namespace,
                target,
                caller: pending.caller,
            });
        }
        self.facts.valid = true;
        self.facts
    }
}

fn end_line(node: Node<'_>) -> usize {
    node.end_position().row + usize::from(node.end_position().column > 0)
}
fn string_value(text: &str) -> Option<String> {
    let quote = text.as_bytes().first()?;
    if !matches!(quote, b'\'' | b'"') || text.as_bytes().last() != Some(quote) || text.len() < 2 {
        return None;
    }
    let value = &text[1..text.len() - 1];
    (!value.contains(['\\', '\n', '\r', '\0'])).then(|| value.to_owned())
}
fn reference_identifier(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "import_specifier"
        | "import_clause"
        | "namespace_import"
        | "export_specifier"
        | "labeled_statement"
        | "break_statement"
        | "continue_statement"
        | "jsx_opening_element"
        | "jsx_closing_element"
        | "jsx_self_closing_element" => false,
        "pair" | "pair_pattern" => parent.child_by_field_name("key") != Some(node),
        "member_expression" => parent.child_by_field_name("property") != Some(node),
        "nested_type_identifier" => false,
        "property_signature" | "method_signature" | "public_field_definition" => {
            parent.child_by_field_name("name") != Some(node)
        }
        _ => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Definition,
    Caller,
}
#[derive(Clone, Debug)]
pub struct Related {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub name: String,
    pub kind: DefinitionKind,
    pub relation: Relation,
    pub reference_path: String,
    pub reference_line: usize,
    pub target_path: String,
    pub target_line: usize,
}
#[derive(Clone)]
struct Edge {
    source_file: usize,
    reference: usize,
    target_file: usize,
    definition: usize,
}
pub struct NavigationIndex {
    files: Vec<(String, Arc<FileFacts>)>,
    paths: HashMap<String, usize>,
    incoming: HashMap<String, Vec<(usize, usize)>>,
}
impl NavigationIndex {
    pub fn new<'a>(files: impl IntoIterator<Item = (&'a str, &'a FileFacts)>) -> Self {
        Self::new_shared(
            files
                .into_iter()
                .map(|(path, facts)| (path, Arc::new(facts.clone()))),
        )
    }
    pub fn new_shared<'a>(files: impl IntoIterator<Item = (&'a str, Arc<FileFacts>)>) -> Self {
        let mut files = files
            .into_iter()
            .map(|(path, facts)| (path.to_owned(), facts))
            .collect::<Vec<_>>();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let paths = files
            .iter()
            .enumerate()
            .map(|(i, (p, _))| (p.clone(), i))
            .collect();
        let mut index = Self {
            incoming: HashMap::new(),
            files,
            paths,
        };
        // Postings only: cross-file resolution happens for the selected evidence,
        // not for every reference whenever a CLI process starts.
        for (file, (_, facts)) in index.files.iter().enumerate() {
            if !facts.valid {
                continue;
            }
            for (reference, item) in facts.references.iter().enumerate() {
                if !item.is_call || item.caller.is_none() {
                    continue;
                }
                let name = match item.target {
                    BindingTarget::Definition(id) => {
                        facts.definitions.get(id).map(|d| d.name.as_str())
                    }
                    BindingTarget::Import(id) => facts.imports.get(id).map(|i| i.imported.as_str()),
                    BindingTarget::Opaque => None,
                };
                if let Some(name) = name {
                    if let Some(postings) = index.incoming.get_mut(name) {
                        postings.push((file, reference));
                    } else {
                        index
                            .incoming
                            .insert(name.to_owned(), vec![(file, reference)]);
                    }
                }
            }
        }
        index
    }
    pub fn definitions(&self, path: &str) -> &[Definition] {
        self.paths
            .get(path)
            .map_or(&[], |file| self.files[*file].1.definitions.as_slice())
    }
    pub fn is_parsed(&self, path: &str) -> bool {
        supports(path)
            && self
                .paths
                .get(path)
                .is_some_and(|file| self.files[*file].1.valid)
    }
    pub fn definition(&self, path: &str, line: usize) -> Option<&Definition> {
        let file = *self.paths.get(path)?;
        self.files[file]
            .1
            .definitions
            .iter()
            .filter(|d| d.start_line <= line && line <= d.end_line)
            .min_by_key(|d| d.end_line - d.start_line)
    }
    fn resolve(&self, file: usize, reference: &Reference) -> Option<(usize, usize)> {
        match reference.target {
            BindingTarget::Definition(definition) => self.files[file]
                .1
                .definitions
                .get(definition)
                .map(|_| (file, definition)),
            BindingTarget::Import(import) => {
                let import = self.files[file].1.imports.get(import)?;
                if import.type_only && reference.namespace == Namespace::Value {
                    return None;
                }
                let target = self.module(file, &import.specifier)?;
                if !self.files[target].1.valid {
                    return None;
                }
                let mut definitions =
                    self.files[target]
                        .1
                        .definitions
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| {
                            d.exports.contains(&import.imported)
                                && d.namespace.accepts(reference.namespace)
                        });
                let (definition, _) = definitions.next()?;
                definitions.next().is_none().then_some((target, definition))
            }
            BindingTarget::Opaque => None,
        }
    }
    fn module(&self, file: usize, specifier: &str) -> Option<usize> {
        let directory = parent(&self.files[file].0);
        let bases = if specifier.starts_with("./") || specifier.starts_with("../") {
            vec![normalize(directory, specifier)?]
        } else {
            let (config_path, config) = self.nearest_config(directory)?;
            // An unknown inherited baseUrl changes every paths target. Only an
            // explicit own baseUrl makes own mappings independent of extends.
            if config.extends && config.base_url.is_none() {
                return None;
            }
            let root = normalize(
                parent(config_path),
                config.base_url.as_deref().unwrap_or("."),
            )?;
            let mut matched = config
                .paths
                .iter()
                .filter_map(|(pattern, targets)| {
                    pattern_match(pattern, specifier).map(|capture| (pattern, targets, capture))
                })
                .collect::<Vec<_>>();
            matched.sort_by_key(|(pattern, _, _)| {
                std::cmp::Reverse((
                    usize::from(!pattern.contains('*')),
                    pattern.split('*').next().unwrap_or("").len(),
                ))
            });
            if let Some((pattern, targets, capture)) = matched.first() {
                if matched.get(1).is_some_and(|(other, _, _)| {
                    other.contains('*') == pattern.contains('*')
                        && other.split('*').next().unwrap_or("").len()
                            == pattern.split('*').next().unwrap_or("").len()
                        && other != pattern
                }) {
                    return None;
                }
                targets
                    .iter()
                    .map(|target| {
                        if target.matches('*').count() > 1 {
                            None
                        } else {
                            normalize(&root, &target.replace('*', capture))
                        }
                    })
                    .collect::<Option<Vec<_>>>()?
            } else if config.base_url.is_some() {
                vec![normalize(&root, specifier)?]
            } else {
                return None;
            }
        };
        let candidates = bases
            .into_iter()
            .flat_map(|base| module_candidates(&base))
            .filter_map(|p| self.paths.get(&p).copied())
            .filter(|i| supports(&self.files[*i].0))
            .collect::<BTreeSet<_>>();
        (candidates.len() == 1).then(|| *candidates.first().unwrap())
    }
    fn nearest_config(&self, directory: &str) -> Option<(&str, &ModuleConfig)> {
        let mut directory = directory;
        loop {
            let path = if directory.is_empty() {
                "tsconfig.json".into()
            } else {
                format!("{directory}/tsconfig.json")
            };
            if let Some(i) = self.paths.get(&path) {
                return self.files[*i]
                    .1
                    .config
                    .as_ref()
                    .map(|config| (self.files[*i].0.as_str(), config));
            }
            if directory.is_empty() {
                return None;
            }
            directory = parent(directory);
        }
    }
    pub fn related(
        &self,
        path: &str,
        start_line: usize,
        end_line: usize,
        question: &str,
        limit: usize,
    ) -> Vec<Related> {
        let Some(&file) = self.paths.get(path) else {
            return vec![];
        };
        let mut candidates = vec![];
        let mut seen = BTreeSet::new();
        for (id, reference) in self.files[file].1.references.iter().enumerate() {
            if !(start_line..=end_line).contains(&reference.line) {
                continue;
            }
            let Some((target_file, target_definition)) = self.resolve(file, reference) else {
                continue;
            };
            let definition = &self.files[target_file].1.definitions[target_definition];
            if target_file == file
                && definition.start_line <= end_line
                && definition.end_line >= start_line
            {
                continue;
            }
            if !seen.insert((target_file, target_definition)) {
                continue;
            }
            candidates.push(self.related_edge(
                &Edge {
                    source_file: file,
                    reference: id,
                    target_file,
                    definition: target_definition,
                },
                Relation::Definition,
            ));
        }
        // Only true call expressions become callers; a mention, type annotation,
        // or method receiver is not evidence of invoking the winning function.
        let mut remaining = 2048usize;
        for (id, definition) in
            self.files[file]
                .1
                .definitions
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    d.kind == DefinitionKind::Function
                        && d.start_line <= end_line
                        && d.end_line >= start_line
                })
        {
            let names = std::iter::once(&definition.name)
                .chain(definition.exports.iter())
                .collect::<BTreeSet<_>>();
            for (source_file, reference_id) in names
                .into_iter()
                .flat_map(|name| self.incoming.get(name).into_iter().flatten())
            {
                if remaining == 0 {
                    break;
                }
                remaining -= 1;
                let reference = &self.files[*source_file].1.references[*reference_id];
                if self.resolve(*source_file, reference) != Some((file, id)) {
                    continue;
                }
                let Some(caller) = reference.caller else {
                    continue;
                };
                let caller_definition = &self.files[*source_file].1.definitions[caller];
                if *source_file == file
                    && caller_definition.start_line <= end_line
                    && caller_definition.end_line >= start_line
                {
                    continue;
                }
                if !seen.insert((*source_file, caller)) {
                    continue;
                }
                candidates.push(self.related_edge(
                    &Edge {
                        source_file: *source_file,
                        reference: *reference_id,
                        target_file: file,
                        definition: id,
                    },
                    Relation::Caller,
                ));
            }
        }
        let terms = question
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|s| s.len() > 2)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let named_symbols = question
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect::<BTreeSet<_>>();
        candidates.sort_by_cached_key(|r| {
            let name = r.name.to_ascii_lowercase();
            let path = r.path.to_ascii_lowercase();
            let name_relevance = terms
                .iter()
                .filter(|term| name.contains(term.as_str()))
                .count();
            let path_relevance = terms
                .iter()
                .filter(|term| path.contains(term.as_str()))
                .count();
            let evidence_class = if r.relation == Relation::Caller {
                1
            } else if r.kind == DefinitionKind::Type {
                2
            } else {
                0
            };
            (
                // An explicitly named symbol wins. Otherwise, first explain
                // the implementation's runtime dependencies, then its callers,
                // then static annotations. A caller with a descriptive name
                // must not crowd out the code this implementation actually uses.
                std::cmp::Reverse(named_symbols.contains(&name)),
                evidence_class,
                std::cmp::Reverse(name_relevance),
                std::cmp::Reverse(path_relevance),
                usize::from(r.relation == Relation::Caller),
                r.path.clone(),
                r.start_line,
            )
        });
        candidates.truncate(limit.min(8));
        candidates
    }
    fn related_edge(&self, edge: &Edge, relation: Relation) -> Related {
        let source = &self.files[edge.source_file];
        let reference = &source.1.references[edge.reference];
        let target = &self.files[edge.target_file];
        let target_definition = &target.1.definitions[edge.definition];
        let (path, definition) = if relation == Relation::Definition {
            (&target.0, target_definition)
        } else {
            (&source.0, &source.1.definitions[reference.caller.unwrap()])
        };
        Related {
            path: path.clone(),
            start_line: definition.start_line,
            end_line: definition.end_line,
            name: definition.name.clone(),
            kind: definition.kind,
            relation,
            reference_path: source.0.clone(),
            reference_line: reference.line,
            target_path: target.0.clone(),
            target_line: target_definition.start_line,
        }
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}
fn normalize(base: &str, path: &str) -> Option<String> {
    if path.starts_with('/') || path.contains(['\\', ':', '\0']) {
        return None;
    }
    let mut parts = base
        .split('/')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}
fn pattern_match(pattern: &str, specifier: &str) -> Option<String> {
    if pattern.matches('*').count() > 1 {
        return None;
    }
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        specifier
            .strip_prefix(prefix)?
            .strip_suffix(suffix)
            .map(str::to_owned)
    } else {
        (pattern == specifier).then(String::new)
    }
}
fn module_candidates(base: &str) -> Vec<String> {
    if base.ends_with(".js") {
        let stem = base.strip_suffix(".js").unwrap();
        return ["ts", "tsx", "js", "jsx"]
            .map(|ext| format!("{stem}.{ext}"))
            .to_vec();
    }
    if base.ends_with(".mjs") || base.ends_with(".cjs") {
        let stem = &base[..base.len() - 4];
        let ext = if base.ends_with(".mjs") { "mts" } else { "cts" };
        return vec![format!("{stem}.{ext}"), base.into()];
    }
    if supports(base) {
        return vec![base.into()];
    }
    if base
        .rsplit('/')
        .next()
        .is_some_and(|name| name.contains('.'))
    {
        return vec![];
    }
    ["ts", "tsx", "js", "jsx"]
        .into_iter()
        .flat_map(|ext| [format!("{base}.{ext}"), format!("{base}/index.{ext}")])
        .collect()
}
fn parse_config(text: &str) -> Option<ModuleConfig> {
    // JSONC comments and trailing commas are common in tsconfig. Preserve string
    // contents, remove only lexical comments/commas outside strings.
    let bytes = text.as_bytes();
    let mut output = bytes.to_vec();
    let mut i = 0;
    let mut string = false;
    while i < bytes.len() {
        if string {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                string = false;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            string = true;
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                output[i] = b' ';
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            output[i] = b' ';
            output[i + 1] = b' ';
            i += 2;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i..].starts_with(b"*/") {
                    output[i] = b' ';
                    output[i + 1] = b' ';
                    i += 2;
                    closed = true;
                    break;
                }
                if bytes[i] != b'\n' {
                    output[i] = b' ';
                }
                i += 1;
            }
            if !closed {
                return None;
            }
            continue;
        }
        i += 1;
    }
    let mut string = false;
    let mut i = 0;
    while i < output.len() {
        if string {
            if output[i] == b'\\' {
                i += 2;
                continue;
            }
            if output[i] == b'"' {
                string = false;
            }
        } else if output[i] == b'"' {
            string = true;
        } else if output[i] == b','
            && output[i + 1..]
                .iter()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| matches!(b, b'}' | b']'))
        {
            output[i] = b' ';
        }
        i += 1;
    }
    let value: serde_json::Value = serde_json::from_slice(&output).ok()?;
    let options = value.get("compilerOptions");
    let mut config = ModuleConfig {
        base_url: options
            .and_then(|o| o.get("baseUrl"))
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        extends: value.get("extends").is_some(),
        ..ModuleConfig::default()
    };
    if let Some(paths) = options.and_then(|o| o.get("paths")) {
        for (name, targets) in paths.as_object()? {
            config.paths.insert(
                name.clone(),
                targets
                    .as_array()?
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
            );
        }
    }
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_export_parser_spans_include_export_keyword() {
        let source = "export\nfunction choose(ready: boolean): string | null {\n if (!ready) return null;\n return '/ready';\n}\n";
        let facts = NavigationPreparer::default().prepare("decision.ts", source);
        assert!(facts.valid);
        assert_eq!(facts.definitions[0].start_line, 1);
        assert_eq!(facts.definitions[0].end_line, 5);
    }

    fn index(files: &[(&str, &str)]) -> NavigationIndex {
        let mut preparer = NavigationPreparer::default();
        let facts = files
            .iter()
            .map(|(path, text)| (*path, Arc::new(preparer.prepare(path, text))))
            .collect::<Vec<_>>();
        NavigationIndex::new_shared(facts)
    }
    #[test]
    fn validators_types_and_union_return_functions_have_real_bounds() {
        let text = "const CursorSchema = schema.object({ id: schema.string() });\ntype Cursor = { id: string };\nexport function decode(value: string): Cursor | null {\n  const parsed = CursorSchema.safeParse(value);\n  return parsed.success ? parsed.data : null;\n}\n";
        let index = index(&[("decode.ts", text)]);
        let function = index.definition("decode.ts", 3).unwrap();
        assert_eq!(
            (function.start_line, function.end_line, function.complete),
            (3, 6, true)
        );
        let related = index.related("decode.ts", 3, 6, "decode cursor", 2);
        assert_eq!(related.len(), 2);
        assert!(
            related
                .iter()
                .any(|r| r.name == "CursorSchema" && r.kind == DefinitionKind::Constant)
        );
        assert!(
            related
                .iter()
                .any(|r| r.name == "Cursor" && r.kind == DefinitionKind::Type)
        );
    }
    #[test]
    fn named_import_aliases_find_incoming_callers() {
        let index = index(&[
            (
                "lib/access.ts",
                "export function destination(user: User): string | null { return user.path; }",
            ),
            (
                "routes/page.ts",
                "import { destination as pick } from '../lib/access.js';\nexport async function loader() {\n return pick(user);\n}",
            ),
            (
                "other.ts",
                "export function destination() { return 'wrong'; }",
            ),
        ]);
        let related = index.related("lib/access.ts", 1, 1, "destination route", 2);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].path, "routes/page.ts");
        assert_eq!(related[0].relation, Relation::Caller);
        assert_eq!(related[0].reference_line, 3);
        assert_eq!(related[0].name, "loader");
        assert_eq!((related[0].start_line, related[0].end_line), (2, 4));
    }
    #[test]
    fn relative_index_imports_resolve_but_duplicate_extensions_abstain() {
        let files = [
            (
                "src/main.ts",
                "import { validate } from './validation';\nexport function run() { return validate(value); }",
            ),
            (
                "src/validation/index.ts",
                "export function validate(value: unknown) { return value; }",
            ),
        ];
        assert_eq!(
            index(&files)
                .related("src/main.ts", 2, 2, "validate", 2)
                .len(),
            1
        );
        let mut ambiguous = files.to_vec();
        ambiguous.push(("src/validation.ts", "export function validate() {}"));
        assert!(
            index(&ambiguous)
                .related("src/main.ts", 2, 2, "validate", 2)
                .is_empty()
        );
    }
    #[test]
    fn comments_strings_properties_and_shadowed_imports_are_not_calls() {
        let sources = [
            "import { check } from './check';\nexport function run(check: () => void) { check(); }",
            "import { check } from './check';\nexport function run() { const check = other; check(); }",
            "import { check } from './check';\nexport function run() { // check()\n return 'check()'; }",
            "import { check } from './check';\nexport function run() { return object.check(); }",
            "import { check } from './check';\nexport function run({check}: Props) { return check(); }",
        ];
        for source in sources {
            let index = index(&[
                ("caller.ts", source),
                ("check.ts", "export function check() {}"),
            ]);
            assert!(
                index.related("check.ts", 1, 1, "check", 2).is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn type_only_import_never_creates_value_dependency() {
        let index = index(&[
            (
                "main.ts",
                "import type { Cursor } from './types';\nexport function decode(): Cursor { return Cursor(); }",
            ),
            ("types.ts", "export type Cursor = { id: string };"),
        ]);
        let related = index.related("main.ts", 2, 2, "decode cursor", 2);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].kind, DefinitionKind::Type);
        let facts = &index.files[*index.paths.get("main.ts").unwrap()].1;
        assert!(!facts.references.iter().any(|r| r.is_call));
    }
    #[test]
    fn own_nearest_jsonc_alias_config_is_used_and_unknown_extends_abstains() {
        let source = "import { choose } from '~/rules';\nexport const loader = () => choose();";
        let base = [
            ("apps/web/routes/home.ts", source),
            ("apps/web/app/rules.ts", "export function choose() {}"),
            (
                "apps/web/tsconfig.json",
                "{ // config\n \"compilerOptions\": {\"paths\": {\"~/*\": [\"./app/*\"],},},}",
            ),
        ];
        let index = index(&base);
        assert_eq!(
            index
                .related("apps/web/app/rules.ts", 1, 1, "route", 2)
                .len(),
            1
        );
        let mut inherited = base;
        inherited[2].1 = "{\"extends\":\"@shared/config\",\"compilerOptions\":{\"paths\":{\"~/*\":[\"./app/*\"]}}}";
        assert!(
            super::tests::index(&inherited)
                .related("apps/web/app/rules.ts", 1, 1, "route", 2)
                .is_empty()
        );
        inherited[2].1 = "{\"extends\":\"@shared/config\",\"compilerOptions\":{\"baseUrl\":\".\",\"paths\":{\"~/*\":[\"./app/*\"]}}}";
        assert_eq!(
            super::tests::index(&inherited)
                .related("apps/web/app/rules.ts", 1, 1, "route", 2)
                .len(),
            1
        );
    }
    #[test]
    fn unresolved_imports_never_fall_back_to_same_name() {
        for import in [
            "import { helper } from '@external/package';",
            "import { helper } from './barrel';",
            "import * as helper from './target';",
        ] {
            let source = format!("{import}\nexport function run() {{ return helper(); }}");
            let index = index(&[
                ("main.ts", &source),
                ("target.ts", "export function helper() {}"),
                ("barrel.ts", "export { helper } from './target';"),
            ]);
            assert!(
                index.related("main.ts", 2, 2, "helper", 2).is_empty(),
                "{import}"
            );
        }
    }
    #[test]
    fn invalid_syntax_is_not_used_as_semantic_evidence() {
        let index = index(&[
            ("bad.ts", "export function bad( { helper()"),
            ("helper.ts", "export function helper() {}"),
        ]);
        assert!(index.definitions("bad.ts").is_empty());
        assert!(index.related("bad.ts", 1, 1, "helper", 2).is_empty());
    }
    #[test]
    fn cache_roundtrip_binds_facts_to_fresh_path_and_content() {
        let text = "export function decode() { return 1; }";
        let facts = NavigationPreparer::default().prepare("a.ts", text);
        let serialized = postcard::to_allocvec(&facts).unwrap();
        let restored: FileFacts = postcard::from_bytes(&serialized).unwrap();
        assert!(restored.matches_source("a.ts", text));
        assert!(!restored.matches_source("b.ts", text));
        assert!(!restored.matches_source("a.ts", &text.replace('1', "2")));
    }

    #[test]
    fn captured_digests_preserve_identity_and_structural_validation() {
        let text = "export function decode() {\n return 1;\n}\n";
        let facts = NavigationPreparer::default().prepare("a.ts", text);
        let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        assert!(facts.matches_captured_source("a.ts", text.len(), &digest));
        assert!(!facts.matches_captured_source("b.ts", text.len(), &digest));
        assert!(!facts.matches_captured_source("a.ts", text.len() + 1, &digest));
        let changed: [u8; 32] = Sha256::digest(text.replace('1', "2").as_bytes()).into();
        assert!(!facts.matches_captured_source("a.ts", text.len(), &changed));
        let mut invalid = facts.clone();
        invalid.definitions[0].end_byte = text.len() + 1;
        assert!(!invalid.matches_captured_source("a.ts", text.len(), &digest));
        let mut invalid = facts;
        invalid.lines = 0;
        assert!(!invalid.matches_captured_source("a.ts", text.len(), &digest));
    }
    #[test]
    fn default_exports_and_tsx_callers_work_without_jsx_name_guesses() {
        let index = index(&[
            ("get.js", "export default function load() { return 1; }"),
            (
                "view.tsx",
                "import load from './get.js';\nexport const View = () => { const value = load(); return <div>{value}</div>; };",
            ),
        ]);
        let related = index.related("get.js", 1, 1, "view", 2);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].name, "View");
    }

    #[test]
    fn deep_syntax_and_oversized_sources_fall_back_without_partial_facts() {
        let text = format!(
            "export function run() {{ return {}1{}; }}",
            "(".repeat(300),
            ")".repeat(300)
        );
        let facts = NavigationPreparer::default().prepare("nested.ts", &text);
        assert!(!facts.valid);
        assert!(facts.definitions.is_empty());
        assert!(facts.references.is_empty());
        assert!(facts.matches_source("nested.ts", &text));
        let huge = format!("export function run() {{}}\n{}", " ".repeat(256 * 1024));
        assert!(
            NavigationPreparer::default()
                .prepare("large.ts", &huge)
                .definitions
                .is_empty()
        );
    }

    #[test]
    fn type_and_value_namespaces_resolve_independently() {
        let index = index(&[(
            "main.ts",
            "type Cursor = { id: string };\nconst Cursor = schema.string();\nexport function decode(value: Cursor) { return Cursor.parse(value); }",
        )]);
        let related = index.related("main.ts", 3, 3, "cursor", 8);
        assert_eq!(related.len(), 2);
        assert!(
            related
                .iter()
                .any(|r| r.kind == DefinitionKind::Type && r.start_line == 1)
        );
        assert!(
            related
                .iter()
                .any(|r| r.kind == DefinitionKind::Constant && r.start_line == 2)
        );
    }

    #[test]
    fn enclosing_callers_are_not_attributed_to_an_unrelated_nested_callback() {
        let index = index(&[
            ("target.ts", "export function target() {}"),
            (
                "caller.ts",
                "import { target } from './target';\nexport function outer() {\n  [1].map(() => target());\n}",
            ),
        ]);
        // An anonymous callback has no independently bounded named declaration.
        // Its call is deliberately omitted instead of being asserted as outer's.
        assert!(index.related("target.ts", 1, 1, "target", 2).is_empty());
    }

    #[test]
    fn symbol_relevance_and_runtime_ties_beat_broad_filename_matches() {
        let index = index(&[
            (
                "src/stream.ts",
                "import { FrameValidator, EncodedFrameValidator } from './rules';\ntype FrameShape = { id: string };\ntype FrameResult = FrameShape | null;\nexport function decode(value: FrameShape): FrameResult {\n const decoded = EncodedFrameValidator.parse(value);\n return FrameValidator.parse(decoded);\n}",
            ),
            (
                "src/rules.ts",
                "export const FrameValidator = objectRule({ id: validIdentifier() });\nexport const EncodedFrameValidator = encodedStringRule();",
            ),
        ]);
        let related = index.related(
            "src/stream.ts",
            4,
            7,
            "how does stream decode an encoded frame",
            2,
        );
        assert_eq!(
            related.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["EncodedFrameValidator", "FrameValidator"]
        );
        let explicit = index.related("src/stream.ts", 4, 7, "what is FrameShape", 2);
        assert_eq!(explicit[0].name, "FrameShape");
        assert_eq!(explicit[0].kind, DefinitionKind::Type);
    }

    #[test]
    fn descriptive_caller_does_not_displace_direct_runtime_evidence() {
        let index = index(&[
            (
                "src/decode.ts",
                "import { FrameValidator, EncodedFrameValidator } from './rules';\nexport function decode(value: string) {\n const decoded = EncodedFrameValidator.parse(value);\n return FrameValidator.parse(decoded);\n}",
            ),
            (
                "src/rules.ts",
                "export const FrameValidator = objectRule({ id: validIdentifier() });\nexport const EncodedFrameValidator = encodedStringRule();",
            ),
            (
                "src/query.ts",
                "import { decode } from './decode';\nexport function queryStreamEvents() { return decode(input); }",
            ),
        ]);
        let related = index.related(
            "src/decode.ts",
            2,
            5,
            "how do stream events decode encoded frame",
            2,
        );
        assert_eq!(related.len(), 2);
        assert!(related.iter().all(|r| r.relation == Relation::Definition));
        assert!(related.iter().any(|r| r.name == "FrameValidator"));
        assert!(related.iter().any(|r| r.name == "EncodedFrameValidator"));
        let explicit = index.related("src/decode.ts", 2, 5, "show queryStreamEvents", 2);
        assert_eq!(explicit[0].name, "queryStreamEvents");
        assert_eq!(explicit[0].relation, Relation::Caller);
    }
}
