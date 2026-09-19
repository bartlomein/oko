//! Native events are invalidation hints, never proof that files are unchanged.
use crate::search::{self, WorkspaceFile};
use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};

const MAX_DIRTY_PATHS: usize = 4096;
const FULL_VERIFY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RootIdentity {
    device: u64,
    inode: u64,
}

/// Independent metadata reads share only immutable captured state. The caller
/// applies decisions in discovery order after all bounded workers finish.
struct Reconciliation<'a> {
    root: &'a Path,
    identity: Option<RootIdentity>,
    sources: &'a HashMap<String, WorkspaceFile>,
    dirty: &'a HashSet<PathBuf>,
    now: SystemTime,
}

impl Reconciliation<'_> {
    fn reusable(&self, path: &str) -> bool {
        let absolute = self.root.join(path);
        if absolute.ancestors().any(|path| self.dirty.contains(path)) {
            return false;
        }
        self.sources.get(path).is_some_and(|source| {
            source.stamp.as_ref().is_some_and(|previous| {
                previous.can_reuse(self.now)
                    && self
                        .identity
                        .is_some_and(|root| root.device == previous.device())
                    && search::workspace_file_stamp(self.root, path)
                        .ok()
                        .flatten()
                        .as_ref()
                        == Some(previous)
            })
        })
    }

    fn decisions(&self, paths: &[String], workers: usize) -> Result<Vec<bool>> {
        let batch = |paths: &[String]| {
            paths
                .iter()
                .map(|path| self.reusable(path))
                .collect::<Vec<_>>()
        };
        if workers <= 1 || paths.is_empty() {
            return Ok(batch(paths));
        }
        thread::scope(|scope| {
            let handles: Vec<_> = paths
                .chunks(paths.len().div_ceil(workers.min(4)))
                .map(|paths| {
                    match thread::Builder::new().spawn_scoped(scope, move || batch(paths)) {
                        Ok(handle) => Ok(handle),
                        // Resource limits reduce concurrency without losing work.
                        Err(_) => Err(batch(paths)),
                    }
                })
                .collect();
            let mut decisions = Vec::with_capacity(paths.len());
            let mut failure = None;
            for handle in handles {
                match handle {
                    Ok(handle) => match handle.join() {
                        Ok(batch) => decisions.extend(batch),
                        Err(_) => {
                            failure.get_or_insert_with(|| {
                                anyhow::anyhow!("Metadata reconciliation worker failed")
                            });
                        }
                    },
                    Err(batch) => decisions.extend(batch),
                }
            }
            // Join every worker before returning an error; event hints remain
            // pending and capture() requests full reconciliation on failure.
            match failure {
                Some(error) => Err(error),
                None => Ok(decisions),
            }
        })
    }
}

fn root_identity(root: &Path) -> Option<RootIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = root.metadata().ok()?;
        metadata.is_dir().then_some(RootIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        None
    }
}

#[cfg(any(target_os = "linux", test))]
fn known_linux_filesystem(magic: u64) -> bool {
    // ext2/3/4, btrfs, XFS, tmpfs, F2FS, ZFS and ramfs. Network, overlay,
    // pseudo and unrecognized filesystems deliberately retain content reads.
    matches!(
        magic,
        0xef53 | 0x9123_683e | 0x5846_5342 | 0x0102_1994 | 0xf2f5_2010 | 0x2fc1_2fc1 | 0x8584_58f6
    )
}

#[cfg(any(target_os = "macos", test))]
fn known_macos_filesystem(name: &[u8]) -> bool {
    matches!(name, b"apfs" | b"hfs")
}

fn local_filesystem(root: &Path) -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::{ffi::CString, mem::MaybeUninit, os::unix::ffi::OsStrExt};
        let Ok(path) = CString::new(root.as_os_str().as_bytes()) else {
            return false;
        };
        let mut info = MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: path is NUL-terminated and info points to a writable statfs.
        if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return false;
        }
        // SAFETY: successful statfs initializes its output structure.
        let info = unsafe { info.assume_init() };
        #[cfg(target_os = "linux")]
        {
            known_linux_filesystem(info.f_type as u64)
        }
        #[cfg(target_os = "macos")]
        {
            let name: Vec<u8> = info
                .f_fstypename
                .iter()
                .map(|&byte| byte as u8)
                .take_while(|&byte| byte != 0)
                .collect();
            known_macos_filesystem(&name) && info.f_flags & libc::MNT_LOCAL as u32 != 0
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = root;
        false
    }
}

