#![forbid(unsafe_code)]

mod diagnostics;
mod world;

use std::path::{Path, PathBuf};

pub use diagnostics::{Diagnostic, Severity};
use thiserror::Error;
use typst::diag::Warned;
use typst_layout::{Page, PagedDocument};

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
        self.world.reset(text);
        let Warned { output, warnings } = typst::compile::<PagedDocument>(&self.world);
        let warnings = convert_diagnostics(&self.world, warnings);

        match output {
            Ok(document) => CompileOutcome::Success(CompiledDocument { document, warnings }),
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

pub struct CompiledDocument {
    document: PagedDocument,
    warnings: Vec<Diagnostic>,
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
}
