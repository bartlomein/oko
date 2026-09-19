use oko::{
    context::{ContextPacket, build_packet_with_navigation},
    navigation::{NavigationIndex, NavigationPreparer},
    search::{Chunk, chunk_text},
    search_cache::WorkspaceCache,
};

fn packet(files: &[(&str, &str)], winner: &str, question: &str) -> ContextPacket {
    let mut preparer = NavigationPreparer::default();
    let facts: Vec<_> = files
        .iter()
        .map(|(path, text)| (*path, preparer.prepare(path, text)))
        .collect();
    let index = NavigationIndex::new(facts.iter().map(|(path, facts)| (*path, facts)));
    let corpus: Vec<_> = files
        .iter()
        .flat_map(|(path, text)| chunk_text(path, text))
        .collect();
    let winners = vec![(
        corpus
            .iter()
            .find(|chunk| chunk.path == winner)
            .unwrap()
            .clone(),
        0.9,
    )];
    build_packet_with_navigation(&corpus, &winners, question, &index)
}

#[test]
fn attaches_exact_imported_validation_constants_instead_of_same_named_decoys() {
    let result = packet(
        &[
            (
                "src/decode.ts",
                "import { PayloadRule as Payload, TextRule } from './rules.js';\nexport function decode(value: string): unknown {\n const text = TextRule.parse(value);\n return check({ schema: Payload, value: JSON.parse(text) });\n}\n",
            ),
            (
                "src/rules.ts",
                "export const TextRule = stringRule().min(1);\nexport const PayloadRule = objectRule({ tick: finiteNumber(), id: TextRule });\n",
            ),
            (
                "other/rules.ts",
                "export const PayloadRule = unsafeRule();\n",
            ),
        ],
        "src/decode.ts",
        "where is decoded payload validated by text and payload rules?",
    );
    assert_eq!(result.related.len(), 2);
    assert!(
        result
            .related
            .iter()
            .all(|r| r.excerpt.path == "src/rules.ts" && r.relation == "resolved_definition")
    );
    assert!(
        result
            .related
            .iter()
            .any(|r| r.excerpt.text.contains("finiteNumber"))
    );
    assert!(result.related.iter().all(|r| r.target.is_none()));
    assert!(serde_json::to_vec(&result).unwrap().len() <= 16_000);
}

#[test]
fn returns_verified_caller_and_omits_shadowed_import() {
    let result = packet(
        &[
            (
                "src/decision.ts",
                "export function destination(ready: boolean): string | null {\n if (!ready) return null;\n return '/ready';\n}\n",
            ),
            (
                "src/route.ts",
                "import { destination as choose } from './decision.js';\nexport function handle(ready: boolean) {\n const next = choose(ready);\n if (next) return redirect(next);\n}\n",
            ),
            (
                "src/shadow.ts",
                "import { destination as choose } from './decision.js';\nexport function handle(choose: Function) {\n return choose(true);\n}\n",
            ),
        ],
        "src/decision.ts",
        "where does destination decide the route redirect?",
    );
    assert!(result.results[0].excerpt.definition_complete);
    assert!(!result.results[0].excerpt.truncated);
    assert_eq!(result.related.len(), 1);
    let caller = &result.related[0];
    assert_eq!(caller.excerpt.path, "src/route.ts");
    assert_eq!(caller.relation, "resolved_caller");
    assert_eq!(caller.referenced_from[0].path, caller.excerpt.path);
    assert_eq!(caller.target.as_ref().unwrap().path, "src/decision.ts");
    assert!(caller.excerpt.definition_complete);
}

#[test]
fn multiline_export_keeps_the_complete_union_return_implementation() {
    let result = packet(
        &[(
            "decision.ts",
            "export\nfunction choose(ready: boolean): string | null {\n if (!ready) return null;\n return '/ready';\n}\n",
        )],
        "decision.ts",
        "choose ready route",
    );
    assert!(result.results[0].excerpt.definition_complete);
    assert!(!result.results[0].excerpt.truncated);
    assert_eq!(result.results[0].excerpt.start_line, 1);
    assert_eq!(result.results[0].excerpt.end_line, 5);
}

