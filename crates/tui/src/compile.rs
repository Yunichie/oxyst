use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

use ratatui_image::{picker::Picker, sliced::SlicedProtocol};
use tokio::runtime::Handle;
use typst_tui_compiler::{
    CompileOutcome, CompileSnapshot, CompiledDocument, Compiler, Diagnostic, DocumentSync,
};
use typst_tui_document::TextEdit;

use crate::{components::Preview, event::Event};

const MAX_ACTIVE_COMPILES: usize = 2;

pub(crate) struct CompileResult {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) elapsed: Duration,
    pub(crate) outcome: CompileResultKind,
}

pub(crate) enum CompileResultKind {
    Success {
        pages: Vec<SlicedProtocol>,
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
    snapshot: CompileSnapshot,
    picker: Picker,
    width: u16,
}

#[derive(Default)]
struct Scheduler {
    active: usize,
    pending: Option<CompileRequest>,
}

impl Scheduler {
    fn enqueue(&mut self, request: CompileRequest) -> Option<CompileRequest> {
        if self.active < MAX_ACTIVE_COMPILES {
            self.active += 1;
            Some(request)
        } else {
            self.pending = Some(request);
            None
        }
    }

    fn cancel_pending(&mut self) {
        self.pending = None;
    }

    fn finish(&mut self) -> Option<CompileRequest> {
        self.active = self.active.saturating_sub(1);
        self.pending.take().inspect(|_| self.active += 1)
    }
}

#[derive(Clone)]
struct Runner {
    sender: Sender<Event>,
    runtime: Handle,
    generation: Arc<AtomicU64>,
    scheduler: Arc<Mutex<Scheduler>>,
}

pub(crate) struct CompileWorker {
    compiler: Mutex<Compiler>,
    runner: Runner,
}

impl CompileWorker {
    pub(crate) fn new(
        mut compiler: Compiler,
        source: &str,
        sender: Sender<Event>,
        runtime: Handle,
    ) -> Self {
        compiler.replace_source(source);
        Self {
            compiler: Mutex::new(compiler),
            runner: Runner {
                sender,
                runtime,
                generation: Arc::new(AtomicU64::new(0)),
                scheduler: Arc::new(Mutex::new(Scheduler::default())),
            },
        }
    }

    pub(crate) fn apply_edit(&self, edit: &TextEdit) -> Result<u64, String> {
        let generation = self.advance_and_cancel_pending()?;
        let mut compiler = self
            .compiler
            .lock()
            .map_err(|_| "compiler coordinator is unavailable".to_owned())?;
        compiler
            .apply_edit(edit.range(), edit.replacement())
            .map_err(|error| error.to_string())?;
        Ok(generation)
    }

    pub(crate) fn invalidate(&self) -> Result<u64, String> {
        self.advance_and_cancel_pending()
    }

    pub(crate) fn spawn(&self, revision: u64, picker: Picker, width: u16) -> u64 {
        let generation = match self.advance_and_cancel_pending() {
            Ok(generation) => generation,
            Err(error) => return self.report_preparation_error(revision, error),
        };
        let snapshot = match self.compiler.lock() {
            Ok(compiler) => compiler.snapshot(),
            Err(_) => {
                self.send_error(revision, generation, "compiler coordinator is unavailable");
                return generation;
            }
        };
        self.enqueue(CompileRequest {
            generation,
            revision,
            snapshot,
            picker,
            width,
        });
        generation
    }

    pub(crate) fn spawn_with_world(
        &self,
        revision: u64,
        source: &str,
        picker: Picker,
        width: u16,
        root: PathBuf,
        main: PathBuf,
    ) -> u64 {
        let generation = match self.advance_and_cancel_pending() {
            Ok(generation) => generation,
            Err(error) => return self.report_preparation_error(revision, error),
        };
        let mut compiler = match Compiler::new(root, main) {
            Ok(compiler) => compiler,
            Err(error) => {
                self.send_error(revision, generation, &error.to_string());
                return generation;
            }
        };
        compiler.replace_source(source);
        let snapshot = compiler.snapshot();
        match self.compiler.lock() {
            Ok(mut coordinator) => *coordinator = compiler,
            Err(_) => {
                self.send_error(revision, generation, "compiler coordinator is unavailable");
                return generation;
            }
        }
        self.enqueue(CompileRequest {
            generation,
            revision,
            snapshot,
            picker,
            width,
        });
        generation
    }

