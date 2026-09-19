use oko::RankingIntent;
use oko::search::{
    Chunk, MAX_FILE_BYTES, rank_lexically, rank_lexically_with_intent, workspace_chunks,
};
use oko::search_cache::{CachedWorkspace, WorkspaceCache};
use std::{
    fs::{self, File, FileTimes},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Barrier},
};
use tempfile::TempDir;

const QUESTIONS: &[&str] = &[
    "where is the archive checksum verified",
    "persist snapshot transaction",
    "recover archive",
    "how is cancellation handled",
    "where is the registry resolved",
];

struct Fixture {
    root: TempDir,
    disk: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
            disk: tempfile::tempdir().unwrap(),
        }
    }

    fn write(&self, path: &str, contents: impl AsRef<[u8]>) {
        let path = self.root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn cache(&self) -> WorkspaceCache {
        WorkspaceCache::with_directory(self.disk.path().to_path_buf())
    }

    fn populate(&self) {
        self.write(
            "src/archive.rs",
            "pub fn verify_archive_checksum() {\n    recover_archive();\n}\n",
        );
        self.write(
            "src/store.ts",
            "export function persistSnapshot() {\n  verify_archive_checksum();\n}\n",
        );
        self.write(
            "docs/storage.md",
            "Persist a snapshot after the archive transaction. Cancellation rolls it back.\n",
        );
    }
}

fn assert_fresh(cache: &mut WorkspaceCache, root: &Path) -> CachedWorkspace {
    let cached = cache.load(root).unwrap();
    let fresh = workspace_chunks(root).unwrap();
    assert_eq!(cached.snapshot.chunks(), fresh.as_slice());
    for question in QUESTIONS {
        assert_eq!(
            cached.snapshot.rank(question),
            rank_lexically(&fresh, question),
            "cached shortlist changed paths, source ranges, text, scores, or order for {question:?}",
        );
        for intent in [
            RankingIntent::General,
            RankingIntent::Explanation,
            RankingIntent::Implementation,
        ] {
            assert_eq!(
                cached.snapshot.rank_with_intent(question, intent),
                rank_lexically_with_intent(&fresh, question, intent),
                "cached intent ranking diverged: {question:?}, {intent:?}",
            );
        }
    }
    cached
}

fn paths(chunks: &[Chunk]) -> Vec<&str> {
    chunks.iter().map(|chunk| chunk.path.as_str()).collect()
}

