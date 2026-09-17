use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

use oxyst_compiler::{
    CompileOutcome, CompileSnapshot, CompiledDocument, Compiler, Diagnostic, DocumentSync,
};
use oxyst_render::RenderManifest;
use ratatui_image::{picker::Picker, sliced::SlicedProtocol};
use tokio::runtime::Handle;
use typst_syntax::Source;

use crate::encoding::encode_rendered_pages_cancellable;

const MAX_ACTIVE_NATIVE_COMPILES: usize = 2;

pub(crate) enum WorkerEvent {
    CompileFinished(CompileResult),
    PreviewPagesFinished(PreviewPageResult),
}

pub(crate) struct CompileResult {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) elapsed: Duration,
    pub(crate) rebuilt_compiler: Option<Compiler>,
    pub(crate) rebuild_attempted: bool,
    pub(crate) outcome: CompileResultKind,
}

pub(crate) enum CompileResultKind {
    Success {
        diagnostics: Vec<Diagnostic>,
        sync: DocumentSync,
        document: Box<CompiledDocument>,
    },
    Diagnostics(Vec<Diagnostic>),
    Error(String),
}

struct CompileRequest {
    generation: u64,
    revision: u64,
    input: CompileInput,
}

pub(crate) struct PreviewPageResult {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) request: u64,
    pub(crate) width: u16,
    pub(crate) requested: Vec<usize>,
    pub(crate) outcome: Result<Vec<(usize, SlicedProtocol)>, String>,
}

enum CompileInput {
    Snapshot(CompileSnapshot),
    Rebuild(WorldRebuild),
}

pub(crate) struct WorldRebuild {
    pub(crate) root: PathBuf,
    pub(crate) main: PathBuf,
    pub(crate) source: String,
}

pub(crate) struct PreviewPageRequest {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) document: CompiledDocument,
    pub(crate) manifest: RenderManifest,
    pub(crate) picker: Picker,
    pub(crate) width: u16,
    pub(crate) pages: Vec<usize>,
}

#[derive(Default)]
struct Scheduler {
    active_native_compiles: usize,
    latest_pending: Option<CompileRequest>,
}

struct QueuedPreviewRequest {
    id: u64,
    request: PreviewPageRequest,
}

#[derive(Default)]
struct PreviewScheduler {
    active: bool,
    latest_pending: Option<QueuedPreviewRequest>,
}

impl PreviewScheduler {
    fn enqueue(&mut self, request: QueuedPreviewRequest) -> Option<QueuedPreviewRequest> {
        if self.active {
            self.latest_pending = Some(request);
            None
        } else {
            self.active = true;
            Some(request)
        }
    }

    fn cancel_pending(&mut self) {
        self.latest_pending = None;
    }

    fn finish(&mut self) -> Option<QueuedPreviewRequest> {
        if let Some(request) = self.latest_pending.take() {
            Some(request)
        } else {
            self.active = false;
            None
        }
    }
}

impl Scheduler {
    fn enqueue(&mut self, request: CompileRequest) -> Option<CompileRequest> {
        if self.active_native_compiles < MAX_ACTIVE_NATIVE_COMPILES {
            self.active_native_compiles += 1;
            Some(request)
        } else {
            self.latest_pending = Some(request);
            None
        }
    }

    fn cancel_pending(&mut self) {
        self.latest_pending = None;
    }

    fn finish(&mut self) -> Option<CompileRequest> {
        self.active_native_compiles = self.active_native_compiles.saturating_sub(1);
        self.latest_pending
            .take()
            .inspect(|_| self.active_native_compiles += 1)
    }
}

#[derive(Clone)]
struct Runner {
    sender: Sender<WorkerEvent>,
    runtime: Handle,
    generation: Arc<AtomicU64>,
    preview_request: Arc<AtomicU64>,
    scheduler: Arc<Mutex<Scheduler>>,
    preview_scheduler: Arc<Mutex<PreviewScheduler>>,
}

pub(crate) struct CompileWorker {
    compiler: Mutex<Compiler>,
    runner: Runner,
}