#[derive(Default)]
struct Events {
    paths: HashSet<PathBuf>,
    full: bool,
    failure: Option<String>,
}

impl Events {
    fn drain(&mut self) -> Self {
        Self {
            paths: std::mem::take(&mut self.paths),
            full: std::mem::take(&mut self.full),
            failure: self.failure.clone(),
        }
    }

    fn record(&mut self, result: notify::Result<Event>) {
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                self.failure = Some(format!("Filesystem watcher: {error}"));
                self.full = true;
                self.paths.clear();
                return;
            }
        };
        if event.need_rescan() || matches!(event.kind, EventKind::Any | EventKind::Other) {
            self.full = true;
            self.paths.clear();
            return;
        }
        // Our own reads can generate access events. Content and metadata
        // writes arrive as Modify/Create/Remove; metadata is checked regardless.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        if event.paths.is_empty() {
            self.full = true;
            return;
        }
        if !self.full {
            for path in event.paths {
                self.paths.insert(path);
                if self.paths.len() > MAX_DIRTY_PATHS {
                    self.full = true;
                    self.paths.clear();
                    break;
                }
            }
        }
    }
}

pub(super) struct CapturedSources {
    pub sources: Vec<WorkspaceFile>,
    pub read_files: usize,
    pub reused_contents: usize,
    pub validation: &'static str,
    pub validation_reason: &'static str,
    pub reason: Option<String>,
}

pub(super) struct WorkspaceWatch {
    root: PathBuf,
    // Dropping the cache/root closes native resources and callback threads.
    _watcher: Option<RecommendedWatcher>,
    events: Arc<Mutex<Events>>,
    sources: HashMap<String, WorkspaceFile>,
    last_full: Option<Instant>,
    identity: Option<RootIdentity>,
    local: bool,
}

