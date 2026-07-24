use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
};

use tokio::runtime::Handle;

use crate::event::Event;

pub(crate) const MAX_FILES: usize = 512;
const MAX_DEPTH: usize = 8;

pub(crate) struct ExplorerScanResult {
    pub(crate) generation: u64,
    pub(crate) root: PathBuf,
    pub(crate) files: Vec<PathBuf>,
}

struct ScanRequest {
    generation: u64,
    root: PathBuf,
}

#[derive(Default)]
struct ScanScheduler {
    active: bool,
    latest_pending: Option<ScanRequest>,
}

#[derive(Clone)]
struct Runner {
    sender: Sender<Event>,
    runtime: Handle,
    generation: Arc<AtomicU64>,
    scheduler: Arc<Mutex<ScanScheduler>>,
}

pub(crate) struct ExplorerWorker {
    runner: Runner,
}

impl ExplorerWorker {
    pub(crate) fn new(sender: Sender<Event>, runtime: Handle) -> Self {
        Self {
            runner: Runner {
                sender,
                runtime,
                generation: Arc::new(AtomicU64::new(0)),
                scheduler: Arc::new(Mutex::new(ScanScheduler::default())),
            },
        }
    }

    pub(crate) fn spawn(&self, root: PathBuf) -> u64 {
        let generation = self
            .runner
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let request = ScanRequest { generation, root };
        let start = match self.runner.scheduler.lock() {
            Ok(mut scheduler) if scheduler.active => {
                scheduler.latest_pending = Some(request);
                None
            }
            Ok(mut scheduler) => {
                scheduler.active = true;
                Some(request)
            }
            Err(_) => None,
        };
        if let Some(request) = start {
            start_request(self.runner.clone(), request);
        }
        generation
    }
}

fn start_request(runner: Runner, request: ScanRequest) {
    let runtime = runner.runtime.clone();
    drop(runtime.spawn_blocking(move || {
        let cancelled = || runner.generation.load(Ordering::Acquire) != request.generation;
        let mut files = Vec::new();
        scan(&request.root, 0, &mut files, &cancelled);
        files.sort();
        if !cancelled() {
            let _ = runner
                .sender
                .send(Event::ExplorerScanFinished(ExplorerScanResult {
                    generation: request.generation,
                    root: request.root,
                    files,
                }));
        }
        finish_request(runner);
    }));
}

fn finish_request(runner: Runner) {
    let next = match runner.scheduler.lock() {
        Ok(mut scheduler) => {
            if let Some(request) = scheduler.latest_pending.take() {
                Some(request)
            } else {
                scheduler.active = false;
                None
            }
        }
        Err(_) => None,
    };
    if let Some(request) = next {
        start_request(runner, request);
    }
}

fn scan(directory: &Path, depth: usize, files: &mut Vec<PathBuf>, cancelled: &impl Fn() -> bool) {
    if cancelled() || depth > MAX_DEPTH || files.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if cancelled() || files.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if !entry.file_name().to_string_lossy().starts_with('.')
                && entry.file_name() != "target"
            {
                scan(&path, depth + 1, files, cancelled);
            }
        } else if path.extension().is_some_and(|extension| extension == "typ") {
            files.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        sync::mpsc::channel,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use oxyst_theme::{ColorDepth, Theme, ThemeName};

    use super::ExplorerWorker;
    use crate::components::FileExplorer;
    use crate::event::Event;

    #[test]
    fn scanner_keeps_only_typst_files_and_latest_generation() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("oxyst-scan-{}-{unique}", std::process::id()));
        let latest = root.join("latest");
        fs::create_dir_all(root.join("old"))?;
        fs::create_dir_all(&latest)?;
        fs::write(root.join("old/old.typ"), "")?;
        fs::write(latest.join("main.typ"), "")?;
        fs::write(latest.join("ignored.txt"), "")?;

        let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
        let (sender, receiver) = channel();
        let worker = ExplorerWorker::new(sender, runtime.handle().clone());
        worker.spawn(root.join("old"));
        let generation = worker.spawn(latest.clone());

        let Event::ExplorerScanFinished(result) = receiver.recv_timeout(Duration::from_secs(5))?
        else {
            return Err("worker returned an unexpected event".into());
        };
        assert_eq!(result.generation, generation);
        assert_eq!(result.root, latest);
        assert_eq!(result.files.len(), 1);
        assert!(result.files[0].ends_with("main.typ"));

        drop(worker);
        runtime.shutdown_timeout(Duration::from_millis(100));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[ignore = "manual release-mode performance probe"]
    fn large_project_rescan_vs_incremental_update() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "oxyst-explorer-perf-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        for directory in 0..100 {
            let path = root.join(format!("dir-{directory:03}"));
            fs::create_dir(&path)?;
            for file in 0..200 {
                fs::write(path.join(format!("asset-{file:03}.txt")), "")?;
            }
            for file in 0..5 {
                fs::write(path.join(format!("doc-{file:03}.typ")), "")?;
            }
        }

        let mut scan_samples = Vec::new();
        let mut files = Vec::new();
        for _ in 0..21 {
            files.clear();
            let started = Instant::now();
            super::scan(&root, 0, &mut files, &|| false);
            scan_samples.push(started.elapsed());
        }

        let added = root.join("dir-050/added.typ");
        fs::write(&added, "")?;
        let mut explorer = FileExplorer::new(
            root.clone(),
            Theme::new(ThemeName::Dark, ColorDepth::Ansi16),
        );
        let mut incremental_samples = Vec::new();
        for _ in 0..21 {
            explorer.install_files(files.clone());
            let started = Instant::now();
            assert!(!explorer.apply_paths(std::slice::from_ref(&added)));
            incremental_samples.push(started.elapsed());
        }

        scan_samples.sort_unstable();
        incremental_samples.sort_unstable();
        eprintln!(
            "20k non-Typst + 500 Typst files: rescan_p50={:?}, rescan_p95={:?}, incremental_p50={:?}, incremental_p95={:?}",
            scan_samples[10], scan_samples[19], incremental_samples[10], incremental_samples[19]
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
