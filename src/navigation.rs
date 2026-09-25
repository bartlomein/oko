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
/// Files of a supported language are parsed up to this size. Bigger than the
/// lexical cap because a generated or bundled source file can hold the
/// definitions agents ask for most; see `search::file_byte_limit`.
pub const PARSED_FILE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionKind {
    Function,
    Constant,
    Type,
    Class,
    Method,
    Module,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Definition {
    pub name: String,
    /// The name with its containers: `Server.handle`, `routes.list`. Equal to
    /// `name` for a top-level definition.
    pub qualified: String,
    /// Index of the enclosing class or object definition in the same file.
    pub container: Option<usize>,
    pub start_line: usize,
    pub end_line: usize,
    pub complete: bool,
    pub kind: DefinitionKind,
    exports: Vec<String>,
    start_byte: usize,
    end_byte: usize,
    namespace: Namespace,
}

impl Definition {
    pub fn exported(&self) -> bool {
        !self.exports.is_empty()
    }
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
    /// The file had syntax errors: definitions were kept, relationships were not.
    partial: bool,
    /// The language's visitor records definitions only (no bindings, imports
    /// or references): relationships come from the lexical fallback.
    definitions_only: bool,
}

impl FileFacts {
    /// Parsed, with definitions worth aligning chunks to.
    pub fn has_definitions(&self) -> bool {
        self.valid && !self.definitions.is_empty()
    }
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
            && (!(self.partial || self.definitions_only) || self.references.is_empty())
            && self.definitions.iter().all(|d| {
                d.start_line > 0
                    && d.start_line <= d.end_line
                    && d.end_line <= self.lines
                    && d.start_byte <= d.end_byte
                    && d.end_byte <= self.bytes
                    && !d.name.is_empty()
                    && d.qualified.ends_with(d.name.as_str())
                    && d.container.is_none_or(|i| i < self.definitions.len())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Language {
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Go,
    Rust,
    Ruby,
    Java,
    Kotlin,
}
impl Language {
    fn of(path: &str) -> Option<Self> {
        Some(match path.rsplit('.').next()? {
            "js" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "py" | "pyi" => Self::Python,
            "go" => Self::Go,
            "rs" => Self::Rust,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            "java" => Self::Java,
            "kt" | "kts" => Self::Kotlin,
            _ => return None,
        })
    }
    /// The JavaScript family has scopes, imports and references; the others
    /// record definitions only.
    fn relationships(self) -> bool {
        matches!(self, Self::JavaScript | Self::TypeScript | Self::Tsx)
    }
    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
        }
    }
}

/// A language whose definitions are parsed.
pub fn supports(path: &str) -> bool {
    Language::of(path).is_some()
}

#[derive(Default)]
pub struct NavigationPreparer {
    parsers: HashMap<Language, Parser>,
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
        let Some(language) = Language::of(path).filter(|_| text.len() <= PARSED_FILE_BYTES) else {
            return facts;
        };
        let parser = self.parsers.entry(language).or_insert_with(|| {
            let mut parser = Parser::new();
            parser
                .set_language(&language.grammar())
                .expect("bundled parser ABI");
            parser
        });
        let started = Instant::now();
        // 250 ms per 256 KiB: a big file gets proportionally longer before the
        // parse is abandoned, so a slow machine does not silently lose it.
        let budget = Duration::from_millis(250 * text.len().div_ceil(256 * 1024).max(1) as u64);
        let mut cancel = |_: &tree_sitter::ParseState| started.elapsed() > budget;
        let mut read = |offset: usize, _: tree_sitter::Point| &text.as_bytes()[offset..];
        let Some(tree) = parser.parse_with_options(
            &mut read,
            None,
            Some(ParseOptions::new().progress_callback(&mut cancel)),
        ) else {
            parser.reset();
            return facts;
        };
        // A partial syntax tree can misidentify scopes. Keep the lexical fallback
        // for relationships in that case, but keep the definitions: a name and
        // its lines are still right wherever the definition's own span parsed.
        facts.partial = tree.root_node().has_error();
        if !language.relationships() {
            facts.definitions_only = true;
            let mut visitor = Definitions {
                text,
                language,
                facts,
                depth: 0,
            };
            visitor.visit(tree.root_node(), None);
            visitor.facts.valid = true;
            return visitor.facts;
        }
        let mut collector = Collector::new(text, facts);
        collector.visit(tree.root_node(), 0);
        collector.finish()
    }
}

/// Definitions of a language without the JavaScript scope machinery: one
/// walk that records functions, methods, classes, types, modules and
/// constants with their containers and qualified names.
struct Definitions<'a> {
    text: &'a str,
    language: Language,
    facts: FileFacts,
    depth: usize,
}
impl<'a> Definitions<'a> {
    fn text(&self, node: Node<'_>) -> &'a str {
        &self.text[node.byte_range()]
    }
    fn push(
        &mut self,
        name: &str,
        qualified: Option<String>,
        kind: DefinitionKind,
        container: Option<usize>,
        span: Node<'_>,
        exported: bool,
    ) -> usize {
        let qualified = qualified.unwrap_or_else(|| {
            match container.and_then(|i| self.facts.definitions.get(i)) {
                Some(owner) => format!("{}.{name}", owner.qualified),
                None => name.to_owned(),
            }
        });
        let id = self.facts.definitions.len();
        self.facts.definitions.push(Definition {
            name: name.to_owned(),
            qualified,
            container,
            start_line: span.start_position().row + 1,
            end_line: end_line(span),
            complete: !span.has_error(),
            kind,
            exports: if exported {
                vec![name.to_owned()]
            } else {
                vec![]
            },
            start_byte: span.start_byte(),
            end_byte: span.end_byte(),
            namespace: Namespace::Both,
        });
        id
    }
    fn children(&mut self, node: Node<'_>, container: Option<usize>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, container);
        }
    }
    fn visit(&mut self, node: Node<'_>, container: Option<usize>) {
        if self.depth >= 256 || self.facts.definitions.len() > MAX_FACTS {
            return;
        }
        self.depth += 1;
        match self.language {
            Language::Python => self.python(node, container),
            Language::Go => self.go(node, container),
            Language::Rust => self.rust(node, container),
            Language::Ruby => self.ruby(node, container),
            Language::Java => self.java(node, container),
            Language::Kotlin => self.kotlin(node, container),
            _ => {}
        }
        self.depth -= 1;
    }
    fn name_of(&self, node: Node<'_>) -> Option<&'a str> {
        node.child_by_field_name("name").map(|name| self.text(name))
    }
    fn python(&mut self, node: Node<'_>, container: Option<usize>) {
        match node.kind() {
            "function_definition" | "class_definition" => {
                let Some(name) = self.name_of(node) else {
                    return;
                };
                // Decorators belong to the definition.
                let span = node
                    .parent()
                    .filter(|p| p.kind() == "decorated_definition")
                    .unwrap_or(node);
                let is_class = node.kind() == "class_definition";
                let kind = if is_class {
                    DefinitionKind::Class
                } else if container.is_some() {
                    DefinitionKind::Method
                } else {
                    DefinitionKind::Function
                };
                let id = self.push(name, None, kind, container, span, !name.starts_with('_'));
                if let Some(body) = node.child_by_field_name("body") {
                    // Methods belong to their class; a nested function to no one.
                    self.children(body, is_class.then_some(id));
                }
            }
            _ => self.children(node, container),
        }
    }
    fn go(&mut self, node: Node<'_>, container: Option<usize>) {
        let exported = |name: &str| name.starts_with(|c: char| c.is_uppercase());
        match node.kind() {
            "function_declaration" => {
                if let Some(name) = self.name_of(node) {
                    self.push(
                        name,
                        None,
                        DefinitionKind::Function,
                        None,
                        node,
                        exported(name),
                    );
                }
            }
            "method_declaration" => {
                let Some(name) = self.name_of(node) else {
                    return;
                };
                // The receiver type qualifies the method: `Context.Next`.
                let receiver = node
                    .child_by_field_name("receiver")
                    .and_then(|list| list.named_child(0))
                    .and_then(|parameter| parameter.child_by_field_name("type"))
                    .map(|mut ty| {
                        while let Some(inner) = ty.child_by_field_name("type") {
                            ty = inner;
                        }
                        self.text(ty).trim_start_matches('*').to_owned()
                    });
                let qualified = receiver.map(|receiver| format!("{receiver}.{name}"));
                self.push(
                    name,
                    qualified,
                    DefinitionKind::Method,
                    None,
                    node,
                    exported(name),
                );
            }
            "type_declaration" => {
                let specs: Vec<Node<'_>> = {
                    let mut cursor = node.walk();
                    node.named_children(&mut cursor)
                        .filter(|c| matches!(c.kind(), "type_spec" | "type_alias"))
                        .collect()
                };
                for spec in &specs {
                    let Some(name) = self.name_of(*spec) else {
                        continue;
                    };
                    let kind = match spec.child_by_field_name("type").map(|t| t.kind()) {
                        Some("struct_type") => DefinitionKind::Class,
                        _ => DefinitionKind::Type,
                    };
                    let span = if specs.len() == 1 { node } else { *spec };
                    self.push(name, None, kind, None, span, exported(name));
                }
            }
            "const_declaration" | "var_declaration" if container.is_none() => {
                let mut cursor = node.walk();
                for spec in node.named_children(&mut cursor) {
                    if let Some(name) = self.name_of(spec) {
                        self.push(
                            name,
                            None,
                            DefinitionKind::Constant,
                            None,
                            node,
                            exported(name),
                        );
                    }
                }
            }
            "source_file" => self.children(node, container),
            _ => {}
        }
    }
    fn rust(&mut self, node: Node<'_>, container: Option<usize>) {
        let public = |node: Node<'_>| {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .any(|c| c.kind() == "visibility_modifier")
        };
        match node.kind() {
            "function_item" | "function_signature_item" => {
                if let Some(name) = self.name_of(node) {
                    let kind = if container.is_some() {
                        DefinitionKind::Method
                    } else {
                        DefinitionKind::Function
                    };
                    self.push(name, None, kind, container, node, public(node));
                }
            }
            "struct_item" | "enum_item" | "union_item" | "type_item" => {
                if let Some(name) = self.name_of(node) {
                    self.push(name, None, DefinitionKind::Type, None, node, public(node));
                }
            }
            "trait_item" => {
                if let Some(name) = self.name_of(node) {
                    let id = self.push(name, None, DefinitionKind::Type, None, node, public(node));
                    if let Some(body) = node.child_by_field_name("body") {
                        self.children(body, Some(id));
                    }
                }
            }
            "impl_item" => {
                // `impl<'a> Searcher<'a>` and `impl Matcher for Searcher` both
                // define members of `Searcher`.
                let name = node.child_by_field_name("type").map(|mut ty| {
                    while let Some(inner) = ty.child_by_field_name("type") {
                        ty = inner;
                    }
                    self.text(ty).to_owned()
                });
                if let Some(name) = name {
                    let id = self.push(&name, None, DefinitionKind::Class, None, node, true);
                    if let Some(body) = node.child_by_field_name("body") {
                        self.children(body, Some(id));
                    }
                }
            }
            "mod_item" => {
                if let Some(name) = self.name_of(node) {
                    self.push(name, None, DefinitionKind::Module, None, node, public(node));
                    if let Some(body) = node.child_by_field_name("body") {
                        self.children(body, None);
                    }
                }
            }
            "const_item" | "static_item" => {
                if let Some(name) = self.name_of(node) {
                    self.push(
                        name,
                        None,
                        DefinitionKind::Constant,
                        None,
                        node,
                        public(node),
                    );
                }
            }
            "macro_definition" => {
                if let Some(name) = self.name_of(node) {
                    self.push(name, None, DefinitionKind::Function, None, node, true);
                }
            }
            "source_file" => self.children(node, container),
            _ => {}
        }
    }
    fn ruby(&mut self, node: Node<'_>, container: Option<usize>) {
        match node.kind() {
            "class" | "module" => {
                let Some(name) = node.child_by_field_name("name") else {
                    return;
                };
                // `class Foo::Bar` defines `Bar` inside `Foo`.
                let full = self.text(name).replace("::", ".");
                let leaf = full.rsplit('.').next().unwrap_or(&full).to_owned();
                let qualified = match container.and_then(|i| self.facts.definitions.get(i)) {
                    Some(owner) => format!("{}.{full}", owner.qualified),
                    None => full,
                };
                let kind = if node.kind() == "class" {
                    DefinitionKind::Class
                } else {
                    DefinitionKind::Module
                };
                let id = self.push(&leaf, Some(qualified), kind, container, node, true);
                if let Some(body) = node.child_by_field_name("body") {
                    self.children(body, Some(id));
                }
            }
            "method" | "singleton_method" => {
                if let Some(name) = self.name_of(node) {
                    let kind = if container.is_some() {
                        DefinitionKind::Method
                    } else {
                        DefinitionKind::Function
                    };
                    self.push(name, None, kind, container, node, true);
                }
            }
            _ => self.children(node, container),
        }
    }
    fn kotlin(&mut self, node: Node<'_>, container: Option<usize>) {
        // Keywords and modifiers are unnamed children; annotations live in
        // `modifiers`, which the declaration's span already covers.
        let words = |node: Node<'_>| -> Vec<&'a str> {
            let mut cursor = node.walk();
            let mut words: Vec<&'a str> = node
                .children(&mut cursor)
                .filter(|c| !c.is_named())
                .map(|c| self.text(c))
                .collect();
            let mut cursor = node.walk();
            if let Some(m) = node.children(&mut cursor).find(|c| c.kind() == "modifiers") {
                let mut cursor = m.walk();
                words.extend(m.children(&mut cursor).map(|c| self.text(c)));
            }
            words
        };
        // Kotlin is public unless it says otherwise.
        let exported = |words: &[&str]| {
            !words
                .iter()
                .any(|w| matches!(*w, "private" | "internal" | "protected"))
        };
        let first_identifier = |node: Node<'_>| -> Option<&'a str> {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "identifier")
                .map(|c| self.text(c))
        };
        match node.kind() {
            "class_declaration" | "object_declaration" => {
                let Some(name) = self.name_of(node) else {
                    return;
                };
                let words = words(node);
                let kind = if words.contains(&"interface") {
                    DefinitionKind::Type
                } else {
                    DefinitionKind::Class
                };
                let id = self.push(name, None, kind, container, node, exported(&words));
                let mut cursor = node.walk();
                let bodies: Vec<Node<'_>> = node
                    .children(&mut cursor)
                    .filter(|c| matches!(c.kind(), "class_body" | "enum_class_body"))
                    .collect();
                for body in bodies {
                    self.children(body, Some(id));
                }
            }
            // `companion object` members are addressed through the class.
            "companion_object" => {
                let mut cursor = node.walk();
                let bodies: Vec<Node<'_>> = node
                    .children(&mut cursor)
                    .filter(|c| c.kind() == "class_body")
                    .collect();
                for body in bodies {
                    self.children(body, container);
                }
            }
            "function_declaration" => {
                if let Some(name) = self.name_of(node) {
                    let kind = if container.is_some() {
                        DefinitionKind::Method
                    } else {
                        DefinitionKind::Function
                    };
                    let exported = exported(&words(node));
                    self.push(name, None, kind, container, node, exported);
                }
            }
            "secondary_constructor" => {
                if let Some(owner) = container.and_then(|i| self.facts.definitions.get(i)) {
                    let name = owner.name.clone();
                    let exported = exported(&words(node));
                    self.push(
                        &name,
                        None,
                        DefinitionKind::Method,
                        container,
                        node,
                        exported,
                    );
                }
            }
            // Properties are the named values agents ask for
            // (`JavalinConfig.routes`, `const val DEFAULT_PORT`, the `var`
            // settings of a config class); locals are never reached.
            "property_declaration" => {
                let words = words(node);
                if !words.contains(&"val") && !words.contains(&"var") {
                    return;
                }
                let mut cursor = node.walk();
                let names: Vec<&'a str> = node
                    .children(&mut cursor)
                    .filter(|c| c.kind() == "variable_declaration")
                    .filter_map(first_identifier)
                    .collect();
                for name in names {
                    self.push(
                        name,
                        None,
                        DefinitionKind::Constant,
                        container,
                        node,
                        exported(&words),
                    );
                }
            }
            "enum_entry" => {
                if let Some(name) = first_identifier(node) {
                    self.push(name, None, DefinitionKind::Constant, container, node, true);
                }
            }
            "type_alias" => {
                if let Some(name) = node.child_by_field_name("type").map(|n| self.text(n)) {
                    let exported = exported(&words(node));
                    self.push(name, None, DefinitionKind::Type, container, node, exported);
                }
            }
            // Function bodies are not visited; statement and member wrappers
            // pass through.
            "function_body" | "block" => {}
            _ => self.children(node, container),
        }
    }
    fn java(&mut self, node: Node<'_>, container: Option<usize>) {
        // Annotations sit inside the declaration's `modifiers` child, so a
        // declaration's own span already covers them.
        let modifiers = |node: Node<'_>| -> Vec<&'a str> {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "modifiers")
                .map(|m| {
                    let mut cursor = m.walk();
                    m.children(&mut cursor)
                        .filter(|c| !c.is_named())
                        .map(|c| self.text(c))
                        .collect()
                })
                .unwrap_or_default()
        };
        // Interface members are public without saying so.
        let in_interface = |node: Node<'_>| {
            node.parent()
                .is_some_and(|p| matches!(p.kind(), "interface_body" | "annotation_type_body"))
        };
        match node.kind() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "annotation_type_declaration" => {
                let Some(name) = self.name_of(node) else {
                    return;
                };
                let kind = if node.kind() == "interface_declaration" {
                    DefinitionKind::Type
                } else {
                    DefinitionKind::Class
                };
                let exported = modifiers(node).contains(&"public") || in_interface(node);
                let id = self.push(name, None, kind, container, node, exported);
                if let Some(body) = node.child_by_field_name("body") {
                    self.children(body, Some(id));
                }
            }
            "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration" => {
                if let Some(name) = self.name_of(node) {
                    let exported = modifiers(node).contains(&"public") || in_interface(node);
                    let kind = if container.is_some() {
                        DefinitionKind::Method
                    } else {
                        DefinitionKind::Function
                    };
                    self.push(name, None, kind, container, node, exported);
                }
            }
            // `static final` fields and interface constants are the named
            // values agents ask for; instance fields are not definitions.
            "field_declaration" | "constant_declaration" => {
                let mods = modifiers(node);
                let constant = node.kind() == "constant_declaration"
                    || (mods.contains(&"static") && mods.contains(&"final"));
                if !constant {
                    return;
                }
                let exported = mods.contains(&"public") || in_interface(node);
                let mut cursor = node.walk();
                let declarators: Vec<Node<'_>> = node
                    .children_by_field_name("declarator", &mut cursor)
                    .collect();
                for declarator in declarators {
                    if let Some(name) = self.name_of(declarator) {
                        self.push(
                            name,
                            None,
                            DefinitionKind::Constant,
                            container,
                            node,
                            exported,
                        );
                    }
                }
            }
            "enum_constant" => {
                if let Some(name) = self.name_of(node) {
                    self.push(name, None, DefinitionKind::Constant, container, node, true);
                }
            }
            // Method bodies are not visited, so local and anonymous classes
            // stay out of the index; wrappers such as `enum_body_declarations`
            // pass through.
            _ => self.children(node, container),
        }
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
    /// The class or object definition whose body is being visited.
    current_container: Option<usize>,
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
            current_container: None,
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
        self.definition_named(self.text(name), node, kind, scope, namespace, true)
    }
    /// Record a definition. `bind` makes the name a scope binding, which is
    /// right for declarations and wrong for members (a method is a property of
    /// its class, not a name in scope).
    fn definition_named(
        &mut self,
        name: &str,
        node: Node<'_>,
        kind: DefinitionKind,
        scope: usize,
        namespace: Namespace,
        bind: bool,
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
                name.into()
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
            exports.push(name.into());
        }
        let start = export_prefix.unwrap_or(span);
        let id = self.facts.definitions.len();
        let container = self.current_container;
        let qualified = match container.and_then(|i| self.facts.definitions.get(i)) {
            Some(owner) => format!("{}.{name}", owner.qualified),
            None => name.into(),
        };
        self.facts.definitions.push(Definition {
            name: name.into(),
            qualified,
            container,
            start_line: start.start_position().row + 1,
            end_line: end_line(span),
            complete: !span.has_error(),
            kind,
            exports,
            start_byte: start.start_byte(),
            end_byte: span.end_byte(),
            namespace,
        });
        if bind {
            self.bind(name, scope, namespace, BindingTarget::Definition(id));
        }
        id
    }
    /// The name of a class member or object property, when it is a plain name.
    fn member_name(&self, node: Node<'_>) -> Option<&'a str> {
        let name = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("key"))?;
        match name.kind() {
            "property_identifier" | "private_property_identifier" | "identifier" => {
                Some(self.text(name))
            }
            "string" => string_value(self.text(name)).and_then(|value| {
                let inner = &self.text(name)[1..1 + value.len()];
                (inner == value).then_some(inner)
            }),
            _ => None,
        }
    }
    /// Visit a class body or object literal with `owner` as the container of
    /// the members defined directly inside it.
    fn visit_members(&mut self, node: Node<'_>, scope: usize, owner: Option<usize>) {
        let previous = self.current_container;
        self.current_container = owner;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, scope);
        }
        self.current_container = previous;
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
                let parent = node.parent();
                let is_value_of = |kind: &str| {
                    parent.is_some_and(|parent| {
                        parent.kind() == kind && parent.child_by_field_name("value") == Some(node)
                    })
                };
                let owner = if matches!(
                    node.kind(),
                    "function_declaration" | "generator_function_declaration"
                ) {
                    match node.child_by_field_name("name") {
                        Some(name) => Some(self.definition(
                            name,
                            node,
                            DefinitionKind::Function,
                            scope,
                            Namespace::Value,
                        )),
                        // `export default function () {}` has no name of its own.
                        None if parent.is_some_and(|p| p.kind() == "export_statement") => {
                            Some(self.definition_named(
                                "default",
                                node,
                                DefinitionKind::Function,
                                scope,
                                Namespace::Value,
                                false,
                            ))
                        }
                        None => None,
                    }
                } else if matches!(node.kind(), "function_expression" | "arrow_function")
                    && parent.is_some_and(|p| {
                        p.kind() == "export_statement"
                            && p.child_by_field_name("value") == Some(node)
                    })
                {
                    // `export default function () {}` / `export default () => {}`.
                    Some(self.definition_named(
                        "default",
                        node,
                        DefinitionKind::Function,
                        scope,
                        Namespace::Value,
                        false,
                    ))
                } else if node.kind() == "method_definition"
                    && parent.is_some_and(|p| matches!(p.kind(), "class_body" | "object"))
                {
                    // Members are properties of their container, not names in scope.
                    let kind = if parent.is_some_and(|p| p.kind() == "class_body") {
                        DefinitionKind::Method
                    } else {
                        DefinitionKind::Function
                    };
                    self.member_name(node).map(|name| {
                        self.definition_named(name, node, kind, scope, Namespace::Value, false)
                    })
                } else if is_value_of("variable_declarator") {
                    self.facts
                        .definitions
                        .last()
                        .filter(|d| {
                            d.kind == DefinitionKind::Function
                                && d.start_byte <= node.start_byte()
                                && d.end_byte >= node.end_byte()
                        })
                        .map(|_| self.facts.definitions.len() - 1)
                } else if is_value_of("pair") {
                    // `{ list: () => {} }` inside an object literal.
                    let pair = parent.unwrap();
                    self.member_name(pair).map(|name| {
                        self.definition_named(
                            name,
                            pair,
                            DefinitionKind::Function,
                            scope,
                            Namespace::Value,
                            false,
                        )
                    })
                } else if is_value_of("public_field_definition") || is_value_of("field_definition")
                {
                    // `handle = () => {}` as a class property.
                    let field = parent.unwrap();
                    self.member_name(field).map(|name| {
                        self.definition_named(
                            name,
                            field,
                            DefinitionKind::Method,
                            scope,
                            Namespace::Value,
                            false,
                        )
                    })
                } else {
                    None
                };
                let previous_caller = self.current_caller;
                self.current_caller = owner;
                // Names defined inside a body belong to the function, not to the
                // class or object around it.
                let previous_container = self.current_container.take();
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
                self.current_container = previous_container;
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
                let mut owner = None;
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
                    owner =
                        Some(self.definition(name, node, kind, binding_scope, Namespace::Value));
                } else {
                    self.pattern(name, binding_scope);
                }
                if let Some(value) = node.child_by_field_name("value") {
                    if value.kind() == "object" && owner.is_some() {
                        // `const routes = { list() {} }`: members qualify as `routes.list`.
                        self.visit_members(value, scope, owner);
                    } else {
                        self.visit(value, scope);
                    }
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
                let owner = node
                    .child_by_field_name("name")
                    .map(|name| self.definition(name, node, kind, scope, namespace));
                let inner = self.scope(scope, false);
                if let Some(parameters) = node.child_by_field_name("type_parameters") {
                    self.pattern(parameters, inner);
                }
                let previous_container = self.current_container;
                self.current_container = owner.filter(|_| kind == DefinitionKind::Class);
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if Some(child) != node.child_by_field_name("name")
                        && Some(child) != node.child_by_field_name("type_parameters")
                    {
                        self.visit(child, inner);
                    }
                }
                self.current_container = previous_container;
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
        if self.facts.partial {
            // Scopes in a tree with errors are not trustworthy; keep only the
            // definitions, whose own spans say whether they parsed.
            self.facts.imports.clear();
            self.facts.valid = true;
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
/// A definition found by name: the file and its position in that file's facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefinitionRef {
    pub file: usize,
    pub definition: usize,
}

