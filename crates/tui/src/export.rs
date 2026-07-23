use std::{path::PathBuf, sync::mpsc::Sender};

use oxyst_compiler::CompiledDocument;
use oxyst_render::ExportFormat;
use tokio::runtime::Handle;

use crate::event::Event;

pub(crate) struct ExportResult {
    pub(crate) format: ExportFormat,
    pub(crate) path: PathBuf,
    pub(crate) result: Result<(), String>,
}

pub(crate) struct ExportWorker {
    sender: Sender<Event>,
    runtime: Handle,
}

impl ExportWorker {
    pub(crate) fn new(sender: Sender<Event>, runtime: Handle) -> Self {
        Self { sender, runtime }
    }

    pub(crate) fn spawn(&self, document: CompiledDocument, format: ExportFormat, path: PathBuf) {
        let sender = self.sender.clone();
        drop(self.runtime.spawn_blocking(move || {
            let result =
                oxyst_render::export(&document, format, &path).map_err(|error| error.to_string());
            let _ = sender.send(Event::ExportFinished(ExportResult {
                format,
                path,
                result,
            }));
        }));
    }
}
