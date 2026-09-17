use std::path::PathBuf;

use oxyst_render::ExportFormat;

pub(crate) struct ExplorerScanResult {
    pub(crate) generation: u64,
    pub(crate) root: PathBuf,
    pub(crate) files: Vec<PathBuf>,
}

pub(crate) struct ExportResult {
    pub(crate) format: ExportFormat,
    pub(crate) path: PathBuf,
    pub(crate) result: Result<(), String>,
}