impl CompileWorker {
    pub(crate) fn new(compiler: Compiler, sender: Sender<WorkerEvent>, runtime: Handle) -> Self {
        Self {
            compiler: Mutex::new(compiler),
            runner: Runner {
                sender,
                runtime,
                generation: Arc::new(AtomicU64::new(0)),
                preview_request: Arc::new(AtomicU64::new(0)),
                scheduler: Arc::new(Mutex::new(Scheduler::default())),
                preview_scheduler: Arc::new(Mutex::new(PreviewScheduler::default())),
            },
        }
    }

    pub(crate) fn invalidate(&self) -> Result<u64, String> {
        self.advance_and_cancel_pending()
    }

    pub(crate) fn invalidate_files(&self) -> Result<u64, String> {
        let generation = self.advance_and_cancel_pending()?;
        let mut compiler = self
            .compiler
            .lock()
            .map_err(|_| "compiler coordinator is unavailable".to_owned())?;
        compiler.invalidate_files();
        Ok(generation)
    }

    pub(crate) fn spawn(&self, revision: u64, source: Source) -> u64 {
        let generation = match self.advance_and_cancel_pending() {
            Ok(generation) => generation,
            Err(error) => return self.report_preparation_error(revision, error),
        };
        let snapshot = match self.compiler.lock() {
            Ok(compiler) => match compiler.snapshot_with_source(source) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.send_error(revision, generation, &error.to_string());
                    return generation;
                }
            },
            Err(_) => {
                self.send_error(revision, generation, "compiler coordinator is unavailable");
                return generation;
            }
        };
        self.enqueue(CompileRequest {
            generation,
            revision,
            input: CompileInput::Snapshot(snapshot),
        });
        generation
    }

    pub(crate) fn spawn_with_world(&self, revision: u64, rebuild: WorldRebuild) -> u64 {
        let generation = match self.advance_and_cancel_pending() {
            Ok(generation) => generation,
            Err(error) => return self.report_preparation_error(revision, error),
        };
        self.enqueue(CompileRequest {
            generation,
            revision,
            input: CompileInput::Rebuild(rebuild),
        });
        generation
    }

    pub(crate) fn install(&self, compiler: Compiler) -> Result<(), String> {
        let mut current = self
            .compiler
            .lock()
            .map_err(|_| "compiler coordinator is unavailable".to_owned())?;
        *current = compiler;
        Ok(())
    }

    pub(crate) fn spawn_preview_pages(&self, request: PreviewPageRequest) -> u64 {
        let request_id = advance(&self.runner.preview_request);
        let start = match self.runner.preview_scheduler.lock() {
            Ok(mut scheduler) => scheduler.enqueue(QueuedPreviewRequest {
                id: request_id,
                request,
            }),
            Err(_) => {
                let _ =
                    self.runner
                        .sender
                        .send(WorkerEvent::PreviewPagesFinished(PreviewPageResult {
                            generation: request.generation,
                            revision: request.revision,
                            request: request_id,
                            width: request.width,
                            requested: request.pages,
                            outcome: Err("preview scheduler is unavailable".to_owned()),
                        }));
                None
            }
        };
        if let Some(request) = start {
            start_preview_request(self.runner.clone(), request);
        }
        request_id
    }

    fn advance_and_cancel_pending(&self) -> Result<u64, String> {
        let generation = advance(&self.runner.generation);
        advance(&self.runner.preview_request);
        self.runner
            .preview_scheduler
            .lock()
            .map_err(|_| "preview scheduler is unavailable".to_owned())?
            .cancel_pending();
        self.runner
            .scheduler
            .lock()
            .map_err(|_| "compiler scheduler is unavailable".to_owned())?
            .cancel_pending();
        Ok(generation)
    }

    fn enqueue(&self, request: CompileRequest) {
        let start = match self.runner.scheduler.lock() {
            Ok(mut scheduler) => scheduler.enqueue(request),
            Err(_) => {
                self.send_error(
                    request.revision,
                    request.generation,
                    "compiler scheduler is unavailable",
                );
                None
            }
        };
        if let Some(request) = start {
            start_request(self.runner.clone(), request);
        }
    }

    fn report_preparation_error(&self, revision: u64, error: String) -> u64 {
        let generation = advance(&self.runner.generation);
        self.send_error(revision, generation, &error);
        generation
    }

    fn send_error(&self, revision: u64, generation: u64, error: &str) {
        let _ = self
            .runner
            .sender
            .send(WorkerEvent::CompileFinished(CompileResult {
                generation,
                revision,
                elapsed: Duration::ZERO,
                rebuilt_compiler: None,
                rebuild_attempted: false,
                outcome: CompileResultKind::Error(error.to_owned()),
            }));
    }
}