#[test]
fn unchanged_memory_and_restarted_caches_reuse_all_preparation() {
    let fixture = Fixture::new();
    fixture.populate();
    let before = workspace_chunks(fixture.root.path()).unwrap();
    let mut cache = fixture.cache();
    let cold = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(cold.timings.rebuilt_files, 3);
    assert_eq!(cold.timings.reused_files, 0);
    assert!(!cold.timings.aggregate_reused);

    let warm = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(warm.timings.rebuilt_files, 0);
    assert_eq!(warm.timings.reused_files, 3);
    assert!(warm.timings.aggregate_reused);
    assert!(Arc::ptr_eq(&cold.snapshot, &warm.snapshot));

    let restarted = assert_fresh(&mut fixture.cache(), fixture.root.path());
    assert_eq!(restarted.timings.rebuilt_files, 0);
    assert_eq!(restarted.timings.reused_files, 3);
    assert!(restarted.timings.aggregate_reused);
    assert_eq!(before, workspace_chunks(fixture.root.path()).unwrap());
    assert_eq!(fs::read_dir(fixture.root.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn aged_unchanged_sources_skip_content_reads_but_restart_validates_again() {
    let fixture = Fixture::new();
    fixture.populate();
    // Age before capture: a recently captured source must not become reusable
    // merely because its timestamp later passes the racy-write cutoff.
    std::thread::sleep(std::time::Duration::from_millis(2100));
    let mut cache = fixture.cache();
    let cold = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(cold.timings.read_files, 3);
    assert_eq!(cold.timings.reused_contents, 0);
    assert_eq!(cold.timings.validation, "full");
    let warm = assert_fresh(&mut cache, fixture.root.path());
    if warm.timings.fallback_reason.is_some()
        || warm.timings.validation_reason == "unsupported-filesystem"
    {
        // Unsupported filesystems/backends must remain correct through full reads.
        assert_eq!(warm.timings.validation, "full");
        assert_eq!(warm.timings.read_files, 3);
        assert_eq!(warm.timings.reused_contents, 0);
    } else {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(warm.timings.validation, "incremental");
        let precise_timestamps = ["src/archive.rs", "src/store.ts", "docs/storage.md"]
            .iter()
            .all(|path| {
                fs::metadata(fixture.root.path().join(path))
                    .unwrap()
                    .ctime_nsec()
                    > 0
            });
        if precise_timestamps {
            assert_eq!(warm.timings.read_files, 0);
            assert_eq!(warm.timings.reused_contents, 3);
        } else {
            // Zero subsecond change times cannot authorize metadata-only reuse.
            assert!(warm.timings.read_files > 0);
            assert_eq!(warm.timings.read_files + warm.timings.reused_contents, 3);
        }
    }
    assert!(Arc::ptr_eq(&cold.snapshot, &warm.snapshot));

    let restarted = assert_fresh(&mut fixture.cache(), fixture.root.path());
    assert_eq!(restarted.timings.read_files, 3);
    assert_eq!(restarted.timings.reused_contents, 0);
    assert_eq!(restarted.timings.validation, "full");
    assert_eq!(restarted.timings.rebuilt_files, 0);
}

#[test]
fn large_workspace_preserves_all_intents_across_parallel_refresh_and_restart() {
    let fixture = Fixture::new();
    for index in 0..96 {
        let extension = ["rs", "ts", "py", "md", "custom"][index % 5];
        fixture.write(
            &format!("src/file{index:03}.{extension}"),
            format!(
                "\u{feff}fn verify_archive_checksum_{index}() {{\r\n    resolve_registry();\r\n    persist_snapshot_transaction();\r\n    // recover archive 🦀\r\n}}\r\n"
            ),
        );
    }
    let mut cache = fixture.cache();
    let cold = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(cold.timings.rebuilt_files, 96);
    let disk = assert_fresh(&mut fixture.cache(), fixture.root.path());
    assert!(disk.timings.aggregate_reused);
    let warm = assert_fresh(&mut cache, fixture.root.path());
    assert!(Arc::ptr_eq(&cold.snapshot, &warm.snapshot));

    fixture.write(
        "src/file000.rs",
        "fn recover_archive() { cancel_snapshot_transaction(); }\n",
    );
    fs::remove_file(fixture.root.path().join("src/file001.ts")).unwrap();
    let refreshed = assert_fresh(&mut cache, fixture.root.path());
    assert!(!refreshed.timings.aggregate_reused);
    assert_eq!(refreshed.timings.rebuilt_files, 1);
    assert_eq!(refreshed.timings.reused_files, 94);
    let restarted = assert_fresh(&mut fixture.cache(), fixture.root.path());
    assert!(restarted.timings.aggregate_reused);
    assert_eq!(refreshed.snapshot.chunks(), restarted.snapshot.chunks());
}

#[test]
fn same_length_edit_with_restored_modified_time_is_never_a_hit() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache();
    let original = assert_fresh(&mut cache, fixture.root.path());
    let path = fixture.root.path().join("src/archive.rs");
    let metadata = fs::metadata(&path).unwrap();
    let old_text = fs::read_to_string(&path).unwrap();
    let new_text = old_text.replace("verify", "repair");
    assert_eq!(new_text.len(), old_text.len());
    fs::write(&path, new_text).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), metadata.len());
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        metadata.modified().unwrap(),
    );

    let edited = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(edited.timings.rebuilt_files, 1);
    assert_eq!(edited.timings.reused_files, 2);
    assert_ne!(original.snapshot.chunks(), edited.snapshot.chunks());
    // A previous request retains its captured source even after a refresh.
    assert!(original.snapshot.chunks()[0].text.contains("archive"));
    assert!(original.snapshot.chunks().iter().any(|chunk| {
        chunk.path == "src/archive.rs" && chunk.text.contains("fn verify_archive_checksum")
    }));
    assert_fresh(&mut fixture.cache(), fixture.root.path());
}