/// What the index holds, for the coverage line of an answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexCoverage {
    /// Parsed files by extension, most first.
    pub extensions: std::collections::BTreeMap<String, usize>,
    /// Files of a supported language that parsed (fully or with errors).
    pub parsed_files: usize,
    /// Parsed files whose tree had errors: definitions only, no relationships.
    pub partial_files: usize,
    pub definitions: usize,
}

pub struct NavigationIndex {
    files: Vec<(String, Arc<FileFacts>)>,
    paths: HashMap<String, usize>,
    incoming: HashMap<String, Vec<(usize, usize)>>,
    /// Definitions by leaf name, in path order.
    by_name: HashMap<String, Vec<DefinitionRef>>,
    /// Leaf names by their ASCII-lowercased form, for a case-insensitive fallback.
    by_lower: HashMap<String, Vec<String>>,
    coverage: IndexCoverage,
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
            by_name: HashMap::new(),
            by_lower: HashMap::new(),
            coverage: IndexCoverage::default(),
        };
        // Postings only: cross-file resolution happens for the selected evidence,
        // not for every reference whenever a CLI process starts.
        for (file, (path, facts)) in index.files.iter().enumerate() {
            if !facts.valid {
                continue;
            }
            if supports(path) {
                index.coverage.parsed_files += 1;
                index.coverage.partial_files += usize::from(facts.partial);
                if let Some(extension) = path.rsplit('.').next() {
                    *index
                        .coverage
                        .extensions
                        .entry(extension.to_owned())
                        .or_default() += 1;
                }
            }
            for (definition, item) in facts.definitions.iter().enumerate() {
                index.coverage.definitions += 1;
                let entry = DefinitionRef { file, definition };
                match index.by_name.get_mut(&item.name) {
                    Some(refs) => refs.push(entry),
                    None => {
                        let lower = item.name.to_ascii_lowercase();
                        if lower != item.name {
                            index
                                .by_lower
                                .entry(lower)
                                .or_default()
                                .push(item.name.clone());
                        }
                        index.by_name.insert(item.name.clone(), vec![entry]);
                    }
                }
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
    /// Fully parsed: relationships in this file are precise. A file with syntax
    /// errors has definitions but keeps the lexical fallback for relationships.
    pub fn is_parsed(&self, path: &str) -> bool {
        supports(path)
            && self.paths.get(path).is_some_and(|file| {
                let facts = &self.files[*file].1;
                facts.valid && !facts.partial && !facts.definitions_only
            })
    }
    pub fn coverage(&self) -> &IndexCoverage {
        &self.coverage
    }
    pub fn path(&self, reference: DefinitionRef) -> &str {
        &self.files[reference.file].0
    }
    pub fn get(&self, reference: DefinitionRef) -> &Definition {
        &self.files[reference.file].1.definitions[reference.definition]
    }
    /// Every definition with this exact leaf name, in path order.
    pub fn lookup(&self, name: &str) -> &[DefinitionRef] {
        self.by_name.get(name).map_or(&[], Vec::as_slice)
    }
    /// Definitions whose qualified name is `qualified` or ends with `.qualified`:
    /// `Server.handle` finds `NextNodeServer.handle`, never `handle` alone.
    pub fn lookup_qualified(&self, qualified: &str) -> Vec<DefinitionRef> {
        let (_, leaf) = qualified.rsplit_once('.').unwrap_or(("", qualified));
        if leaf == qualified {
            return self.lookup(leaf).to_vec();
        }
        self.lookup(leaf)
            .iter()
            .copied()
            .filter(|reference| {
                let full = &self.get(*reference).qualified;
                full == qualified
                    || full
                        .strip_suffix(qualified)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
            .collect()
    }
    /// Definitions whose leaf name matches ignoring ASCII case, when no exact
    /// match exists. Names that differ only by case are all returned.
    pub fn lookup_insensitive(&self, name: &str) -> Vec<DefinitionRef> {
        let lower = name.to_ascii_lowercase();
        let mut names = vec![];
        if self.by_name.contains_key(&lower) {
            names.push(lower.clone());
        }
        if let Some(others) = self.by_lower.get(&lower) {
            names.extend(others.iter().cloned());
        }
        names
            .iter()
            .flat_map(|name| self.lookup(name).iter().copied())
            .collect()
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

    #[test]
    fn class_members_object_members_and_default_exports_are_definitions() {
        let source = "export default class NextNodeServer extends BaseServer<Options> {\n  private handle(req: Request) {\n    const inner = () => 1;\n    return inner();\n  }\n  static create() { return new NextNodeServer(); }\n  onError = (error: Error) => { log(error); };\n}\nexport const routes = {\n  list() { return []; },\n  get: async (id: string) => id,\n  'quoted-key': () => 2,\n};\nexport default function () { return routes; }\n";
        let facts = NavigationPreparer::default().prepare("server.ts", source);
        assert!(facts.valid && !facts.partial);
        let summary = facts
            .definitions
            .iter()
            .map(|d| {
                (
                    d.qualified.as_str(),
                    d.kind,
                    d.start_line,
                    d.end_line,
                    d.container,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            vec![
                ("NextNodeServer", DefinitionKind::Class, 1, 8, None),
                (
                    "NextNodeServer.handle",
                    DefinitionKind::Method,
                    2,
                    5,
                    Some(0)
                ),
                // A const arrow inside a body is a definition of its own (as
                // before), but it belongs to no container.
                ("inner", DefinitionKind::Function, 3, 3, None),
                (
                    "NextNodeServer.create",
                    DefinitionKind::Method,
                    6,
                    6,
                    Some(0)
                ),
                (
                    "NextNodeServer.onError",
                    DefinitionKind::Method,
                    7,
                    7,
                    Some(0)
                ),
                ("routes", DefinitionKind::Constant, 9, 13, None),
                ("routes.list", DefinitionKind::Function, 10, 10, Some(5)),
                ("routes.get", DefinitionKind::Function, 11, 11, Some(5)),
                (
                    "routes.quoted-key",
                    DefinitionKind::Function,
                    12,
                    12,
                    Some(5)
                ),
                ("default", DefinitionKind::Function, 14, 14, None),
            ]
        );
        assert!(facts.definitions[0].exported());
        assert!(!facts.definitions[1].exported());
    }

    #[test]
    fn files_with_syntax_errors_keep_definitions_and_drop_relationships() {
        let source = "import { helper } from './helper';\nexport class BaseServer {\n  handle() { return helper(); }\n}\nconst broken = ;\nexport function after() { return 2; }\n";
        let facts = NavigationPreparer::default().prepare("base.ts", source);
        assert!(facts.valid && facts.partial);
        assert!(facts.references.is_empty() && facts.imports.is_empty());
        let names = facts
            .definitions
            .iter()
            .map(|d| (d.qualified.as_str(), d.complete))
            .collect::<Vec<_>>();
        assert!(names.contains(&("BaseServer", true)));
        assert!(names.contains(&("BaseServer.handle", true)));
        assert!(names.contains(&("after", true)));
        let index = index(&[("base.ts", source)]);
        assert!(!index.is_parsed("base.ts"));
        assert_eq!(index.lookup("BaseServer").len(), 1);
        assert_eq!(index.coverage().partial_files, 1);
    }

    #[test]
    fn lookup_by_leaf_qualified_and_case() {
        let index = index(&[
            (
                "a.ts",
                "export class Server { handle() {} }\nexport function handle() {}\n",
            ),
            (
                "b.ts",
                "export class NextNodeServer { handle() {} }\nexport const HANDLE = 1;\n",
            ),
        ]);
        assert_eq!(index.coverage().definitions, 6);
        assert_eq!(index.coverage().parsed_files, 2);
        assert_eq!(index.lookup("handle").len(), 3);
        let qualified = index.lookup_qualified("Server.handle");
        assert_eq!(qualified.len(), 1);
        assert_eq!(index.path(qualified[0]), "a.ts");
        assert_eq!(index.get(qualified[0]).qualified, "Server.handle");
        assert_eq!(index.lookup_qualified("NextNodeServer.handle").len(), 1);
        assert_eq!(index.lookup_qualified("Nope.handle").len(), 0);
        assert!(index.lookup("Handle").is_empty());
        let insensitive = index.lookup_insensitive("Handle");
        assert_eq!(insensitive.len(), 4);
        assert!(insensitive.iter().any(|r| index.get(*r).name == "HANDLE"));
    }

    fn summary(
        path: &str,
        source: &str,
    ) -> Vec<(String, DefinitionKind, usize, usize, Option<usize>, bool)> {
        let facts = NavigationPreparer::default().prepare(path, source);
        assert!(facts.valid && !facts.partial, "{path} should parse");
        assert!(facts.references.is_empty() && facts.imports.is_empty());
        facts
            .definitions
            .iter()
            .map(|d| {
                (
                    d.qualified.clone(),
                    d.kind,
                    d.start_line,
                    d.end_line,
                    d.container,
                    d.exported(),
                )
            })
            .collect()
    }

    #[test]
    fn python_definitions_with_decorators_methods_and_nested_functions() {
        let source = "import os\n\nX = 1\n\n@decorator\ndef top(a, b=2):\n    def inner(): pass\n    return a\n\nclass Flask(App):\n    \"\"\"Doc.\"\"\"\n    def __init__(self, name):\n        self.name = name\n\n    @property\n    def wsgi_app(self):\n        return 1\n\n    @staticmethod\n    def _make(): pass\n\nasync def fetch(): pass\n";
        let got = summary("app.py", source);
        let want = [
            ("top", DefinitionKind::Function, 5, 8, None, true),
            ("inner", DefinitionKind::Function, 7, 7, None, true),
            ("Flask", DefinitionKind::Class, 10, 20, None, true),
            (
                "Flask.__init__",
                DefinitionKind::Method,
                12,
                13,
                Some(2),
                false,
            ),
            (
                "Flask.wsgi_app",
                DefinitionKind::Method,
                15,
                17,
                Some(2),
                true,
            ),
            (
                "Flask._make",
                DefinitionKind::Method,
                19,
                20,
                Some(2),
                false,
            ),
            ("fetch", DefinitionKind::Function, 22, 22, None, true),
        ];
        assert_eq!(
            got,
            want.map(|(q, k, s, e, c, x)| (q.to_owned(), k, s, e, c, x))
        );
        let index = NavigationIndex::new_shared([(
            "app.py",
            Arc::new(NavigationPreparer::default().prepare("app.py", source)),
        )]);
        assert_eq!(index.lookup_qualified("Flask.wsgi_app").len(), 1);
        assert!(
            !index.is_parsed("app.py"),
            "definitions only: lexical relationships stay"
        );
        assert_eq!(index.coverage().parsed_files, 1);
    }

    #[test]
    fn go_definitions_qualify_methods_by_receiver() {
        let source = "package gin\n\ntype Context struct{ index int }\n\ntype Handler interface{ Serve() }\n\nconst Version = \"1\"\n\nfunc New() *Engine { return nil }\n\nfunc (c *Context) Next() { c.index++ }\n\nfunc (e Engine) run() {}\n";
        let got = summary("gin.go", source);
        let want = [
            ("Context", DefinitionKind::Class, 3, 3, None, true),
            ("Handler", DefinitionKind::Type, 5, 5, None, true),
            ("Version", DefinitionKind::Constant, 7, 7, None, true),
            ("New", DefinitionKind::Function, 9, 9, None, true),
            ("Context.Next", DefinitionKind::Method, 11, 11, None, true),
            ("Engine.run", DefinitionKind::Method, 13, 13, None, false),
        ];
        assert_eq!(
            got,
            want.map(|(q, k, s, e, c, x)| (q.to_owned(), k, s, e, c, x))
        );
    }

    #[test]
    fn rust_definitions_group_impl_members_under_their_type() {
        let source = "pub struct Searcher<'a> { x: &'a str }\npub enum Kind { A, B }\npub trait Matcher { fn find(&self) -> bool; }\nimpl<'a> Searcher<'a> {\n    pub fn new(x: &'a str) -> Self { Self { x } }\n    fn private(&self) {}\n}\nimpl Matcher for Searcher<'_> { fn find(&self) -> bool { true } }\npub fn top_level() {}\npub mod inner { pub fn nested() {} }\nconst MAX: usize = 1;\npub type Alias = u8;\nmacro_rules! m { () => {} }\n";
        let got = summary("lib.rs", source);
        let want = [
            ("Searcher", DefinitionKind::Type, 1, 1, None, true),
            ("Kind", DefinitionKind::Type, 2, 2, None, true),
            ("Matcher", DefinitionKind::Type, 3, 3, None, true),
            ("Matcher.find", DefinitionKind::Method, 3, 3, Some(2), false),
            ("Searcher", DefinitionKind::Class, 4, 7, None, true),
            ("Searcher.new", DefinitionKind::Method, 5, 5, Some(4), true),
            (
                "Searcher.private",
                DefinitionKind::Method,
                6,
                6,
                Some(4),
                false,
            ),
            ("Searcher", DefinitionKind::Class, 8, 8, None, true),
            (
                "Searcher.find",
                DefinitionKind::Method,
                8,
                8,
                Some(7),
                false,
            ),
            ("top_level", DefinitionKind::Function, 9, 9, None, true),
            ("inner", DefinitionKind::Module, 10, 10, None, true),
            ("nested", DefinitionKind::Function, 10, 10, None, true),
            ("MAX", DefinitionKind::Constant, 11, 11, None, false),
            ("Alias", DefinitionKind::Type, 12, 12, None, true),
            ("m", DefinitionKind::Function, 13, 13, None, true),
        ];
        assert_eq!(
            got,
            want.map(|(q, k, s, e, c, x)| (q.to_owned(), k, s, e, c, x))
        );
    }

    #[test]
    fn ruby_definitions_nest_classes_in_modules_and_split_scoped_names() {
        let source = "module Discourse\n  class Upload < ActiveRecord::Base\n    def url; end\n    def self.create_for(user); end\n    private\n    def secret; end\n  end\nend\nclass Foo::Bar\n  def call; end\nend\ndef top; end\n";
        let got = summary("upload.rb", source);
        let want = [
            ("Discourse", DefinitionKind::Module, 1, 8, None, true),
            (
                "Discourse.Upload",
                DefinitionKind::Class,
                2,
                7,
                Some(0),
                true,
            ),
            (
                "Discourse.Upload.url",
                DefinitionKind::Method,
                3,
                3,
                Some(1),
                true,
            ),
            (
                "Discourse.Upload.create_for",
                DefinitionKind::Method,
                4,
                4,
                Some(1),
                true,
            ),
            (
                "Discourse.Upload.secret",
                DefinitionKind::Method,
                6,
                6,
                Some(1),
                true,
            ),
            ("Foo.Bar", DefinitionKind::Class, 9, 11, None, true),
            (
                "Foo.Bar.call",
                DefinitionKind::Method,
                10,
                10,
                Some(5),
                true,
            ),
            ("top", DefinitionKind::Function, 12, 12, None, true),
        ];
        assert_eq!(
            got,
            want.map(|(q, k, s, e, c, x)| (q.to_owned(), k, s, e, c, x))
        );
        let index = NavigationIndex::new_shared([(
            "upload.rb",
            Arc::new(NavigationPreparer::default().prepare("upload.rb", source)),
        )]);
        assert_eq!(index.lookup("Upload").len(), 1);
        assert_eq!(index.lookup_qualified("Upload.url").len(), 1);
        assert_eq!(
            index
                .lookup_qualified("Foo::Bar".replace("::", ".").as_str())
                .len(),
            1
        );
    }

    #[test]
    fn java_definitions_nest_members_constants_and_enum_values() {
        let source = "package io.javalin;\n\nimport java.util.List;\n\n@Component\npublic class Javalin implements AutoCloseable {\n    public static final int DEFAULT_PORT = 8080;\n    private int port;\n\n    public Javalin(int port) { this.port = port; }\n\n    @Override\n    public Javalin start() { return this; }\n\n    private void stop() {}\n\n    public static class Config { public boolean debug; }\n\n    public enum Mode { DEV, PROD; public String label() { return name(); } }\n}\n\ninterface Handler {\n    int LIMIT = 1;\n    void handle(Context ctx);\n}\n\nrecord Point(int x, int y) {}\n";
        let got = summary("Javalin.java", source);
        let want = [
            ("Javalin", DefinitionKind::Class, 5, 20, None, true),
            (
                "Javalin.DEFAULT_PORT",
                DefinitionKind::Constant,
                7,
                7,
                Some(0),
                true,
            ),
            (
                "Javalin.Javalin",
                DefinitionKind::Method,
                10,
                10,
                Some(0),
                true,
            ),
            (
                "Javalin.start",
                DefinitionKind::Method,
                12,
                13,
                Some(0),
                true,
            ),
            (
                "Javalin.stop",
                DefinitionKind::Method,
                15,
                15,
                Some(0),
                false,
            ),
            (
                "Javalin.Config",
                DefinitionKind::Class,
                17,
                17,
                Some(0),
                true,
            ),
            ("Javalin.Mode", DefinitionKind::Class, 19, 19, Some(0), true),
            (
                "Javalin.Mode.DEV",
                DefinitionKind::Constant,
                19,
                19,
                Some(6),
                true,
            ),
            (
                "Javalin.Mode.PROD",
                DefinitionKind::Constant,
                19,
                19,
                Some(6),
                true,
            ),
            (
                "Javalin.Mode.label",
                DefinitionKind::Method,
                19,
                19,
                Some(6),
                true,
            ),
            ("Handler", DefinitionKind::Type, 22, 25, None, false),
            (
                "Handler.LIMIT",
                DefinitionKind::Constant,
                23,
                23,
                Some(10),
                true,
            ),
            (
                "Handler.handle",
                DefinitionKind::Method,
                24,
                24,
                Some(10),
                true,
            ),
            ("Point", DefinitionKind::Class, 27, 27, None, false),
        ];
        assert_eq!(
            got,
            want.map(|(q, k, s, e, c, x)| (q.to_owned(), k, s, e, c, x))
        );
        let index = NavigationIndex::new_shared([(
            "Javalin.java",
            Arc::new(NavigationPreparer::default().prepare("Javalin.java", source)),
        )]);
        assert_eq!(index.lookup("start").len(), 1);
        assert_eq!(index.lookup_qualified("Javalin.start").len(), 1);
        assert_eq!(index.lookup_qualified("Mode.DEV").len(), 1);
    }

    #[test]
    fn kotlin_definitions_cover_classes_objects_properties_and_enums() {
        let source = "package io.javalin\n\nimport java.util.List\n\nconst val DEFAULT_PORT = 8080\n\nclass JavalinConfig(val port: Int) {\n    @JvmField val routes = Routes()\n    var started = false\n\n    fun start(): JavalinConfig {\n        val local = 1\n        return this\n    }\n\n    private fun stop() {}\n\n    constructor() : this(0)\n\n    companion object {\n        @JvmStatic fun create() = JavalinConfig()\n    }\n\n    inner class Http {\n        fun bind() {}\n    }\n}\n\ninterface Handler {\n    fun handle(ctx: Context)\n}\n\nobject Defaults {\n    val limit = 1\n}\n\nenum class Mode {\n    DEV,\n    PROD\n}\n\ntypealias Ctx = Context\n\ninternal fun helper() = 1\n";
        let got = summary("JavalinConfig.kt", source);
        let want = [
            ("DEFAULT_PORT", DefinitionKind::Constant, 5, 5, None, true),
            ("JavalinConfig", DefinitionKind::Class, 7, 27, None, true),
            (
                "JavalinConfig.routes",
                DefinitionKind::Constant,
                8,
                8,
                Some(1),
                true,
            ),
            (
                "JavalinConfig.started",
                DefinitionKind::Constant,
                9,
                9,
                Some(1),
                true,
            ),
            (
                "JavalinConfig.start",
                DefinitionKind::Method,
                11,
                14,
                Some(1),
                true,
            ),
            (
                "JavalinConfig.stop",
                DefinitionKind::Method,
                16,
                16,
                Some(1),
                false,
            ),
            (
                "JavalinConfig.JavalinConfig",
                DefinitionKind::Method,
                18,
                18,
                Some(1),
                true,
            ),
            (
                "JavalinConfig.create",
                DefinitionKind::Method,
                21,
                21,
                Some(1),
                true,
            ),
            (
                "JavalinConfig.Http",
                DefinitionKind::Class,
                24,
                26,
                Some(1),
                true,
            ),
            (
                "JavalinConfig.Http.bind",
                DefinitionKind::Method,
                25,
                25,
                Some(8),
                true,
            ),
            ("Handler", DefinitionKind::Type, 29, 31, None, true),
            (
                "Handler.handle",
                DefinitionKind::Method,
                30,
                30,
                Some(10),
                true,
            ),
            ("Defaults", DefinitionKind::Class, 33, 35, None, true),
            (
                "Defaults.limit",
                DefinitionKind::Constant,
                34,
                34,
                Some(12),
                true,
            ),
            ("Mode", DefinitionKind::Class, 37, 40, None, true),
            ("Mode.DEV", DefinitionKind::Constant, 38, 38, Some(14), true),
            (
                "Mode.PROD",
                DefinitionKind::Constant,
                39,
                39,
                Some(14),
                true,
            ),
            ("Ctx", DefinitionKind::Type, 42, 42, None, true),
            ("helper", DefinitionKind::Function, 44, 44, None, false),
        ];
        assert_eq!(
            got,
            want.map(|(q, k, s, e, c, x)| (q.to_owned(), k, s, e, c, x))
        );
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
        let huge = format!(
            "export function run() {{}}\n{}",
            " ".repeat(PARSED_FILE_BYTES)
        );
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