fn start_preview_request(runner: Runner, queued: QueuedPreviewRequest) {
    let runtime = runner.runtime.clone();
    drop(runtime.spawn_blocking(move || {
        if let Some(result) = render_preview_request(&runner, queued) {
            let _ = runner
                .sender
                .send(WorkerEvent::PreviewPagesFinished(result));
        }
        finish_preview_request(runner);
    }));
}

fn render_preview_request(
    runner: &Runner,
    queued: QueuedPreviewRequest,
) -> Option<PreviewPageResult> {
    let request = queued.request;
    let cancelled = || {
        !is_current(&runner.generation, request.generation)
            || !is_current(&runner.preview_request, queued.id)
    };
    let requested = request.pages.clone();
    let outcome = match oxyst_render::render_pages_cancellable(
        &request.document,
        &request.manifest,
        request.pages,
        cancelled,
    ) {
        Ok(rendered) => match encode_rendered_pages_cancellable(
            &request.picker,
            rendered,
            request.width,
            cancelled,
        ) {
            Ok(Some(pages)) => Ok(pages),
            Ok(None) => return None,
            Err(_) if cancelled() => return None,
            Err(error) => Err(error),
        },
        Err(oxyst_render::Error::Cancelled) => return None,
        Err(error) => Err(error.to_string()),
    };
    (!cancelled()).then_some(PreviewPageResult {
        generation: request.generation,
        revision: request.revision,
        request: queued.id,
        width: request.width,
        requested,
        outcome,
    })
}

fn finish_preview_request(runner: Runner) {
    let next = match runner.preview_scheduler.lock() {
        Ok(mut scheduler) => scheduler.finish(),
        Err(_) => None,
    };
    if let Some(request) = next {
        start_preview_request(runner, request);
    }
}

fn start_request(runner: Runner, request: CompileRequest) {
    let runtime = runner.runtime.clone();
    drop(runtime.spawn_blocking(move || {
        let result = compile(request, &runner.generation, Instant::now());
        if let Some(result) = result
            && is_current(&runner.generation, result.generation)
        {
            let _ = runner.sender.send(WorkerEvent::CompileFinished(result));
        }
        finish_request(runner);
    }));
}

fn finish_request(runner: Runner) {
    let next = match runner.scheduler.lock() {
        Ok(mut scheduler) => scheduler.finish(),
        Err(_) => None,
    };
    if let Some(request) = next {
        start_request(runner, request);
    }
}

fn compile(
    request: CompileRequest,
    generations: &AtomicU64,
    started: Instant,
) -> Option<CompileResult> {
    if !is_current(generations, request.generation) {
        return None;
    }
    let (snapshot, rebuilt_compiler, rebuild_attempted) = match request.input {
        CompileInput::Snapshot(snapshot) => (snapshot, None, false),
        CompileInput::Rebuild(rebuild) => {
            let mut compiler = match Compiler::new(rebuild.root, rebuild.main) {
                Ok(compiler) => compiler,
                Err(error) => {
                    return Some(CompileResult {
                        generation: request.generation,
                        revision: request.revision,
                        elapsed: started.elapsed(),
                        rebuilt_compiler: None,
                        rebuild_attempted: true,
                        outcome: CompileResultKind::Error(error.to_string()),
                    });
                }
            };
            compiler.replace_source(&rebuild.source);
            let snapshot = compiler.snapshot();
            (snapshot, Some(compiler), true)
        }
    };
    let compiled = match snapshot.compile() {
        CompileOutcome::Success(compiled) => compiled,
        CompileOutcome::Failure(diagnostics) => {
            return is_current(generations, request.generation).then_some(CompileResult {
                generation: request.generation,
                revision: request.revision,
                elapsed: started.elapsed(),
                rebuilt_compiler,
                rebuild_attempted,
                outcome: CompileResultKind::Diagnostics(diagnostics),
            });
        }
    };
    if !is_current(generations, request.generation) {
        return None;
    }
    let sync = compiled.sync();
    is_current(generations, request.generation).then_some(CompileResult {
        generation: request.generation,
        revision: request.revision,
        elapsed: started.elapsed(),
        rebuilt_compiler,
        rebuild_attempted,
        outcome: CompileResultKind::Success {
            diagnostics: compiled.warnings().to_vec(),
            sync,
            document: Box::new(compiled),
        },
    })
}