    fn advance_and_cancel_pending(&self) -> Result<u64, String> {
        let generation = advance(&self.runner.generation);
        let mut scheduler = self
            .runner
            .scheduler
            .lock()
            .map_err(|_| "compiler scheduler is unavailable".to_owned())?;
        scheduler.cancel_pending();
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
            .send(Event::CompileFinished(CompileResult {
                generation,
                revision,
                elapsed: Duration::ZERO,
                outcome: CompileResultKind::Error(error.to_owned()),
            }));
    }
}

fn start_request(runner: Runner, request: CompileRequest) {
    let runtime = runner.runtime.clone();
    drop(runtime.spawn_blocking(move || {
        let started = Instant::now();
        let result = compile(request, &runner.generation, started);
        if let Some(result) = result
            && is_current(&runner.generation, result.generation)
        {
            let _ = runner.sender.send(Event::CompileFinished(result));
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
    let compiled = match request.snapshot.compile() {
        CompileOutcome::Success(compiled) => compiled,
        CompileOutcome::Failure(diagnostics) => {
            return is_current(generations, request.generation).then_some(CompileResult {
                generation: request.generation,
                revision: request.revision,
                elapsed: started.elapsed(),
                outcome: CompileResultKind::Diagnostics(diagnostics),
            });
        }
    };
    if !is_current(generations, request.generation) {
        return None;
    }
    let sync = compiled.sync();
    let font_width = request.picker.font_size().width.max(1);
    let max_columns = (2_048 / font_width).max(1);
    let width = request.width.clamp(1, max_columns);
    let target_pixels = u32::from(width) * u32::from(font_width);
    let rendered = match typst_tui_render::render_cancellable(&compiled, target_pixels, || {
        !is_current(generations, request.generation)
    }) {
        Ok(rendered) => rendered,
        Err(typst_tui_render::Error::Cancelled) => return None,
        Err(error) => {
            return Some(CompileResult {
                generation: request.generation,
                revision: request.revision,
                elapsed: started.elapsed(),
                outcome: CompileResultKind::Error(error.to_string()),
            });
        }
    };
    let pages = match Preview::encode_pages_cancellable(&request.picker, rendered, width, || {
        !is_current(generations, request.generation)
    }) {
        Ok(Some(pages)) => pages,
        Ok(None) => return None,
        Err(error) => {
            return Some(CompileResult {
                generation: request.generation,
                revision: request.revision,
                elapsed: started.elapsed(),
                outcome: CompileResultKind::Error(error),
            });
        }
    };

    is_current(generations, request.generation).then_some(CompileResult {
        generation: request.generation,
        revision: request.revision,
        elapsed: started.elapsed(),
        outcome: CompileResultKind::Success {
            pages,
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

    use ratatui_image::picker::Picker;
    use typst_tui_compiler::Compiler;

    use super::{CompileRequest, Scheduler, advance, is_current};

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
    fn scheduler_starts_two_jobs_and_coalesces_to_the_latest_pending() -> Result<(), Box<dyn Error>>
    {
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

        let next = scheduler.finish().ok_or("latest job was not retained")?;
        assert_eq!(next.generation, 4);
        assert_eq!(scheduler.active, 2);
        Ok(())
    }

    fn request(generation: u64) -> Result<CompileRequest, Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        compiler.replace_source("= scheduler test");
        Ok(CompileRequest {
            generation,
            revision: generation,
            snapshot: compiler.snapshot(),
            picker: Picker::halfblocks(),
            width: 40,
        })
    }
}
