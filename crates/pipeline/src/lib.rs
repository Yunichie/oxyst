#![forbid(unsafe_code)]

mod encoding;
mod worker;

use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError, channel},
    time::{Duration, Instant},
};

pub use encoding::{encode_document, encode_rendered_pages_cancellable};
use oxyst_compiler::{CompiledDocument, Compiler, Diagnostic, DocumentSync, Severity};
use oxyst_render::RenderManifest;
use ratatui::layout::Size;
use ratatui_image::{
    picker::{Picker, ProtocolType},
    sliced::SlicedProtocol,
};
use tokio::runtime::Handle;
use typst_syntax::Source;

use worker::{
    CompileResult, CompileResultKind, CompileWorker, PreviewPageRequest, PreviewPageResult,
    WorkerEvent, WorldRebuild,
};

const AUTO_COMPILE_DELAY: Duration = Duration::from_millis(150);
const COMPILE_STALLED_AFTER: Duration = Duration::from_secs(5);
const HALFBLOCK_PIXELS_PER_COLUMN: u32 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompileState {
    NotStarted,
    Compiling,
    Ready,
    Stale,
    Stalled,
    Failed(usize),
    Error,
}

pub enum PipelineUpdate {
    SourceRebuilt(Source),
    Compiled {
        diagnostics: Vec<Diagnostic>,
        manifest: RenderManifest,
        width: u16,
    },
    Failed(Vec<Diagnostic>),
    ManifestRescaled {
        manifest: RenderManifest,
        width: u16,
    },
    Pages(Vec<(usize, SlicedProtocol)>),
    Stalled,
    Error(String),
}

#[derive(Clone)]
struct WorldTarget {
    root: PathBuf,
    main: PathBuf,
}

pub struct PreviewPipeline {
    worker: CompileWorker,
    events: Receiver<WorkerEvent>,
    picker: Picker,
    debounce_deadline: Option<Instant>,
    generation: u64,
    world_target: Option<WorldTarget>,
    target_width: u16,
    manifest: Option<(u64, u64, RenderManifest, u16)>,
    page_request: Option<(u64, Vec<usize>)>,
    stale: bool,
    compiled_document: Option<(u64, CompiledDocument)>,
    document_sync: Option<(u64, DocumentSync)>,
    compile_started_at: Option<Instant>,
    state: CompileState,
    last_compile_time: Option<Duration>,
}

impl PreviewPipeline {
    pub fn new(compiler: Compiler, picker: Picker, runtime: Handle) -> Self {
        let (sender, events) = channel();
        Self {
            worker: CompileWorker::new(compiler, sender, runtime),
            events,
            picker,
            debounce_deadline: None,
            generation: 0,
            world_target: None,
            target_width: 0,
            manifest: None,
            page_request: None,
            stale: false,
            compiled_document: None,
            document_sync: None,
            compile_started_at: None,
            state: CompileState::NotStarted,
            last_compile_time: None,
        }
    }

    pub fn picker(&self) -> &Picker {
        &self.picker
    }

    pub fn state(&self) -> CompileState {
        self.state
    }

    pub fn last_compile_time(&self) -> Option<Duration> {
        self.last_compile_time
    }

    pub fn is_stale(&self) -> bool {
        self.stale
    }

    pub fn compiled_document(&self, revision: u64) -> Option<&CompiledDocument> {
        (!self.stale)
            .then_some(self.compiled_document.as_ref())
            .flatten()
            .filter(|(compiled_revision, _)| *compiled_revision == revision)
            .map(|(_, document)| document)
    }

    pub fn document_sync(&self, revision: u64) -> Option<&DocumentSync> {
        (!self.stale)
            .then_some(self.document_sync.as_ref())
            .flatten()
            .filter(|(sync_revision, _)| *sync_revision == revision)
            .map(|(_, sync)| sync)
    }

    pub fn document_changed(&mut self, now: Instant) -> Result<(), String> {
        match self.worker.invalidate() {
            Ok(generation) => {
                self.generation = generation;
                self.mark_stale(now, true);
                Ok(())
            }
            Err(error) => self.invalidation_failed(error),
        }
    }