#[test]
fn additions_deletions_and_renames_refresh_corpus_statistics_and_paths() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache();
    assert_fresh(&mut cache, fixture.root.path());
    fixture.write(
        "src/cancel.rs",
        "pub fn cancel_archive_transaction() {\n    rollback_snapshot();\n}\n",
    );
    let added = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(added.timings.rebuilt_files, 1);
    assert_eq!(added.timings.reused_files, 3);

    fs::remove_file(fixture.root.path().join("src/archive.rs")).unwrap();
    let removed = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(removed.timings.rebuilt_files, 0);
    assert_eq!(removed.timings.reused_files, 3);
    assert!(!paths(removed.snapshot.chunks()).contains(&"src/archive.rs"));

    fs::rename(
        fixture.root.path().join("src/cancel.rs"),
        fixture.root.path().join("src/recover.rs"),
    )
    .unwrap();
    let renamed = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(renamed.timings.rebuilt_files, 1);
    assert_eq!(renamed.timings.reused_files, 2);
    assert!(!paths(renamed.snapshot.chunks()).contains(&"src/cancel.rs"));
    assert!(paths(renamed.snapshot.chunks()).contains(&"src/recover.rs"));
    assert_fresh(&mut fixture.cache(), fixture.root.path());
}

#[test]
fn new_declarations_use_references_in_unchanged_files() {
    let fixture = Fixture::new();
    fixture.write(
        "src/caller.rs",
        "pub fn start_job() {\n    resolve_registry();\n}\n",
    );
    fixture.write("docs/registry.md", "resolve registry configuration\n");
    let mut cache = fixture.cache();
    assert_fresh(&mut cache, fixture.root.path());
    fixture.write(
        "src/registry.rs",
        "pub fn resolve_registry() {\n    read_configuration();\n}\n",
    );
    let added = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(added.timings.rebuilt_files, 1);
    assert_eq!(added.timings.reused_files, 2);
    assert_fresh(&mut fixture.cache(), fixture.root.path());
    fs::remove_file(fixture.root.path().join("src/registry.rs")).unwrap();
    assert_fresh(&mut cache, fixture.root.path());
}

#[test]
fn changed_ignore_rules_remove_and_restore_previously_cached_files() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache();
    assert_fresh(&mut cache, fixture.root.path());
    // .ignore works in non-Git directories too, unlike default .gitignore handling.
    fixture.write(".ignore", "docs/\n");
    let ignored = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(ignored.timings.rebuilt_files, 0);
    assert_eq!(ignored.timings.reused_files, 2);
    assert!(!paths(ignored.snapshot.chunks()).contains(&"docs/storage.md"));
    assert_fresh(&mut fixture.cache(), fixture.root.path());

    fixture.write(".ignore", "src/\n");
    let changed = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(changed.timings.rebuilt_files, 1);
    assert_eq!(paths(changed.snapshot.chunks()), ["docs/storage.md"]);
    fs::remove_file(fixture.root.path().join(".ignore")).unwrap();
    assert_fresh(&mut cache, fixture.root.path());
}

#[test]
fn files_that_become_ineligible_do_not_survive_in_memory_or_on_disk() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache();
    assert_fresh(&mut cache, fixture.root.path());
    for contents in [
        b"archive\0checksum".to_vec(),
        vec![b'a'; MAX_FILE_BYTES + 1],
        vec![0xff, 0xfe, b'a'],
        b" \n\r\t".to_vec(),
    ] {
        fixture.write("src/archive.rs", &contents);
        let excluded = assert_fresh(&mut cache, fixture.root.path());
        assert!(!paths(excluded.snapshot.chunks()).contains(&"src/archive.rs"));
        assert_fresh(&mut fixture.cache(), fixture.root.path());

        fixture.write(
            "src/archive.rs",
            "\u{feff}pub fn verify_archive_checksum() {\r\n    recover_archive();\r\n}\r\n",
        );
        let restored = assert_fresh(&mut cache, fixture.root.path());
        assert_eq!(restored.timings.rebuilt_files, 1);
        assert_eq!(restored.timings.reused_files, 2);
    }
}