#[test]
fn rejected_syntax_preserves_legacy_related_definition_fallback() {
    let mut preparer = NavigationPreparer::default();
    let source = "function run(value) {\n return helper(value);\n}\nconst incomplete = ;\n";
    let support = "function helper(value) {\n return value;\n}\n";
    let files = [("entry.ts", source), ("helper.ts", support)];
    let facts: Vec<_> = files
        .iter()
        .map(|(p, t)| (*p, preparer.prepare(p, t)))
        .collect();
    let index = NavigationIndex::new(facts.iter().map(|(p, f)| (*p, f)));
    assert!(!index.is_parsed("entry.ts"));
    let corpus: Vec<_> = files.iter().flat_map(|(p, t)| chunk_text(p, t)).collect();
    let winners = [(corpus[0].clone(), 0.9)];
    let expected = oko::context::build_packet(&corpus, &winners, "run helper");
    let actual = build_packet_with_navigation(&corpus, &winners, "run helper", &index);
    assert!(!expected.related.is_empty());
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn complete_primary_definition_stays_distinct_from_packet_truncation() {
    let source = "export function choose(flag: boolean): string | null {\n return flag ? '/next' : null;\n}\n";
    let mut preparer = NavigationPreparer::default();
    let facts = preparer.prepare("choose.ts", source);
    let index = NavigationIndex::new([("choose.ts", &facts)]);
    let chunks = chunk_text("choose.ts", source);
    let winners: Vec<_> = (0..4).map(|_| (chunks[0].clone(), 0.9)).collect();
    let mut result = build_packet_with_navigation(&chunks, &winners, "choose next route", &index);
    assert!(result.results[0].excerpt.definition_complete);
    let budget = serde_json::to_vec(&result).unwrap().len() - 20;
    result.fit_to_budget(budget);
    assert!(result.truncated);
    assert!(result.results.is_empty() || !result.results[0].excerpt.definition_complete);
}

fn cached_packet(cache: &mut WorkspaceCache, root: &std::path::Path) -> ContextPacket {
    let loaded = cache.load(root).unwrap();
    let chunks = loaded.snapshot.chunks();
    let winner: Chunk = chunks
        .iter()
        .find(|c| c.path == "entry.ts")
        .unwrap()
        .clone();
    build_packet_with_navigation(
        chunks,
        &[(winner, 0.9)],
        "validate payload rule",
        loaded.snapshot.navigation(),
    )
}

#[test]
fn cached_relationships_refresh_when_only_the_dependency_changes_or_disappears() {
    let root = tempfile::tempdir().unwrap();
    let disk = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("entry.ts"), "import { Rule } from './rule.js';\nexport function validate(value: unknown) {\n return Rule.parse(value);\n}\n").unwrap();
    std::fs::write(
        root.path().join("rule.ts"),
        "export const Rule = strictValidator();\n",
    )
    .unwrap();
    let mut cache = WorkspaceCache::with_directory(disk.path().to_owned());
    let first = cached_packet(&mut cache, root.path());
    assert!(
        first
            .related
            .iter()
            .any(|r| r.excerpt.text.contains("strictValidator"))
    );
    let mut restarted = WorkspaceCache::with_directory(disk.path().to_owned());
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(cached_packet(&mut restarted, root.path())).unwrap()
    );
    std::fs::write(
        root.path().join("rule.ts"),
        "export const Rule = changedValidator();\n",
    )
    .unwrap();
    let changed = cached_packet(&mut cache, root.path());
    assert!(
        changed
            .related
            .iter()
            .any(|r| r.excerpt.text.contains("changedValidator"))
    );
    assert!(
        !changed
            .related
            .iter()
            .any(|r| r.excerpt.text.contains("strictValidator"))
    );
    std::fs::remove_file(root.path().join("rule.ts")).unwrap();
    assert!(cached_packet(&mut cache, root.path()).related.is_empty());
}

#[test]
fn bom_normalized_syntax_facts_are_reused_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let disk = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("entry.ts"),
        "\u{feff}export function choose(): string | null {\n return null;\n}\n",
    )
    .unwrap();
    let mut first = WorkspaceCache::with_directory(disk.path().to_owned());
    assert_eq!(first.load(root.path()).unwrap().timings.rebuilt_files, 1);
    let mut restarted = WorkspaceCache::with_directory(disk.path().to_owned());
    let loaded = restarted.load(root.path()).unwrap();
    assert_eq!(loaded.timings.status, "disk");
    assert_eq!(loaded.timings.rebuilt_files, 0);
    assert_eq!(loaded.timings.reused_files, 1);
    assert!(loaded.snapshot.navigation().is_parsed("entry.ts"));
}