impl WorkspaceWatch {
    pub fn new(root: &Path) -> Self {
        let events = Arc::new(Mutex::new(Events::default()));
        let identity = root_identity(root);
        let local = identity.is_some() && local_filesystem(root);
        if !local {
            return Self {
                root: root.to_owned(),
                _watcher: None,
                events,
                sources: HashMap::new(),
                last_full: None,
                identity,
                local,
            };
        }
        let callback_events = Arc::clone(&events);
        let watcher = RecommendedWatcher::new(
            move |event| {
                if let Ok(mut events) = callback_events.lock() {
                    events.record(event);
                }
            },
            Config::default().with_follow_symlinks(false),
        )
        .and_then(|mut watcher| {
            watcher.watch(root, RecursiveMode::Recursive)?;
            Ok(watcher)
        });
        let watcher = match watcher {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                events.lock().expect("new watcher state").record(Err(error));
                None
            }
        };
        Self {
            root: root.to_owned(),
            _watcher: watcher,
            events,
            sources: HashMap::new(),
            last_full: None,
            identity,
            local,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn capture(&mut self) -> Result<CapturedSources> {
        let result = self.capture_inner();
        if result.is_err()
            && let Ok(mut events) = self.events.lock()
        {
            // Drained hints cannot be lost when discovery fails halfway through.
            events.full = true;
        }
        result
    }

    fn capture_inner(&mut self) -> Result<CapturedSources> {
        if self.identity != root_identity(&self.root) {
            // Replaced roots invalidate both contents and native watch handles.
            let root = self.root.clone();
            *self = Self::new(&root);
        }
        // Drain before work, never after it. Events arriving during capture or
        // index construction remain pending for the next query.
        let Events {
            paths: dirty,
            full: requested_full,
            failure,
        } = match self.events.lock() {
            Ok(mut events) => events.drain(),
            Err(_) => Events {
                paths: HashSet::new(),
                full: true,
                failure: Some("Filesystem watcher state unavailable".into()),
            },
        };
        let validation_reason = if failure.is_some() {
            "watcher-error"
        } else if !self.local {
            "unsupported-filesystem"
        } else if self.last_full.is_none() {
            "startup"
        } else if requested_full {
            "events-rescan"
        } else if self
            .last_full
            .is_some_and(|last| last.elapsed() >= FULL_VERIFY_INTERVAL)
        {
            "periodic"
        } else {
            "metadata-and-events"
        };
        let full = validation_reason != "metadata-and-events";
        // This runs even with no events: additions, deletions, ignore rules and
        // branch changes must be visible before asynchronous delivery catches up.
        let paths = search::workspace_paths(&self.root)?;
        let decisions = if full {
            vec![false; paths.len()]
        } else {
            Reconciliation {
                root: &self.root,
                identity: self.identity,
                sources: &self.sources,
                dirty: &dirty,
                now: SystemTime::now(),
            }
            .decisions(&paths, super::preparation_workers(paths.len()))?
        };
        let mut reused = HashMap::new();
        let mut read = Vec::new();
        for (path, reusable) in paths.iter().zip(decisions) {
            if reusable {
                reused.insert(path.clone(), self.sources[path].clone());
            } else {
                read.push(path.clone());
            }
        }
        let workers = super::preparation_workers(read.len());
        let loaded = if self.local && failure.is_none() {
            search::read_workspace_paths_watched(&self.root, &read, workers)?
        } else {
            search::read_workspace_paths(&self.root, &read, workers)?
        };
        let mut captured: HashMap<_, _> = loaded
            .into_iter()
            .map(|source| (source.path.clone(), source))
            .collect();
        let reused_contents = reused.len();
        captured.extend(reused);
        // Paths from discovery dictate order, eligibility and deletions. Text
        // clones share Arc<str>, so retaining captures does not copy contents.
        let sources: Vec<_> = paths
            .into_iter()
            .filter_map(|path| captured.get(&path).cloned())
            .collect();
        self.sources = captured;
        if full {
            self.last_full = Some(Instant::now());
        }
        Ok(CapturedSources {
            sources,
            read_files: read.len(),
            reused_contents,
            validation: if full { "full" } else { "incremental" },
            reason: failure,
            validation_reason,
        })
    }
}

pub(super) fn capture_full(root: &Path) -> Result<CapturedSources> {
    let paths = search::workspace_paths(root)?;
    Ok(CapturedSources {
        sources: search::read_workspace_paths(
            root,
            &paths,
            super::preparation_workers(paths.len()),
        )?,
        read_files: paths.len(),
        reused_contents: 0,
        validation: "full",
        reason: None,
        validation_reason: "watching-disabled",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, Flag};

    #[test]
    fn filesystem_selection_excludes_network_unknown_and_overlay_filesystems() {
        for magic in [0xef53, 0x9123_683e, 0x5846_5342, 0x0102_1994] {
            assert!(known_linux_filesystem(magic));
        }
        for magic in [0x6969, 0xff53_4d42, 0x794c_7630, 0xdead_beef] {
            assert!(!known_linux_filesystem(magic));
        }
        assert!(known_macos_filesystem(b"apfs"));
        assert!(known_macos_filesystem(b"hfs"));
        for name in [b"nfs".as_slice(), b"smbfs", b"unknown"] {
            assert!(!known_macos_filesystem(name));
        }
    }

    #[test]
    fn parallel_reconciliation_preserves_serial_decisions_for_mixed_sources() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let mut paths: Vec<_> = (0..80)
            .map(|index| format!("source-{index:03}.rs"))
            .collect();
        for path in &paths {
            std::fs::write(root.join(path), "fn initial() {}\n").unwrap();
        }
        // Eligibility belongs to capture time, so age files before capturing.
        std::thread::sleep(Duration::from_millis(2100));
        let sources: HashMap<_, _> = search::read_workspace_paths_watched(&root, &paths, 4)
            .unwrap()
            .into_iter()
            .map(|source| (source.path.clone(), source))
            .collect();
        std::fs::write(root.join(&paths[12]), "fn changed() {}\n").unwrap();
        std::fs::remove_file(root.join(&paths[27])).unwrap();
        paths.push("new.rs".into());
        std::fs::write(root.join("new.rs"), "fn new_source() {}\n").unwrap();
        let dirty = HashSet::from([root.join(&paths[41])]);
        let reconciliation = Reconciliation {
            root: &root,
            identity: root_identity(&root),
            sources: &sources,
            dirty: &dirty,
            now: SystemTime::now(),
        };
        let serial = reconciliation.decisions(&paths, 1).unwrap();
        for _ in 0..3 {
            assert_eq!(reconciliation.decisions(&paths, 4).unwrap(), serial);
        }
        for index in [12, 27, 41, 80] {
            assert!(!serial[index]);
        }
        assert_eq!(
            serial[0],
            sources[&paths[0]].stamp.is_some() && reconciliation.identity.is_some()
        );
    }

    #[test]
    fn dropped_unknown_and_error_events_request_full_reconciliation() {
        for event in [
            Event::new(EventKind::Any),
            Event::new(EventKind::Other),
            Event::new(EventKind::Create(CreateKind::File)).set_flag(Flag::Rescan),
        ] {
            let mut events = Events::default();
            events.record(Ok(event));
            assert!(events.full);
        }
        let mut events = Events::default();
        events.record(Err(notify::Error::generic("backend stopped")));
        assert!(events.full && events.failure.is_some());
    }

    #[test]
    fn dirty_queue_is_bounded_and_overflow_requests_a_full_scan() {
        let mut events = Events::default();
        for index in 0..=MAX_DIRTY_PATHS {
            events.record(Ok(Event::new(EventKind::Create(CreateKind::File))
                .add_path(PathBuf::from(format!("file-{index}")))));
        }
        assert!(events.full);
        assert!(events.paths.is_empty());
    }

    #[test]
    fn events_arriving_after_capture_begins_remain_pending() {
        let mut events = Events::default();
        events.record(Ok(
            Event::new(EventKind::Create(CreateKind::File)).add_path("before.rs".into())
        ));
        let in_progress = events.drain();
        events.record(Ok(
            Event::new(EventKind::Create(CreateKind::File)).add_path("during.rs".into())
        ));
        assert!(in_progress.paths.contains(Path::new("before.rs")));
        assert!(!in_progress.paths.contains(Path::new("during.rs")));
        let next = events.drain();
        assert!(next.paths.contains(Path::new("during.rs")));
        assert!(!next.paths.contains(Path::new("before.rs")));
    }

    fn fixture() -> (tempfile::TempDir, WorkspaceWatch) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.rs"), "fn before() {}\n").unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let mut watcher = WorkspaceWatch::new(&canonical);
        // A deterministic backend substitute lets us inject faults and omit
        // events independently of the host's native event delivery timing.
        watcher._watcher = None;
        watcher.events = Arc::new(Mutex::new(Events::default()));
        // Event-policy tests use a deterministic trusted-backend substitute;
        // separate filesystem-selection tests cover unsupported host behavior.
        watcher.local = true;
        (root, watcher)
    }

