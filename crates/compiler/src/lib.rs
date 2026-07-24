#![forbid(unsafe_code)]

mod diagnostics;
mod sync;
mod world;

use std::{
    ops::Range,
    path::{Path, PathBuf},
};

pub use diagnostics::{Diagnostic, DiagnosticNote, DiagnosticNoteKind, Severity};
pub use sync::{DocumentSync, PagePosition};
use thiserror::Error;
use typst::diag::Warned;
use typst_layout::{Page, PagedDocument};
use unicode_segmentation::UnicodeSegmentation;

use crate::{diagnostics::convert_diagnostics, world::TypstWorld};

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to resolve project root {path}")]
    ProjectRoot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve main file {path}")]
    MainFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("main file {main} is outside project root {root}")]
    MainOutsideRoot { main: PathBuf, root: PathBuf },
    #[error("main file path {path} cannot be represented by Typst")]
    InvalidMainPath {
        path: PathBuf,
        #[source]
        source: typst::syntax::VirtualizeError,
    },
    #[error("source edit {start}..{end} is not on valid UTF-8 boundaries")]
    InvalidSourceEdit { start: usize, end: usize },
}

pub struct Compiler {
    world: TypstWorld,
}

impl Compiler {
    pub fn new(root: impl AsRef<Path>, main: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            world: TypstWorld::new(root.as_ref(), main.as_ref())?,
        })
    }

    #[must_use]
    pub fn compile(&mut self, text: &str) -> CompileOutcome {
        self.replace_source(text);
        self.snapshot().compile()
    }

    pub fn replace_source(&mut self, text: &str) {
        self.world.replace_source(text);
    }

    pub fn apply_edit(&mut self, range: Range<usize>, replacement: &str) -> Result<(), Error> {
        let text = self.world.source_text();
        if range.start > range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return Err(Error::InvalidSourceEdit {
                start: range.start,
                end: range.end,
            });
        }
        self.world.edit_source(range, replacement);
        Ok(())
    }

    pub fn invalidate_files(&mut self) {
        self.world.invalidate_files();
    }

    #[must_use]
    pub fn snapshot(&self) -> CompileSnapshot {
        CompileSnapshot {
            world: self.world.snapshot(),
        }
    }
}

pub struct CompileSnapshot {
    world: TypstWorld,
}

impl CompileSnapshot {
    #[must_use]
    pub fn word_count(&self) -> usize {
        self.world.source_text().unicode_words().count()
    }

    #[must_use]
    pub fn compile(self) -> CompileOutcome {
        let Warned { output, warnings } = typst::compile::<PagedDocument>(&self.world);
        let warnings = convert_diagnostics(&self.world, warnings);

        match output {
            Ok(document) => {
                let sync = DocumentSync::new(document.clone(), self.world.main_source());
                CompileOutcome::Success(CompiledDocument {
                    document,
                    warnings,
                    sync,
                })
            }
            Err(errors) => {
                let mut diagnostics = convert_diagnostics(&self.world, errors);
                diagnostics.extend(warnings);
                CompileOutcome::Failure(diagnostics)
            }
        }
    }
}

pub enum CompileOutcome {
    Success(CompiledDocument),
    Failure(Vec<Diagnostic>),
}

#[derive(Clone)]
pub struct CompiledDocument {
    document: PagedDocument,
    warnings: Vec<Diagnostic>,
    sync: DocumentSync,
}

impl CompiledDocument {
    #[must_use]
    pub fn pages(&self) -> &[Page] {
        self.document.pages()
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.document.pages().len()
    }

    #[must_use]
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    #[must_use]
    pub fn sync(&self) -> DocumentSync {
        self.sync.clone()
    }

    #[must_use]
    pub fn paged_document(&self) -> &PagedDocument {
        &self.document
    }
}