#[test]
fn directories_and_search_scopes_have_independent_paths_and_statistics() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache();
    let whole = assert_fresh(&mut cache, fixture.root.path());
    let src = fixture.root.path().join("src");
    let scoped = assert_fresh(&mut cache, &src);
    assert!(paths(whole.snapshot.chunks()).contains(&"src/archive.rs"));
    assert!(paths(scoped.snapshot.chunks()).contains(&"archive.rs"));
    assert!(!paths(scoped.snapshot.chunks()).contains(&"docs/storage.md"));
    assert_fresh(&mut fixture.cache(), &src);
    assert_fresh(&mut cache, fixture.root.path());

    let another_root = tempfile::tempdir().unwrap();
    fs::write(
        another_root.path().join("archive.rs"),
        "pub fn repair_archive() {\n    recover_archive();\n}\n",
    )
    .unwrap();
    assert_fresh(&mut cache, another_root.path());
    assert_fresh(&mut fixture.cache(), fixture.root.path());
    assert_fresh(&mut fixture.cache(), another_root.path());
}

#[test]
fn parent_ignore_changes_apply_to_a_warmed_subdirectory_scope() {
    let fixture = Fixture::new();
    fixture.write("src/visible.rs", "fn verify_archive_checksum() {}\n");
    fixture.write("src/hidden.rs", "fn recover_archive() {}\n");
    let scope = fixture.root.path().join("src");
    let mut cache = fixture.cache();
    let original = assert_fresh(&mut cache, &scope);
    assert!(paths(original.snapshot.chunks()).contains(&"hidden.rs"));

    // The ignore file is outside the selected scope and its watcher root.
    fixture.write(".ignore", "hidden.rs\n");
    let excluded = assert_fresh(&mut cache, &scope);
    assert_eq!(paths(excluded.snapshot.chunks()), ["visible.rs"]);
    assert!(paths(original.snapshot.chunks()).contains(&"hidden.rs"));

    fs::remove_file(fixture.root.path().join(".ignore")).unwrap();
    let restored = assert_fresh(&mut cache, &scope);
    assert_eq!(restored.snapshot.chunks(), original.snapshot.chunks());
}

#[test]
fn returning_to_an_edited_root_does_not_reuse_another_roots_freshness() {
    let fixture = Fixture::new();
    fixture.populate();
    let other = tempfile::tempdir().unwrap();
    fs::write(
        other.path().join("archive.rs"),
        "fn unrelated_archive() {}\n",
    )
    .unwrap();
    let mut cache = fixture.cache();
    let original = assert_fresh(&mut cache, fixture.root.path());
    let original_chunks = original.snapshot.chunks().to_vec();
    let other_snapshot = assert_fresh(&mut cache, other.path());

    // This write happens while the cache's selected root is elsewhere.
    fixture.write("src/archive.rs", "fn cancel_archive_transaction() {}\n");
    let returned = assert_fresh(&mut cache, fixture.root.path());
    assert_ne!(returned.snapshot.chunks(), original_chunks);
    assert_eq!(original.snapshot.chunks(), original_chunks);
    assert_eq!(paths(other_snapshot.snapshot.chunks()), ["archive.rs"]);
    assert_fresh(&mut cache, other.path());
}

#[test]
fn atomic_replacement_with_same_length_and_mtime_is_visible_immediately() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache();
    let original = assert_fresh(&mut cache, fixture.root.path());
    let path = fixture.root.path().join("src/archive.rs");
    let metadata = fs::metadata(&path).unwrap();
    let old_text = fs::read_to_string(&path).unwrap();
    let new_text = old_text.replace("verify", "repair");
    assert_eq!(new_text.len(), old_text.len());
    let replacement = fixture.root.path().join("src/.replacement");
    fs::write(&replacement, &new_text).unwrap();
    File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    fs::rename(&replacement, &path).unwrap();

    // No sleep to wait for native filesystem callbacks: the next load must see it.
    let replaced = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(replaced.timings.rebuilt_files, 1);
    assert!(replaced.snapshot.chunks().iter().any(|chunk| {
        chunk.path == "src/archive.rs" && chunk.text.contains("fn repair_archive_checksum")
    }));
    assert!(original.snapshot.chunks().iter().any(|chunk| {
        chunk.path == "src/archive.rs" && chunk.text.contains("fn verify_archive_checksum")
    }));
}

