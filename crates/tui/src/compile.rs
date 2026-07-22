use std::{
    sync::{Arc, Mutex, mpsc::Sender},
    time::{Duration, Instant},
};

use ratatui_image::{picker::Picker, sliced::SlicedProtocol};
use tokio::runtime::Handle;
use typst_tui_compiler::{CompileOutcome, Compiler, Diagnostic};

use crate::{components::Preview, event::Event};

pub(crate) struct CompileResult {
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
}

impl CompileWorker {
    pub(crate) fn new(compiler: Compiler, sender: Sender<Event>, runtime: Handle) -> Self {
        Self {
            compiler: Arc::new(Mutex::new(compiler)),
            sender,
            runtime,
        }
    }

    pub(crate) fn spawn(&self, revision: u64, source: String, picker: Picker, width: u16) {
        let compiler = Arc::clone(&self.compiler);
        let sender = self.sender.clone();
        drop(self.runtime.spawn_blocking(move || {
            let started = Instant::now();
            let outcome = compile(compiler, source, &picker, width);
            let result = CompileResult {
                revision,
                elapsed: started.elapsed(),
                outcome,
            };
            let _ = sender.send(Event::CompileFinished(result));
        }));
    }
}

fn compile(
    compiler: Arc<Mutex<Compiler>>,
    source: String,
    picker: &Picker,
    width: u16,
) -> CompileResultKind {
    let mut compiler = match compiler.lock() {
        Ok(compiler) => compiler,
        Err(_) => return CompileResultKind::Error("compiler worker is unavailable".to_owned()),
    };
    let compiled = match compiler.compile(&source) {
        CompileOutcome::Success(compiled) => compiled,
        CompileOutcome::Failure(diagnostics) => {
            return CompileResultKind::Diagnostics(diagnostics);
        }
    };

    let font_width = picker.font_size().width.max(1);
    let max_columns = (2_048 / font_width).max(1);
    let width = width.clamp(1, max_columns);
    let target_pixels = u32::from(width) * u32::from(font_width);
    let rendered = match typst_tui_render::render(&compiled, target_pixels) {
        Ok(rendered) => rendered,
        Err(error) => return CompileResultKind::Error(error.to_string()),
    };
    let pages = match Preview::encode_pages(picker, rendered, width) {
        Ok(pages) => pages,
        Err(error) => return CompileResultKind::Error(error),
    };

    CompileResultKind::Success {
        pages,
        diagnostics: compiled.warnings().to_vec(),
    }
}