fn advance(generation: &AtomicU64) -> u64 {
    generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
}

fn is_current(generations: &AtomicU64, generation: u64) -> bool {
    generations.load(Ordering::Acquire) == generation
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf, sync::atomic::AtomicU64};

    use oxyst_compiler::Compiler;

    use oxyst_render::RenderManifest;
    use ratatui_image::picker::Picker;

    use super::{
        CompileInput, CompileRequest, PreviewPageRequest, PreviewScheduler, QueuedPreviewRequest,
        Scheduler, advance, is_current,
    };

    #[test]
    fn advancing_the_generation_invalidates_older_work() {
        let generations = AtomicU64::new(0);
        let first = advance(&generations);
        assert!(is_current(&generations, first));
        let second = advance(&generations);
        assert!(!is_current(&generations, first));
        assert!(is_current(&generations, second));
    }

    #[test]
    fn scheduler_bounds_native_jobs_and_retains_only_the_latest_request()
    -> Result<(), Box<dyn Error>> {
        let mut scheduler = Scheduler::default();
        assert_eq!(
            scheduler.enqueue(request(1)?).map(|job| job.generation),
            Some(1)
        );
        assert_eq!(
            scheduler.enqueue(request(2)?).map(|job| job.generation),
            Some(2)
        );
        assert!(scheduler.enqueue(request(3)?).is_none());
        assert!(scheduler.enqueue(request(4)?).is_none());
        assert_eq!(scheduler.finish().map(|job| job.generation), Some(4));
        assert_eq!(scheduler.active_native_compiles, 2);
        Ok(())
    }

    #[test]
    fn cancelling_drops_pending_compile_work() -> Result<(), Box<dyn Error>> {
        let mut scheduler = Scheduler::default();
        assert!(scheduler.enqueue(request(1)?).is_some());
        assert!(scheduler.enqueue(request(2)?).is_some());
        assert!(scheduler.enqueue(request(3)?).is_none());
        scheduler.cancel_pending();
        assert!(scheduler.finish().is_none());
        assert_eq!(scheduler.active_native_compiles, 1);
        Ok(())
    }

    #[test]
    fn preview_scheduler_retains_only_the_latest_request() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let oxyst_compiler::CompileOutcome::Success(document) =
            compiler.compile("= One\n#pagebreak()\n= Two")
        else {
            return Err("fixture did not compile".into());
        };
        let picker = Picker::halfblocks();
        let pixels = 40 * u32::from(picker.font_size().width.max(1));
        let manifest: RenderManifest = oxyst_render::render_manifest(&document, pixels)?;
        let queued = |id| QueuedPreviewRequest {
            id,
            request: PreviewPageRequest {
                generation: 1,
                revision: 1,
                document: document.clone(),
                manifest: manifest.clone(),
                picker: picker.clone(),
                width: 40,
                pages: vec![1],
            },
        };
        let mut scheduler = PreviewScheduler::default();
        assert_eq!(
            scheduler.enqueue(queued(1)).map(|request| request.id),
            Some(1)
        );
        assert!(scheduler.enqueue(queued(2)).is_none());
        assert!(scheduler.enqueue(queued(3)).is_none());
        assert_eq!(scheduler.finish().map(|request| request.id), Some(3));
        assert!(scheduler.finish().is_none());
        Ok(())
    }

    fn request(generation: u64) -> Result<CompileRequest, Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        compiler.replace_source("= scheduler test");
        Ok(CompileRequest {
            generation,
            revision: generation,
            input: CompileInput::Snapshot(compiler.snapshot()),
        })
    }
}
