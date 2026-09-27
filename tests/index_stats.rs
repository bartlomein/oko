//! Index statistics for a real repository, for measuring index changes.
//!
//!   OKO_STATS_ROOT=/path/to/repo cargo test --release --test index_stats -- --ignored --nocapture
//!
//! Prints one JSON line: files, parsed files, definitions by kind, cold
//! preparation time and the snapshot size on disk. Ignored by default because
//! it needs a repository to point at.
use oko::search_cache::WorkspaceCache;
use std::collections::BTreeMap;
use std::path::Path;

#[test]
#[ignore]
fn index_stats() {
    let root = std::env::var("OKO_STATS_ROOT").expect("OKO_STATS_ROOT");
    let disk = tempfile::tempdir().unwrap();
    let mut cache = WorkspaceCache::with_directory(disk.path().to_owned()).without_watching();
    let loaded = cache.load(Path::new(&root)).unwrap();
    let navigation = loaded.snapshot.navigation();
    let coverage = navigation.coverage();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut partial_definitions = 0;
    let mut with_container = 0;
    // Every parsed file, including big files kept without chunks.
    for (_, definition) in navigation.all_definitions() {
        *kinds
            .entry(format!("{:?}", definition.kind).to_lowercase())
            .or_default() += 1;
        partial_definitions += usize::from(!definition.complete);
        with_container += usize::from(definition.container.is_some());
    }
    let snapshot_bytes = std::fs::read_dir(disk.path())
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.metadata().ok().map(|m| m.len()))
        .sum::<u64>();
    println!(
        "STATS {}",
        serde_json::json!({
            "root": root,
            "files": loaded.timings.read_files,
            "chunks": loaded.snapshot.chunks().len(),
            "parsedFiles": coverage.parsed_files,
            "partialFiles": coverage.partial_files,
            "definitions": coverage.definitions,
            "definitionsByKind": kinds,
            "incompleteDefinitions": partial_definitions,
            "withContainer": with_container,
            "coldPrepareMs": loaded.timings.total_ms,
            "prepareMs": loaded.timings.prepare_ms,
            "navigationMs": loaded.timings.navigation_ms,
            "snapshotBytes": snapshot_bytes,
        })
    );
}