    pub fn project_files_changed(&mut self, now: Instant) -> Result<(), String> {
        let invalidated = if self.world_target.is_some() {
            self.worker.invalidate()
        } else {
            self.worker.invalidate_files()
        };
        match invalidated {
            Ok(generation) => {
                self.generation = generation;
                self.mark_stale(now, true);
                Ok(())
            }
            Err(error) => self.invalidation_failed(error),
        }
    }

    pub fn reset_for_world(
        &mut self,
        root: PathBuf,
        main: PathBuf,
        now: Instant,
    ) -> Result<(), String> {
        self.world_target = Some(WorldTarget { root, main });
        self.manifest = None;
        self.page_request = None;
        self.compiled_document = None;
        self.document_sync = None;
        self.last_compile_time = None;
        match self.worker.invalidate() {
            Ok(generation) => {
                self.generation = generation;
                self.mark_stale(now, true);
                Ok(())
            }
            Err(error) => self.invalidation_failed(error),
        }
    }

    pub fn start_compile(&mut self, revision: u64, source: Source, now: Instant) {
        self.debounce_deadline = None;
        self.generation = if let Some(target) = &self.world_target {
            self.worker.spawn_with_world(
                revision,
                WorldRebuild {
                    root: target.root.clone(),
                    main: target.main.clone(),
                    source: source.text().to_owned(),
                },
            )
        } else {
            self.worker.spawn(revision, source)
        };
        self.page_request = None;
        self.compile_started_at = Some(now);
        self.state = CompileState::Compiling;
    }

    pub fn start_world_compile(
        &mut self,
        revision: u64,
        source: Source,
        root: PathBuf,
        main: PathBuf,
        now: Instant,
    ) {
        self.world_target = Some(WorldTarget { root, main });
        self.start_compile(revision, source, now);
    }

