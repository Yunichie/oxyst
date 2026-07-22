use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

use ratatui_image::{picker::Picker, sliced::SlicedProtocol};
use tokio::runtime::Handle;
use typst_tui_compiler::{CompileOutcome, Compiler, Diagnostic};

use crate::{components::Preview, event::Event};

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
    },
    Diagnostics(Vec<Diagnostic>),
    Error(String),
}

pub(crate) struct CompileWorker {
    compiler: Arc<Mutex<Compiler>>,
    sender: Sender<Event>,
    runtime: Handle,
    generation: Arc<AtomicU64>,
}

impl CompileWorker {
    pub(crate) fn new(compiler: Compiler, sender: Sender<Event>, runtime: Handle) -> Self {
        Self {
            compiler: Arc::new(Mutex::new(compiler)),
            sender,
            runtime,
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn invalidate(&self) -> u64 {
        advance(&self.generation)
    }

    pub(crate) fn spawn(&self, revision: u64, source: String, picker: Picker, width: u16) -> u64 {
        let compiler = Arc::clone(&self.compiler);
        let sender = self.sender.clone();
        let generations = Arc::clone(&self.generation);
        let generation = advance(&generations);
        drop(self.runtime.spawn_blocking(move || {
            if !is_current(&generations, generation) {
                return;
            }
            let started = Instant::now();
            let Some(outcome) = compile(compiler, source, &picker, width, &generations, generation)
            else {
                return;
            };
            let result = CompileResult {
                generation,
                revision,
                elapsed: started.elapsed(),
                outcome,
            };
            if is_current(&generations, generation) {
                let _ = sender.send(Event::CompileFinished(result));
            }
        }));
        generation
    }
}

fn compile(
    compiler: Arc<Mutex<Compiler>>,
    source: String,
    picker: &Picker,
    width: u16,
    generations: &AtomicU64,
    generation: u64,
) -> Option<CompileResultKind> {
    if !is_current(generations, generation) {
        return None;
    }
    let mut compiler = match compiler.lock() {
        Ok(compiler) => compiler,
        Err(_) => {
            return Some(CompileResultKind::Error(
                "compiler worker is unavailable".to_owned(),
            ));
        }
    };
    if !is_current(generations, generation) {
        return None;
    }
    let compiled = match compiler.compile(&source) {
        CompileOutcome::Success(compiled) => compiled,
        CompileOutcome::Failure(diagnostics) => {
            return is_current(generations, generation)
                .then_some(CompileResultKind::Diagnostics(diagnostics));
        }
    };
    drop(compiler);
    if !is_current(generations, generation) {
        return None;
    }

    let font_width = picker.font_size().width.max(1);
    let max_columns = (2_048 / font_width).max(1);
    let width = width.clamp(1, max_columns);
    let target_pixels = u32::from(width) * u32::from(font_width);
    let rendered = match typst_tui_render::render(&compiled, target_pixels) {
        Ok(rendered) => rendered,
        Err(error) => return Some(CompileResultKind::Error(error.to_string())),
    };
    if !is_current(generations, generation) {
        return None;
    }
    let pages = match Preview::encode_pages(picker, rendered, width) {
        Ok(pages) => pages,
        Err(error) => return Some(CompileResultKind::Error(error)),
    };

    is_current(generations, generation).then_some(CompileResultKind::Success {
        pages,
        diagnostics: compiled.warnings().to_vec(),
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
    use std::sync::atomic::AtomicU64;

    use super::{advance, is_current};

    #[test]
    fn advancing_the_generation_invalidates_older_work() {
        let generations = AtomicU64::new(0);
        let first = advance(&generations);
        assert!(is_current(&generations, first));

        let second = advance(&generations);
        assert!(!is_current(&generations, first));
        assert!(is_current(&generations, second));
    }
}
