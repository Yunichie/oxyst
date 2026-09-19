use std::{path::PathBuf, sync::mpsc::Sender};

use oxyst_compiler::CompiledDocument;
use oxyst_render::ExportFormat;
use tokio::runtime::Handle;

use crate::{documents::DocumentId, event::Event, message::ExportResult};

pub(crate) struct ExportWorker {
    sender: Sender<Event>,
    runtime: Handle,
}

impl ExportWorker {
    pub(crate) fn new(sender: Sender<Event>, runtime: Handle) -> Self {
        Self { sender, runtime }
    }

    pub(crate) fn spawn(
        &self,
        source: DocumentId,
        document: CompiledDocument,
        format: ExportFormat,
        path: PathBuf,
    ) {
        let sender = self.sender.clone();
        drop(self.runtime.spawn_blocking(move || {
            let result =
                oxyst_render::export(&document, format, &path).map_err(|error| error.to_string());
            let _ = sender.send(Event::ExportFinished(ExportResult {
                document: source,
                format,
                path,
                result,
            }));
        }));
    }
}