    pub fn tick(&mut self, now: Instant, revision: u64, source: Source) -> Option<PipelineUpdate> {
        if self
            .debounce_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.start_compile(revision, source, now);
        }
        if self.state == CompileState::Compiling
            && self
                .compile_started_at
                .is_some_and(|started| now.duration_since(started) >= COMPILE_STALLED_AFTER)
        {
            self.state = CompileState::Stalled;
            return Some(PipelineUpdate::Stalled);
        }
        None
    }

    pub fn poll(&mut self, revision: u64, text: &str) -> Vec<PipelineUpdate> {
        let mut updates = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(WorkerEvent::CompileFinished(result)) => {
                    updates.extend(self.finish_compile(result, revision, text));
                }
                Ok(WorkerEvent::PreviewPagesFinished(result)) => {
                    if let Some(update) = self.finish_preview_pages(result, revision) {
                        updates.push(update);
                    }
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        updates
    }

    pub fn set_preview_width(
        &mut self,
        target_width: u16,
        revision: u64,
    ) -> Option<PipelineUpdate> {
        if target_width == self.target_width {
            return None;
        }
        self.target_width = target_width;
        if self.stale || self.state != CompileState::Ready {
            return None;
        }
        let document = self.compiled_document(revision)?;
        let (width, target_pixels) = preview_dimensions(&self.picker, self.target_width);
        let manifest = match oxyst_render::render_manifest(document, target_pixels) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.state = CompileState::Error;
                return Some(PipelineUpdate::Error(format!("Preview failed: {error}")));
            }
        };
        self.manifest = Some((self.generation, revision, manifest.clone(), width));
        self.page_request = None;
        Some(PipelineUpdate::ManifestRescaled { manifest, width })
    }

    pub fn request_pages(&mut self, revision: u64, pages: Vec<usize>) {
        if self.stale {
            return;
        }
        if pages.is_empty() {
            self.page_request = None;
            return;
        }
        if self
            .page_request
            .as_ref()
            .is_some_and(|(_, requested)| *requested == pages)
        {
            return;
        }
        let Some((document_revision, document)) = self
            .compiled_document
            .as_ref()
            .filter(|(compiled_revision, _)| *compiled_revision == revision)
        else {
            return;
        };
        let Some((_, manifest_revision, manifest, width)) =
            self.manifest
                .as_ref()
                .filter(|(generation, manifest_revision, _, _)| {
                    *generation == self.generation && manifest_revision == document_revision
                })
        else {
            return;
        };
        let request = self.worker.spawn_preview_pages(PreviewPageRequest {
            generation: self.generation,
            revision: *manifest_revision,
            document: document.clone(),
            manifest: manifest.clone(),
            picker: self.picker.clone(),
            width: *width,
            pages: pages.clone(),
        });
        self.page_request = Some((request, pages));
    }

    fn mark_stale(&mut self, now: Instant, preview_stale: bool) {
        self.debounce_deadline = Some(now + AUTO_COMPILE_DELAY);
        self.compile_started_at = None;
        self.state = CompileState::Stale;
        self.stale |= preview_stale;
    }

    fn invalidation_failed(&mut self, error: String) -> Result<(), String> {
        self.debounce_deadline = None;
        self.compile_started_at = None;
        self.state = CompileState::Error;
        self.stale = true;
        Err(error)
    }

    fn finish_compile(
        &mut self,
        mut result: CompileResult,
        revision: u64,
        text: &str,
    ) -> Vec<PipelineUpdate> {
        if result.generation != self.generation {
            return Vec::new();
        }
        self.compile_started_at = None;
        if result.revision != revision {
            self.state = CompileState::Stale;
            return Vec::new();
        }
        let mut updates = Vec::new();
        if let Some(compiler) = result.rebuilt_compiler.take() {
            let source = compiler.main_source();
            if source.text() != text {
                self.state = CompileState::Error;
                return vec![PipelineUpdate::Error(
                    "rebuilt compiler source does not match the editor".to_owned(),
                )];
            }
            if let Err(error) = self.worker.install(compiler) {
                self.state = CompileState::Error;
                return vec![PipelineUpdate::Error(error)];
            }
            self.world_target = None;
            updates.push(PipelineUpdate::SourceRebuilt(source));
        } else if result.rebuild_attempted {
            debug_assert!(self.world_target.is_some());
        }
        self.last_compile_time = Some(result.elapsed);
        match result.outcome {
            CompileResultKind::Success {
                diagnostics,
                sync,
                document,
            } => {
                let document = *document;
                let (width, target_pixels) = preview_dimensions(&self.picker, self.target_width);
                let manifest = match oxyst_render::render_manifest(&document, target_pixels) {
                    Ok(manifest) => manifest,
                    Err(error) => {
                        self.state = CompileState::Error;
                        updates.push(PipelineUpdate::Error(format!("Preview failed: {error}")));
                        return updates;
                    }
                };
                self.document_sync = Some((result.revision, sync));
                self.compiled_document = Some((result.revision, document));
                self.manifest = Some((result.generation, result.revision, manifest.clone(), width));
                self.page_request = None;
                self.state = CompileState::Ready;
                self.stale = false;
                updates.push(PipelineUpdate::Compiled {
                    diagnostics,
                    manifest,
                    width,
                });
            }
            CompileResultKind::Diagnostics(diagnostics) => {
                let errors = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.severity == Severity::Error)
                    .count();
                self.state = CompileState::Failed(errors);
                updates.push(PipelineUpdate::Failed(diagnostics));
            }
            CompileResultKind::Error(error) => {
                self.state = CompileState::Error;
                updates.push(PipelineUpdate::Error(format!("Preview failed: {error}")));
            }
        }
        updates
    }

    fn finish_preview_pages(
        &mut self,
        result: PreviewPageResult,
        revision: u64,
    ) -> Option<PipelineUpdate> {
        if result.generation != self.generation
            || result.revision != revision
            || self
                .page_request
                .as_ref()
                .is_none_or(|(request, _)| *request != result.request)
            || self
                .manifest
                .as_ref()
                .is_none_or(|(_, _, _, width)| *width != result.width)
        {
            return None;
        }
        match result.outcome {
            Ok(pages) => {
                self.page_request = None;
                Some(PipelineUpdate::Pages(pages))
            }
            Err(error) => {
                self.page_request = Some((result.request, result.requested));
                Some(PipelineUpdate::Error(format!(
                    "Preview page failed: {error}"
                )))
            }
        }
    }
}