fn stored_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(stored_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn corrupt_or_missing_snapshots_rebuild_without_breaking_search() {
    let fixture = Fixture::new();
    fixture.populate();
    assert_fresh(&mut fixture.cache(), fixture.root.path());
    let files = stored_files(fixture.disk.path());
    assert!(
        !files.is_empty(),
        "cold search should publish a disk snapshot"
    );
    for file in files {
        fs::write(file, b"{malformed cache payload").unwrap();
    }
    let recovered = assert_fresh(&mut fixture.cache(), fixture.root.path());
    assert_eq!(recovered.timings.rebuilt_files, 3);
    for file in stored_files(fixture.disk.path()) {
        fs::remove_file(file).unwrap();
    }
    let missing = assert_fresh(&mut fixture.cache(), fixture.root.path());
    assert_eq!(missing.timings.rebuilt_files, 3);
}

#[test]
fn unavailable_cache_directory_falls_back_to_search_and_memory_reuse() {
    let fixture = Fixture::new();
    fixture.populate();
    let blocked = fixture.disk.path().join("regular-file");
    fs::write(&blocked, "not a directory").unwrap();
    let mut cache = WorkspaceCache::with_directory(blocked);
    let cold = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(cold.timings.rebuilt_files, 3);
    let warm = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(warm.timings.rebuilt_files, 0);
    assert_eq!(warm.timings.reused_files, 3);
}

#[test]
fn empty_workspaces_and_last_file_deletion_return_empty_results() {
    let fixture = Fixture::new();
    let mut cache = fixture.cache();
    let empty = assert_fresh(&mut cache, fixture.root.path());
    assert!(empty.snapshot.chunks().is_empty());
    assert_eq!(empty.timings.rebuilt_files, 0);
    fixture.write("archive.txt", "verify archive checksum\n");
    assert_fresh(&mut cache, fixture.root.path());
    fs::remove_file(fixture.root.path().join("archive.txt")).unwrap();
    let deleted = assert_fresh(&mut cache, fixture.root.path());
    assert!(deleted.snapshot.chunks().is_empty());
    assert_fresh(&mut fixture.cache(), fixture.root.path());
}

#[test]
fn explicit_bypass_rebuilds_each_request_with_identical_results() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = WorkspaceCache::disabled();
    for _ in 0..2 {
        let result = assert_fresh(&mut cache, fixture.root.path());
        assert_eq!(result.timings.rebuilt_files, 3);
        assert_eq!(result.timings.reused_files, 0);
        assert_eq!(result.timings.status, "disabled");
    }
    assert!(stored_files(fixture.disk.path()).is_empty());
}

#[test]
fn disabling_watching_preserves_preparation_but_reads_every_source_again() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache().without_watching();
    let cold = assert_fresh(&mut cache, fixture.root.path());
    assert_eq!(cold.timings.rebuilt_files, 3);
    for _ in 0..2 {
        let warm = assert_fresh(&mut cache, fixture.root.path());
        assert_eq!(warm.timings.status, "memory");
        assert_eq!(warm.timings.validation, "full");
        assert_eq!(warm.timings.validation_reason, "watching-disabled");
        assert_eq!(warm.timings.read_files, 3);
        assert_eq!(warm.timings.reused_contents, 0);
        assert_eq!(warm.timings.rebuilt_files, 0);
        assert_eq!(warm.timings.reused_files, 3);
        assert!(Arc::ptr_eq(&cold.snapshot, &warm.snapshot));
    }
    assert!(!stored_files(fixture.disk.path()).is_empty());
    let restarted = assert_fresh(&mut fixture.cache().without_watching(), fixture.root.path());
    assert_eq!(restarted.timings.status, "disk");
    assert_eq!(restarted.timings.read_files, 3);
    assert_eq!(restarted.timings.reused_contents, 0);
    assert_eq!(restarted.timings.rebuilt_files, 0);
}