    #[test]
    fn full_reconciliation_runs_after_periodic_deadline_rescan_and_failure() {
        let (root, mut watcher) = fixture();
        assert_eq!(watcher.capture().unwrap().validation_reason, "startup");
        watcher.last_full =
            Instant::now().checked_sub(FULL_VERIFY_INTERVAL + Duration::from_secs(1));
        let periodic = watcher.capture().unwrap();
        assert_eq!(periodic.validation_reason, "periodic");
        assert_eq!(periodic.read_files, 1);
        watcher
            .events
            .lock()
            .unwrap()
            .record(Ok(Event::new(EventKind::Other)));
        let rescan = watcher.capture().unwrap();
        assert_eq!(rescan.validation_reason, "events-rescan");
        assert_eq!(rescan.read_files, 1);
        std::fs::write(root.path().join("source.rs"), "fn after() {}\n").unwrap();
        watcher
            .events
            .lock()
            .unwrap()
            .record(Err(notify::Error::generic("backend stopped")));
        for _ in 0..2 {
            let failed = watcher.capture().unwrap();
            assert_eq!(failed.validation_reason, "watcher-error");
            assert_eq!(failed.read_files, 1);
            assert!(failed.sources[0].text.contains("fn after"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn missing_events_do_not_hide_same_size_edits_with_restored_mtime() {
        let (root, mut watcher) = fixture();
        std::thread::sleep(Duration::from_millis(2100));
        let first = watcher.capture().unwrap();
        let reusable = first.sources[0].stamp.is_some();
        assert_eq!(
            watcher.capture().unwrap().reused_contents,
            usize::from(reusable)
        );
        let path = root.path().join("source.rs");
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, "fn latter() {}\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let changed = watcher.capture().unwrap();
        assert_eq!(changed.read_files, 1);
        assert_eq!(changed.reused_contents, 0);
        assert!(changed.sources[0].text.contains("fn latter"));
        assert!(
            changed.sources[0].stamp.is_none(),
            "recent capture cannot become trusted just by aging"
        );
    }
}