pub fn page_sizes(picker: &Picker, manifest: &RenderManifest, width: u16) -> Vec<Size> {
    let width = width.max(1);
    let font = picker.font_size();
    manifest
        .pages()
        .iter()
        .map(|page| {
            let pixel_width = u64::from(page.width().max(1));
            let target_pixel_width = u64::from(width) * u64::from(font.width.max(1));
            let scaled_height = u64::from(page.height()) * target_pixel_width / pixel_width;
            let rows = scaled_height
                .div_ceil(u64::from(font.height.max(1)))
                .clamp(1, u64::from(u16::MAX)) as u16;
            Size::new(width, rows)
        })
        .collect()
}

fn preview_dimensions(picker: &Picker, width: u16) -> (u16, u32) {
    let font_width = picker.font_size().width.max(1);
    let max_columns = (2_048 / font_width).max(1);
    let width = width.clamp(1, max_columns);
    let pixels_per_column = match picker.protocol_type() {
        ProtocolType::Halfblocks => HALFBLOCK_PIXELS_PER_COLUMN,
        ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2 => u32::from(font_width),
    };
    (width, u32::from(width) * pixels_per_column)
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        path::PathBuf,
        time::{Duration, Instant},
    };

    use oxyst_compiler::Compiler;
    use ratatui_image::picker::{Picker, ProtocolType};
    use typst_syntax::Source;

    use super::{CompileState, PipelineUpdate, PreviewPipeline, preview_dimensions};

    #[test]
    fn compile_debounce_restarts_after_each_change() -> Result<(), Box<dyn Error>> {
        let (mut pipeline, runtime, source) = pipeline("= debounce")?;
        let start = Instant::now();
        pipeline.document_changed(start)?;
        pipeline.document_changed(start + Duration::from_millis(100))?;

        assert!(
            pipeline
                .tick(start + Duration::from_millis(249), 0, source.clone())
                .is_none()
        );
        assert_eq!(pipeline.state(), CompileState::Stale);
        assert!(
            pipeline
                .tick(start + Duration::from_millis(250), 0, source)
                .is_none()
        );
        assert_eq!(pipeline.state(), CompileState::Compiling);
        runtime.shutdown_timeout(Duration::from_millis(100));
        Ok(())
    }

    #[test]
    fn resource_changes_and_resize_preserve_pipeline_invariants() -> Result<(), Box<dyn Error>> {
        let (mut pipeline, runtime, source) = pipeline("= resources")?;
        let now = Instant::now();
        pipeline.project_files_changed(now)?;
        let generation = pipeline.generation;
        let deadline = pipeline.debounce_deadline;

        assert!(pipeline.is_stale());
        assert!(pipeline.set_preview_width(60, 0).is_none());
        assert_eq!(pipeline.generation, generation);
        assert_eq!(pipeline.debounce_deadline, deadline);

        pipeline.debounce_deadline = None;
        pipeline.state = CompileState::Compiling;
        pipeline.compile_started_at = Some(now - Duration::from_secs(5));
        assert!(matches!(
            pipeline.tick(now, 0, source),
            Some(PipelineUpdate::Stalled)
        ));
        assert_eq!(pipeline.state(), CompileState::Stalled);
        runtime.shutdown_timeout(Duration::from_millis(100));
        Ok(())
    }

    #[test]
    fn preview_dimensions_match_terminal_protocols() {
        let mut picker = Picker::halfblocks();
        assert_eq!(preview_dimensions(&picker, 80), (80, 320));
        assert_eq!(preview_dimensions(&picker, u16::MAX), (204, 816));

        for protocol in [
            ProtocolType::Sixel,
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
        ] {
            picker.set_protocol_type(protocol);
            assert_eq!(preview_dimensions(&picker, 80), (80, 800));
        }
    }

    #[test]
    fn world_rebuild_returns_the_rebuilt_source() -> Result<(), Box<dyn Error>> {
        let root = fixture_root();
        let compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
        let mut source = compiler.main_source();
        source.replace("= rebuilt world");
        let mut pipeline =
            PreviewPipeline::new(compiler, Picker::halfblocks(), runtime.handle().clone());
        pipeline.start_world_compile(
            3,
            source,
            root.clone(),
            root.join("simple.typ"),
            Instant::now(),
        );

        let updates = wait_for(&mut pipeline, 3, "= rebuilt world", |updates| {
            updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::SourceRebuilt(_)))
        })?;
        assert!(
            updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::Compiled { .. }))
        );
        runtime.shutdown_timeout(Duration::from_millis(100));
        Ok(())
    }

    #[test]
    fn preview_renders_only_requested_pages() -> Result<(), Box<dyn Error>> {
        let (mut pipeline, runtime, mut source) = pipeline("= One\n#pagebreak()\n= Two")?;
        source.replace("= One\n#pagebreak()\n= Two");
        pipeline.set_preview_width(40, 7);
        pipeline.start_compile(7, source, Instant::now());
        wait_for(&mut pipeline, 7, "= One\n#pagebreak()\n= Two", |updates| {
            updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::Compiled { .. }))
        })?;
        pipeline.request_pages(7, vec![1]);
        let updates = wait_for(&mut pipeline, 7, "= One\n#pagebreak()\n= Two", |updates| {
            updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::Pages(_)))
        })?;
        let pages = updates
            .into_iter()
            .find_map(|update| match update {
                PipelineUpdate::Pages(pages) => Some(pages),
                _ => None,
            })
            .ok_or("preview pages were not returned")?;
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].0, 1);
        runtime.shutdown_timeout(Duration::from_millis(100));
        Ok(())
    }

    #[test]
    fn compile_completion_is_validated_and_exposes_current_output() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
        let mut source = compiler.main_source();
        source.replace("= pipeline");
        let mut pipeline =
            PreviewPipeline::new(compiler, Picker::halfblocks(), runtime.handle().clone());
        pipeline.set_preview_width(40, 0);
        pipeline.start_compile(7, source, std::time::Instant::now());

        let mut compiled = false;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            let updates = pipeline.poll(7, "= pipeline");
            if updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::Compiled { .. }))
            {
                compiled = true;
                break;
            }
        }
        assert!(compiled);
        assert_eq!(pipeline.state(), CompileState::Ready);
        assert!(pipeline.compiled_document(7).is_some());
        assert!(pipeline.compiled_document(8).is_none());
        runtime.shutdown_timeout(Duration::from_millis(100));
        Ok(())
    }

    #[test]
    fn failed_compile_keeps_only_non_stale_previous_output() -> Result<(), Box<dyn Error>> {
        let (mut pipeline, runtime, source) = pipeline("= valid")?;
        pipeline.start_compile(1, source.clone(), Instant::now());
        wait_for(&mut pipeline, 1, "= valid", |updates| {
            updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::Compiled { .. }))
        })?;

        let mut invalid = source;
        invalid.replace("#unknown-function(");
        pipeline.start_compile(1, invalid.clone(), Instant::now());
        wait_for(&mut pipeline, 1, "#unknown-function(", |updates| {
            updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::Failed(_)))
        })?;
        assert!(pipeline.compiled_document(1).is_some());

        pipeline.document_changed(Instant::now())?;
        pipeline.start_compile(2, invalid, Instant::now());
        wait_for(&mut pipeline, 2, "#unknown-function(", |updates| {
            updates
                .iter()
                .any(|update| matches!(update, PipelineUpdate::Failed(_)))
        })?;
        assert!(pipeline.compiled_document(2).is_none());
        runtime.shutdown_timeout(Duration::from_millis(100));
        Ok(())
    }

    fn pipeline(
        text: &str,
    ) -> Result<(PreviewPipeline, tokio::runtime::Runtime, Source), Box<dyn Error>> {
        let root = fixture_root();
        let compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
        let mut source = compiler.main_source();
        source.replace(text);
        Ok((
            PreviewPipeline::new(compiler, Picker::halfblocks(), runtime.handle().clone()),
            runtime,
            source,
        ))
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    fn wait_for(
        pipeline: &mut PreviewPipeline,
        revision: u64,
        text: &str,
        done: impl Fn(&[PipelineUpdate]) -> bool,
    ) -> Result<Vec<PipelineUpdate>, Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut all = Vec::new();
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            let updates = pipeline.poll(revision, text);
            let finished = done(&updates);
            all.extend(updates);
            if finished {
                return Ok(all);
            }
        }
        Err("pipeline did not finish before the deadline".into())
    }
}