fn git(root: &Path, arguments: &[&str]) {
    let result = Command::new("git")
        .args([
            "-c",
            "user.name=Cache Test",
            "-c",
            "user.email=cache@example.invalid",
        ])
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("git is required for branch/worktree cache tests");
    assert!(
        result.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&result.stderr),
    );
}

#[test]
fn branch_switches_and_separate_worktrees_never_reuse_stale_source() {
    let fixture = Fixture::new();
    fixture.populate();
    let root = fixture.root.path();
    git(root, &["init", "--quiet"]);
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "Initial fixture"]);
    git(root, &["branch", "cache-original"]);
    git(root, &["checkout", "--quiet", "-b", "cache-alternate"]);
    let mut cache = fixture.cache();
    let original = assert_fresh(&mut cache, root);
    fixture.write(
        "src/archive.rs",
        "pub fn repair_archive_checksum() {\n    recover_archive();\n}\n",
    );
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "Alternate fixture"]);
    let alternate = assert_fresh(&mut cache, root);
    assert_ne!(original.snapshot.chunks(), alternate.snapshot.chunks());
    assert_eq!(alternate.timings.rebuilt_files, 1);

    git(root, &["checkout", "--quiet", "cache-original"]);
    let restored = assert_fresh(&mut cache, root);
    assert_eq!(restored.snapshot.chunks(), original.snapshot.chunks());
    assert_fresh(&mut fixture.cache(), root);

    let worktree_parent = tempfile::tempdir().unwrap();
    let worktree = worktree_parent.path().join("alternate");
    git(
        root,
        &[
            "worktree",
            "add",
            "--quiet",
            worktree.to_str().unwrap(),
            "cache-alternate",
        ],
    );
    let worktree_result = assert_fresh(&mut cache, &worktree);
    assert_eq!(
        worktree_result.snapshot.chunks(),
        alternate.snapshot.chunks()
    );
    assert_fresh(&mut fixture.cache(), &worktree);
    let root_result = assert_fresh(&mut cache, root);
    assert_eq!(root_result.snapshot.chunks(), original.snapshot.chunks());
}

#[test]
fn competing_publishers_and_stale_instances_keep_results_current() {
    let fixture = Fixture::new();
    fixture.populate();
    let barrier = Arc::new(Barrier::new(4));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let barrier = Arc::clone(&barrier);
            let fixture = &fixture;
            scope.spawn(move || {
                let mut cache = fixture.cache();
                barrier.wait();
                assert_fresh(&mut cache, fixture.root.path());
            });
        }
    });
    let mut stale = fixture.cache();
    assert_fresh(&mut stale, fixture.root.path());
    fixture.write(
        "src/store.ts",
        "export function cancelSnapshot() { rollback(); }\n",
    );
    assert_fresh(&mut fixture.cache(), fixture.root.path());
    let updated = assert_fresh(&mut stale, fixture.root.path());
    assert_eq!(updated.timings.rebuilt_files, 1);
    assert_eq!(updated.timings.reused_files, 2);
    assert_fresh(&mut fixture.cache(), fixture.root.path());
}

#[cfg(unix)]
#[test]
fn replacing_a_cached_file_with_an_external_symlink_removes_its_contents() {
    let fixture = Fixture::new();
    fixture.populate();
    let mut cache = fixture.cache();
    assert_fresh(&mut cache, fixture.root.path());
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), "private archive checksum outside workspace").unwrap();
    let path = fixture.root.path().join("src/archive.rs");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(outside.path(), path).unwrap();
    let updated = assert_fresh(&mut cache, fixture.root.path());
    assert!(!paths(updated.snapshot.chunks()).contains(&"src/archive.rs"));
    assert_fresh(&mut fixture.cache(), fixture.root.path());
}
