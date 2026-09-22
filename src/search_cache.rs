//! Disposable, content-validated preparation shared by CLI and MCP searches.
//!
//! Discovery remains authoritative. Persistent queries reconcile metadata and
//! native invalidation hints, with full content checks on uncertainty.
mod watch;
use crate::{
    navigation::{FileFacts, NavigationIndex, NavigationPreparer},
    ranking::RankingIntent,
    search::{self, Chunk, PreparedCorpus, PreparedFile, PreparedStatistics},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

// Bump whenever chunking, tokenization, symbol extraction, or ranking features change.
const FORMAT_VERSION: u32 = 4;
const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheTimings {
    pub status: String,
    pub scan_ms: u64,
    pub load_ms: u64,
    /// Scan and load durations are independent elapsed phases and may overlap.
    pub scan_load_overlapped: bool,
    pub prepare_ms: u64,
    pub aggregate_ms: u64,
    pub aggregate_reused: bool,
    pub navigation_ms: u64,
    pub save_ms: u64,
    pub total_ms: u64,
    pub reused_files: usize,
    pub rebuilt_files: usize,
    pub read_files: usize,
    pub reused_contents: usize,
    pub validation: String,
    pub validation_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

/// One captured view is used for ranking and all subsequent source evidence.
pub struct WorkspaceSnapshot {
    chunks: Arc<[Chunk]>,
    prepared: PreparedCorpus,
    navigation: NavigationIndex,
    /// Built by the first search that needs it, then shared by every later one.
    links: std::sync::OnceLock<crate::connected::Links>,
}
impl WorkspaceSnapshot {
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }
    pub fn navigation(&self) -> &NavigationIndex {
        &self.navigation
    }
    pub fn rank(&self, question: &str) -> Vec<Chunk> {
        self.prepared.rank(question, |_| true)
    }
    pub fn rank_with_intent(&self, question: &str, intent: RankingIntent) -> Vec<Chunk> {
        self.prepared.rank_with_intent(question, |_| true, intent)
    }
    /// Rank unseen candidates using the existing index and corpus statistics.
    pub fn rank_excluding(
        &self,
        question: &str,
        intent: RankingIntent,
        seen: &[Chunk],
    ) -> Vec<Chunk> {
        self.prepared.rank_with_intent(
            question,
            |candidate| {
                !seen.iter().any(|old| {
                    old.path == candidate.path
                        && old.start_line == candidate.start_line
                        && old.end_line == candidate.end_line
                })
            },
            intent,
        )
    }
    /// Candidates one hop from the strongest of `shortlist`, from files it lacks.
    pub fn connected_to(&self, shortlist: &[Chunk], question: &str) -> Vec<Chunk> {
        self.links
            .get_or_init(|| crate::connected::Links::new(&self.chunks))
            .connected_to(&self.chunks, shortlist, question)
    }
}

pub struct CachedWorkspace {
    pub snapshot: Arc<WorkspaceSnapshot>,
    pub timings: CacheTimings,
}

#[derive(Serialize, Deserialize)]
struct CachedFile {
    path: String,
    digest: String,
    prepared: PreparedFile,
    navigation: Arc<FileFacts>,
}
#[derive(Serialize, Deserialize)]
struct DiskSnapshot {
    version: u32,
    root: PathBuf,
    files: Vec<CachedFile>,
    statistics: PreparedStatistics,
}
struct MemorySnapshot {
    record: DiskSnapshot,
    snapshot: Arc<WorkspaceSnapshot>,
}

/// Retains just the most recently searched directory in memory. Separate roots
/// (including subdirectories and worktrees) have separate disk snapshots.
pub struct WorkspaceCache {
    directory: Option<PathBuf>,
    enabled: bool,
    previous: Option<MemorySnapshot>,
    watching: Option<watch::WorkspaceWatch>,
    watch_enabled: bool,
}
impl Default for WorkspaceCache {
    fn default() -> Self {
        Self::new()
    }
}
impl WorkspaceCache {
    /// Honor OKO_CACHE_DIR, OKO_NO_CACHE and OKO_NO_WATCH.
    /// No setup is needed by default.
    pub fn new() -> Self {
        let disabled = std::env::var("OKO_NO_CACHE").is_ok_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
        let directory = std::env::var_os("OKO_CACHE_DIR")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::cache_dir().map(|path| path.join("oko").join("search")));
        Self {
            directory,
            enabled: !disabled,
            previous: None,
            watching: None,
            watch_enabled: !std::env::var("OKO_NO_WATCH").is_ok_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            }),
        }
    }

    /// An explicit directory, independent of environment overrides. Useful for
    /// embedders and isolated tests; this directory contains disposable data.
    pub fn with_directory(directory: PathBuf) -> Self {
        Self {
            directory: Some(directory),
            enabled: true,
            previous: None,
            watching: None,
            watch_enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            directory: None,
            enabled: false,
            previous: None,
            watching: None,
            watch_enabled: false,
        }
    }

    /// Whether a load is retained in memory for the next request.
    pub fn retains_snapshots(&self) -> bool {
        self.enabled
    }

    /// Keep disk caching, but fully read contents on each load. One-shot
    /// callers avoid starting a watcher that cannot benefit later requests.
    pub fn without_watching(mut self) -> Self {
        self.watch_enabled = false;
        self.watching = None;
        self
    }

    pub fn load(&mut self, root: &Path) -> Result<CachedWorkspace> {
        let started = Instant::now();
        let root = root
            .canonicalize()
            .context("Cannot resolve workspace root")?;
        let mut timings = CacheTimings::default();
        // A selected scope has its own relative paths and BM25 statistics.
        let cache_path = if self.enabled {
            self.cache_path(&root).filter(|path| {
                if path.starts_with(&root) {
                    timings.fallback_reason =
                        Some("Disk cache must be outside the searched directory".into());
                    false
                } else {
                    true
                }
            })
        } else {
            None
        };
        let has_previous = self.enabled
            && self
                .previous
                .as_ref()
                .is_some_and(|previous| previous.record.root == root);
        if self.enabled
            && self.watch_enabled
            && self
                .watching
                .as_ref()
                .is_none_or(|watcher| watcher.root() != root)
        {
            // Register before the initial full scan so edits during startup
            // remain pending instead of falling into a watch-registration gap.
            self.watching = Some(watch::WorkspaceWatch::new(&root));
        }
        // Deserialization can overlap fresh reads, but no disk data is used
        // until discovery succeeds and its content manifest has been compared.
        // Same-root memory snapshots never trigger a redundant disk load.
        let captured = scan_and_load_snapshot(
            &root,
            if has_previous {
                None
            } else {
                cache_path.as_deref()
            },
            self.watching.as_mut(),
        )?;
        let sources = captured.capture.sources;
        timings.read_files = captured.capture.read_files;
        timings.reused_contents = captured.capture.reused_contents;
        timings.validation = captured.capture.validation.into();
        timings.validation_reason = captured.capture.validation_reason.into();
        if let Some(reason) = captured.capture.reason {
            timings.fallback_reason = Some(reason);
        }
        timings.scan_ms = captured.scan_ms;
        timings.load_ms = captured.load_ms;
        timings.scan_load_overlapped = captured.overlapped;
        if self.enabled
            && let Some(previous) = &self.previous
            && previous.record.root == root
            && same_manifest(&previous.record.files, &sources)
        {
            timings.status = "memory".into();
            timings.aggregate_reused = true;
            timings.reused_files = sources.len();
            timings.total_ms = elapsed_ms(started);
            return Ok(CachedWorkspace {
                snapshot: Arc::clone(&previous.snapshot),
                timings,
            });
        }

        let previous = self.previous.take().filter(|p| p.record.root == root);
        let mut loaded_disk = false;
        let mut record = if self.enabled {
            if let Some(previous) = previous {
                Some(previous.record)
            } else if let Some(loaded) = captured.disk {
                match loaded {
                    Ok(record) => {
                        loaded_disk = true;
                        Some(record)
                    }
                    Err(error) => {
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_none_or(|error| error.kind() != std::io::ErrorKind::NotFound)
                        {
                            timings.fallback_reason = Some(format!("Cache load: {error}"));
                        }
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
        let unchanged_disk = loaded_disk
            && record
                .as_ref()
                .is_some_and(|r| same_manifest(&r.files, &sources));
        let had_record = record.is_some();
        let statistics = if unchanged_disk {
            record.as_ref().map(|record| record.statistics.clone())
        } else {
            None
        };
        let mut old_files: HashMap<_, _> = record
            .take()
            .into_iter()
            .flat_map(|r| r.files)
            .map(|file| (file.path.clone(), file))
            .collect();
        let prepare_started = Instant::now();
        let workers = preparation_workers(sources.len());
        let jobs = sources
            .into_iter()
            .map(|source| {
                let cached = old_files.remove(&source.path);
                (source, cached)
            })
            .collect();
        // Release deleted files before assembling the corpus.
        drop(old_files);
        let prepared_files = prepare_files(jobs, workers)?;
        let mut chunks = Vec::new();
        let mut files = Vec::with_capacity(prepared_files.len());
        for prepared in prepared_files {
            timings.reused_files += usize::from(prepared.reused);
            timings.rebuilt_files += usize::from(!prepared.reused);
            chunks.extend(prepared.chunks);
            files.push(prepared.file);
        }
        timings.prepare_ms = elapsed_ms(prepare_started);
        let aggregate_started = Instant::now();
        let chunks: Arc<[Chunk]> = chunks.into();
        let cached_prepared = statistics
            .as_ref()
            .filter(|_| timings.rebuilt_files == 0)
            .and_then(|statistics| {
                match PreparedCorpus::from_cached_statistics(
                    Arc::clone(&chunks),
                    files.iter().map(|file| &file.prepared),
                    statistics,
                ) {
                    Ok(prepared) => {
                        timings.aggregate_reused = true;
                        Some(prepared)
                    }
                    Err(error) => {
                        timings.fallback_reason = Some(format!("Cache statistics: {error}"));
                        None
                    }
                }
            });
        let prepared = match cached_prepared.map(Ok).unwrap_or_else(|| {
            PreparedCorpus::from_shared_files(
                Arc::clone(&chunks),
                files.iter().map(|f| &f.prepared),
            )
        }) {
            Ok(prepared) => prepared,
            Err(error) => {
                // Even an unexpected cache incompatibility must not break a
                // search that can be computed from the captured source.
                timings.fallback_reason = Some(format!("Cache assembly: {error}"));
                let mut preparer = search::FilePreparer::default();
                for (file, file_chunks) in files
                    .iter_mut()
                    .zip(chunks.chunk_by(|a, b| a.path == b.path))
                {
                    file.prepared = preparer.prepare_file(file_chunks);
                }
                timings.rebuilt_files = files.len();
                timings.reused_files = 0;
                PreparedCorpus::from_shared_files(
                    Arc::clone(&chunks),
                    files.iter().map(|f| &f.prepared),
                )?
            }
        };
        timings.aggregate_ms = elapsed_ms(aggregate_started);
        let statistics = prepared.statistics();
        let navigation_started = Instant::now();
        let navigation = NavigationIndex::new_shared(
            files
                .iter()
                .map(|file| (file.path.as_str(), Arc::clone(&file.navigation))),
        );
        timings.navigation_ms = elapsed_ms(navigation_started);
        let snapshot = Arc::new(WorkspaceSnapshot {
            chunks,
            prepared,
            navigation,
            links: std::sync::OnceLock::new(),
        });
        let record = DiskSnapshot {
            version: FORMAT_VERSION,
            root,
            files,
            statistics,
        };
        timings.status = if !self.enabled {
            "disabled"
        } else if unchanged_disk && timings.rebuilt_files == 0 {
            "disk"
        } else if had_record {
            "refresh"
        } else {
            "cold"
        }
        .into();
        if self.enabled {
            if (!unchanged_disk || timings.rebuilt_files > 0 || !timings.aggregate_reused)
                && let Some(path) = cache_path
            {
                let save_started = Instant::now();
                if let Err(error) = write_snapshot(&path, &record) {
                    timings.fallback_reason = Some(format!("Cache save: {error}"));
                }
                timings.save_ms = elapsed_ms(save_started);
            }
            self.previous = Some(MemorySnapshot {
                record,
                snapshot: Arc::clone(&snapshot),
            });
        }
        timings.total_ms = elapsed_ms(started);
        Ok(CachedWorkspace { snapshot, timings })
    }

    fn cache_path(&self, root: &Path) -> Option<PathBuf> {
        let directory = resolve_cache_directory(self.directory.as_ref()?).ok()?;
        // Non-Unicode roots work in memory; avoid lossy namespace collisions on disk.
        let root = root.to_str()?;
        let namespace = Sha256::digest(root.as_bytes());
        Some(directory.join(format!("v{FORMAT_VERSION}-{namespace:x}.bin")))
    }
}

struct CapturedWorkspace {
    capture: watch::CapturedSources,
    disk: Option<Result<DiskSnapshot>>,
    scan_ms: u64,
    load_ms: u64,
    overlapped: bool,
}

fn scan_and_load_snapshot(
    root: &Path,
    path: Option<&Path>,
    watcher: Option<&mut watch::WorkspaceWatch>,
) -> Result<CapturedWorkspace> {
    let capture = || match watcher {
        Some(watcher) => watcher.capture(),
        None => watch::capture_full(root),
    };
    let Some(path) = path else {
        let started = Instant::now();
        return Ok(CapturedWorkspace {
            capture: capture()?,
            disk: None,
            scan_ms: elapsed_ms(started),
            load_ms: 0,
            overlapped: false,
        });
    };
    thread::scope(|scope| {
        let load = || {
            let started = Instant::now();
            (read_snapshot(path, root), elapsed_ms(started))
        };
        let loader = thread::Builder::new().spawn_scoped(scope, load);
        let started = Instant::now();
        let sources = capture();
        let scan_ms = elapsed_ms(started);
        // Always join, even if discovery failed. Disk errors remain cache misses
        // while the original discovery error remains authoritative.
        let (disk, load_ms, overlapped) = match loader {
            Ok(loader) => {
                let (disk, load_ms) = loader
                    .join()
                    .unwrap_or_else(|_| (Err(anyhow::anyhow!("Cache loader failed")), 0));
                (disk, load_ms, true)
            }
            Err(_) => {
                let (disk, load_ms) = load();
                (disk, load_ms, false)
            }
        };
        Ok(CapturedWorkspace {
            capture: sources?,
            disk: Some(disk),
            scan_ms,
            load_ms,
            overlapped,
        })
    })
}

type PrepareJob = (search::WorkspaceFile, Option<CachedFile>);
struct PreparedSource {
    file: CachedFile,
    chunks: Vec<Chunk>,
    reused: bool,
}

fn preparation_workers(files: usize) -> usize {
    if files < 64 {
        1
    } else {
        thread::available_parallelism().map_or(1, |count| count.get().min(4))
    }
}

fn prepare_source(
    job: PrepareJob,
    preparer: &mut search::FilePreparer,
    navigator: &mut NavigationPreparer,
) -> PreparedSource {
    let (source, cached) = job;
    let cached = cached
        .filter(|file| {
            file.digest == source.digest
                && file.navigation.matches_captured_source(
                    &source.path,
                    source.text.len(),
                    &source.text_digest,
                )
        })
        .and_then(|file| {
            file.prepared
                .restore_chunks(&source.path, &source.text)
                .map(|chunks| (file, chunks))
        });
    let (prepared, navigation, chunks, reused) = if let Some((file, chunks)) = cached {
        (file.prepared, file.navigation, chunks, true)
    } else {
        let chunks = search::chunk_text(&source.path, &source.text);
        (
            preparer.prepare_file(&chunks),
            Arc::new(navigator.prepare(&source.path, &source.text)),
            chunks,
            false,
        )
    };
    PreparedSource {
        file: CachedFile {
            path: source.path,
            digest: source.digest,
            prepared,
            navigation,
        },
        chunks,
        reused,
    }
}

fn prepare_files(jobs: Vec<PrepareJob>, workers: usize) -> Result<Vec<PreparedSource>> {
    if workers <= 1 || jobs.len() < 2 {
        let mut preparer = search::FilePreparer::default();
        let mut navigator = NavigationPreparer::default();
        return Ok(jobs
            .into_iter()
            .map(|job| prepare_source(job, &mut preparer, &mut navigator))
            .collect());
    }
    let next = Mutex::new(jobs.into_iter().enumerate());
    let work = || -> Result<Vec<(usize, PreparedSource)>> {
        // Stemming maps are private to workers. Only already captured jobs
        // cross threads; preparation never reads the filesystem again.
        let mut preparer = search::FilePreparer::default();
        let mut navigator = NavigationPreparer::default();
        let mut completed = Vec::new();
        loop {
            let job = next
                .lock()
                .map_err(|_| anyhow::anyhow!("Preparation queue failed"))?
                .next();
            let Some((index, job)) = job else {
                break;
            };
            completed.push((index, prepare_source(job, &mut preparer, &mut navigator)));
        }
        Ok(completed)
    };
    thread::scope(|scope| {
        // The caller is one worker. Failed spawns simply leave more jobs for
        // it and the successfully started workers; no work can be lost.
        let handles: Vec<_> = (1..workers.min(4))
            .filter_map(|_| thread::Builder::new().spawn_scoped(scope, work).ok())
            .collect();
        let mut completed = Vec::new();
        let mut first_error = None;
        let mut collect = |result: Result<Vec<(usize, PreparedSource)>>| match result {
            Ok(batch) => completed.extend(batch),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        };
        collect(work());
        for handle in handles {
            collect(
                handle
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("Preparation worker failed"))),
            );
        }
        // Join every worker before propagating any failure. Otherwise scoped
        // auto-joining could re-panic while unwinding an earlier worker error.
        if let Some(error) = first_error {
            return Err(error);
        }
        completed.sort_unstable_by_key(|(index, _)| *index);
        Ok(completed
            .into_iter()
            .map(|(_, prepared)| prepared)
            .collect())
    })
}

// Resolve existing symlink ancestors even before the cache directory exists.
// This also prevents an override beneath the search root from indexing itself.
fn resolve_cache_directory(path: &Path) -> std::io::Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(name) = path.file_name() else {
                return Err(error);
            };
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(resolve_cache_directory(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}
fn same_manifest(files: &[CachedFile], sources: &[search::WorkspaceFile]) -> bool {
    files.len() == sources.len()
        && files
            .iter()
            .zip(sources)
            .all(|(cached, source)| cached.path == source.path && cached.digest == source.digest)
}

fn read_snapshot(path: &Path, root: &Path) -> Result<DiskSnapshot> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_SNAPSHOT_BYTES {
        bail!("Snapshot exceeds the cache size limit");
    }
    let mut bytes = Vec::new();
    file.take(MAX_SNAPSHOT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES || bytes.len() < 65 || bytes[64] != b'\n' {
        bail!("Invalid cache snapshot envelope");
    }
    let (checksum, body) = bytes.split_at(64);
    let body = &body[1..];
    if format!("{:x}", Sha256::digest(body)).as_bytes() != checksum {
        bail!("Cache snapshot checksum mismatch");
    }
    let (snapshot, remaining): (DiskSnapshot, _) = postcard::take_from_bytes(body)?;
    if !remaining.is_empty() {
        bail!("Trailing bytes in cache snapshot");
    }
    if snapshot.version != FORMAT_VERSION || snapshot.root != root {
        bail!("Cache snapshot format or scope mismatch");
    }
    if snapshot
        .files
        .windows(2)
        .any(|pair| search::compare_text(&pair[0].path, &pair[1].path) != std::cmp::Ordering::Less)
    {
        bail!("Cache snapshot has duplicate or unordered paths");
    }
    Ok(snapshot)
}

fn write_snapshot(path: &Path, record: &DiskSnapshot) -> Result<()> {
    let directory = path
        .parent()
        .context("Cache file has no parent directory")?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)?;
    let bytes = postcard::to_allocvec(record)?;
    if bytes.len() as u64 + 65 > MAX_SNAPSHOT_BYTES {
        bail!("Snapshot exceeds the cache size limit");
    }
    // tempfile creates owner-private files on Unix. Keeping the temporary file
    // in the same directory makes replacement atomic, including across clients.
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    writeln!(temporary, "{:x}", Sha256::digest(&bytes))?;
    temporary.write_all(&bytes)?;
    temporary.flush()?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, WorkspaceCache) {
        let root = tempfile::tempdir().unwrap();
        let disk = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("store.rs"),
            "fn store_record() { persist(); }\n",
        )
        .unwrap();
        let mut cache = WorkspaceCache::with_directory(disk.path().to_path_buf());
        cache.load(root.path()).unwrap();
        (root, disk, cache)
    }

    #[test]
    fn implementation_ranking_matches_cold_memory_disk_and_disabled_cache() {
        let root = tempfile::tempdir().unwrap();
        let disk = tempfile::tempdir().unwrap();
        for index in 0..35 {
            fs::write(
                root.path().join(format!("guide{index}.md")),
                "select tenant destination\n",
            )
            .unwrap();
        }
        fs::write(
            root.path().join("handler.ts"),
            "export function choose() {\n  return tenant.destination;\n}\n",
        )
        .unwrap();
        let mut cache = WorkspaceCache::with_directory(disk.path().to_owned());
        let cold = cache.load(root.path()).unwrap();
        let question = "select tenant destination";
        let expected = search::rank_lexically_with_intent(
            cold.snapshot.chunks(),
            question,
            RankingIntent::Implementation,
        );
        assert!(expected.iter().any(|chunk| chunk.path == "handler.ts"));
        assert_eq!(
            cold.snapshot
                .rank_with_intent(question, RankingIntent::Implementation),
            expected
        );
        let memory = cache.load(root.path()).unwrap();
        let loaded = WorkspaceCache::with_directory(disk.path().to_owned())
            .load(root.path())
            .unwrap();
        let disabled = WorkspaceCache::disabled().load(root.path()).unwrap();
        assert_eq!(memory.timings.status, "memory");
        assert_eq!(loaded.timings.status, "disk");
        assert!(loaded.timings.aggregate_reused);
        for snapshot in [&memory.snapshot, &loaded.snapshot, &disabled.snapshot] {
            assert_eq!(
                snapshot.rank_with_intent(question, RankingIntent::Implementation),
                expected
            );
        }
    }

    #[test]
    fn incompatible_and_structurally_invalid_snapshots_rebuild() {
        for corruption in [
            "version",
            "root",
            "features",
            "ranges",
            "duplicates",
            "checksum",
        ] {
            let (root, disk, cache) = fixture();
            let root_path = root.path().canonicalize().unwrap();
            let path = cache.cache_path(&root_path).unwrap();
            let mut record = read_snapshot(&path, &root_path).unwrap();
            match corruption {
                "version" => record.version += 1,
                "root" => record.root = disk.path().to_path_buf(),
                "duplicates" => {
                    let duplicate =
                        serde_json::from_value(serde_json::to_value(&record.files[0]).unwrap())
                            .unwrap();
                    record.files.push(duplicate);
                }
                "features" => {
                    let mut features = serde_json::to_value(&record.files[0].prepared).unwrap();
                    features["chunks"][0]["content"]["length"] = serde_json::json!(usize::MAX);
                    record.files[0].prepared = serde_json::from_value(features).unwrap();
                }
                "ranges" => {
                    let mut features = serde_json::to_value(&record.files[0].prepared).unwrap();
                    features["chunks"][0]["start_line"] = serde_json::json!(usize::MAX);
                    record.files[0].prepared = serde_json::from_value(features).unwrap();
                }
                "checksum" => {}
                _ => unreachable!(),
            }
            write_snapshot(&path, &record).unwrap();
            if corruption == "checksum" {
                let mut bytes = fs::read(&path).unwrap();
                bytes[0] = if bytes[0] == b'a' { b'b' } else { b'a' };
                fs::write(&path, bytes).unwrap();
            }
            let mut restart = WorkspaceCache::with_directory(disk.path().to_path_buf());
            let loaded = restart.load(root.path()).unwrap();
            assert_eq!(loaded.timings.rebuilt_files, 1, "{corruption}");
            assert_eq!(
                loaded.snapshot.rank("store record"),
                search::rank_lexically(
                    &search::workspace_chunks(root.path()).unwrap(),
                    "store record"
                )
            );
            assert_eq!(restart.load(root.path()).unwrap().timings.rebuilt_files, 0);
        }
    }

    #[test]
    fn malformed_aggregate_statistics_recompute_and_repair_the_disk_cache() {
        for corruption in [
            "documents",
            "length",
            "frequency",
            "missing-term",
            "weights",
            "weight-count",
        ] {
            let (root, disk, cache) = fixture();
            let root_path = root.path().canonicalize().unwrap();
            let path = cache.cache_path(&root_path).unwrap();
            let mut record = read_snapshot(&path, &root_path).unwrap();
            let mut statistics = serde_json::to_value(&record.statistics).unwrap();
            match corruption {
                "documents" => statistics["content"]["documents"] = serde_json::json!(usize::MAX),
                "length" => statistics["path"]["total_length"] = serde_json::json!(usize::MAX),
                "frequency" => {
                    for frequency in statistics["content"]["frequencies"]
                        .as_object_mut()
                        .unwrap()
                        .values_mut()
                    {
                        *frequency = serde_json::json!(0);
                    }
                }
                "missing-term" => statistics["content"]["frequencies"] = serde_json::json!({}),
                "weights" => statistics["reference_weights"][0][0] = serde_json::json!(4.0),
                "weight-count" => statistics["reference_weights"] = serde_json::json!([]),
                _ => unreachable!(),
            }
            record.statistics = serde_json::from_value(statistics).unwrap();
            write_snapshot(&path, &record).unwrap();
            let loaded = WorkspaceCache::with_directory(disk.path().to_path_buf())
                .load(root.path())
                .unwrap();
            assert!(!loaded.timings.aggregate_reused, "{corruption}");
            assert!(
                loaded
                    .timings
                    .fallback_reason
                    .as_deref()
                    .unwrap()
                    .contains("statistics")
            );
            assert_eq!(
                loaded.snapshot.rank("store record"),
                search::rank_lexically(
                    &search::workspace_chunks(root.path()).unwrap(),
                    "store record"
                ),
                "{corruption}"
            );
            let repaired = WorkspaceCache::with_directory(disk.path().to_path_buf())
                .load(root.path())
                .unwrap();
            assert!(repaired.timings.aggregate_reused, "{corruption}");
        }
    }

    #[test]
    fn cached_ranges_preserve_newlines_unicode_and_declaration_overlap() {
        for newline in ["\n", "\r\n", "\r"] {
            let root = tempfile::tempdir().unwrap();
            let disk = tempfile::tempdir().unwrap();
            for extension in ["rs", "py", "unknown"] {
                let mut lines = vec![
                    "\u{feff}// Unicode café document".to_owned(),
                    "fn archive_record() {".to_owned(),
                ];
                lines.extend((0..245).map(|index| format!("    persist_record({index});")));
                lines.extend([
                    "}".to_owned(),
                    "// Return current state".to_owned(),
                    "fn state() {}".to_owned(),
                ]);
                fs::write(
                    root.path().join(format!("source.{extension}")),
                    lines.join(newline),
                )
                .unwrap();
            }
            let cold = WorkspaceCache::with_directory(disk.path().to_path_buf())
                .load(root.path())
                .unwrap();
            let loaded = WorkspaceCache::with_directory(disk.path().to_path_buf())
                .load(root.path())
                .unwrap();
            assert!(loaded.timings.aggregate_reused);
            assert_eq!(loaded.timings.rebuilt_files, 0);
            assert_eq!(loaded.snapshot.chunks(), cold.snapshot.chunks());
            for intent in [
                RankingIntent::General,
                RankingIntent::Implementation,
                RankingIntent::Explanation,
            ] {
                assert_eq!(
                    loaded
                        .snapshot
                        .rank_with_intent("archive current record", intent),
                    search::rank_lexically_with_intent(
                        cold.snapshot.chunks(),
                        "archive current record",
                        intent
                    )
                );
            }
        }
    }

    #[test]
    fn parallel_preparation_preserves_fresh_cached_and_mixed_results() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..24 {
            fs::write(root.path().join(format!("{index:03}.rs")), format!(
                "// café archive transactions\r\npub fn archive_record_{index}() {{\r\n    recover_snapshot();\r\n}}\r\n"
            )).unwrap();
        }
        let capture = || search::workspace_files(root.path()).unwrap();
        let serial = prepare_files(
            capture().into_iter().map(|source| (source, None)).collect(),
            1,
        )
        .unwrap();
        let expected: Vec<_> = serial
            .iter()
            .flat_map(|source| source.chunks.iter().cloned())
            .collect();
        let questions = ["archive transactions", "recover snapshot", "unmatched"];
        let check = |prepared: &[PreparedSource]| {
            let chunks: Vec<_> = prepared
                .iter()
                .flat_map(|source| source.chunks.iter().cloned())
                .collect();
            assert_eq!(chunks, expected);
            let corpus = PreparedCorpus::from_shared_files(
                Arc::from(chunks),
                prepared.iter().map(|source| &source.file.prepared),
            )
            .unwrap();
            for question in questions {
                for intent in [
                    RankingIntent::General,
                    RankingIntent::Explanation,
                    RankingIntent::Implementation,
                ] {
                    assert_eq!(
                        corpus.rank_with_intent(question, |_| true, intent),
                        search::rank_lexically_with_intent(&expected, question, intent)
                    );
                }
            }
        };
        let parallel = prepare_files(
            capture().into_iter().map(|source| (source, None)).collect(),
            4,
        )
        .unwrap();
        assert!(parallel.iter().all(|source| !source.reused));
        check(&parallel);
        let cached = prepare_files(
            capture()
                .into_iter()
                .zip(parallel)
                .map(|(source, prepared)| (source, Some(prepared.file)))
                .collect(),
            4,
        )
        .unwrap();
        assert!(cached.iter().all(|source| source.reused));
        check(&cached);
        let mixed = prepare_files(
            capture()
                .into_iter()
                .zip(cached)
                .enumerate()
                .map(|(index, (source, prepared))| {
                    (source, (index % 2 == 0).then_some(prepared.file))
                })
                .collect(),
            4,
        )
        .unwrap();
        assert_eq!(mixed.iter().filter(|source| source.reused).count(), 12);
        check(&mixed);
    }

    #[test]
    fn interrupted_temporary_write_keeps_published_snapshot() {
        let (root, disk, _) = fixture();
        // Simulate a writer dying before persist: only the exact published name
        // is considered for reads; incomplete temporary files are irrelevant.
        fs::write(disk.path().join(".tmp-interrupted"), b"partial record").unwrap();
        let loaded = WorkspaceCache::with_directory(disk.path().to_path_buf())
            .load(root.path())
            .unwrap();
        assert_eq!(loaded.timings.status, "disk");
        assert_eq!(loaded.timings.rebuilt_files, 0);
    }

    #[test]
    fn cache_override_inside_source_is_not_written_or_indexed() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("store.rs"), "fn store_record() {}\n").unwrap();
        let directory = root.path().join("generated-cache");
        let mut cache = WorkspaceCache::with_directory(directory.clone());
        let first = cache.load(root.path()).unwrap();
        assert!(first.timings.fallback_reason.is_some());
        assert!(!directory.exists());
        assert_eq!(cache.load(root.path()).unwrap().timings.status, "memory");
    }

    #[test]
    fn oversized_snapshot_is_a_cache_miss() {
        let (root, disk, cache) = fixture();
        let path = cache
            .cache_path(&root.path().canonicalize().unwrap())
            .unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(MAX_SNAPSHOT_BYTES + 1)
            .unwrap();
        let loaded = WorkspaceCache::with_directory(disk.path().to_path_buf())
            .load(root.path())
            .unwrap();
        assert_eq!(loaded.timings.rebuilt_files, 1);
        assert!(loaded.timings.fallback_reason.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn published_snapshot_is_owner_private() {
        use std::os::unix::fs::PermissionsExt;
        let (root, _disk, cache) = fixture();
        let path = cache
            .cache_path(&root.path().canonicalize().unwrap())
            .unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
