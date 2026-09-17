use std::{
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

use oxyst_compiler::Compiler;
use oxyst_config::{CommandId, Config};
use oxyst_document::Motion;
use oxyst_pipeline::{CompileState, PipelineUpdate, PreviewPipeline};
use oxyst_render::ExportFormat;
use oxyst_theme::{Color, ColorDepth, Theme, ThemeName};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Block, Paragraph},
};
use ratatui_image::picker::Picker;
use tokio::runtime::Handle;

use crate::{
    action::{Action, Pane},
    clipboard::Clipboard,
    components::{
        Component, ConfirmIntent, Diagnostics, Editor, FileExplorer, Header, HeaderState,
        OverlayHost, OverlaySubmission, Preview, Prompt, PromptKind, SearchMode, StatusBar,
        StatusBarState, Welcome, WelcomeChoice, format_diagnostic,
    },
    event::{self, Event},
    explorer::ExplorerWorker,
    export::ExportWorker,
    input::{self, InputMode, Keymap},
    message::{ExplorerScanResult, ExportResult},
    recent::RecentFiles,
    style::{base, color},
    watcher::ProjectWatcher,
    workspace::Workspace,
};

const NARROW_WIDTH: u16 = 80;
const CURSOR_SYNC_DELAY: Duration = Duration::from_millis(50);

fn overlay_transition_requires_clear(action: &Action) -> bool {
    matches!(
        action,
        Action::OverlaySubmit
            | Action::Command(
                CommandId::CloseOverlay
                    | CommandId::Newline
                    | CommandId::Confirm
                    | CommandId::CancelConfirmation
            )
    )
}

pub(crate) struct App {
    editor: Editor,
    header: Header,
    status_bar: StatusBar,
    clipboard: Clipboard,
    preview: Preview,
    explorer: FileExplorer,
    focus: Pane,
    fullscreen: bool,
    workspace: Workspace,
    pipeline: PreviewPipeline,
    explorer_worker: ExplorerWorker,
    explorer_scan_generation: u64,
    export_worker: ExportWorker,
    watcher: ProjectWatcher,
    internal_events: Receiver<Event>,
    cursor_sync_deadline: Option<Instant>,
    diagnostics: Diagnostics,
    keymap: Keymap,
    color_depth: ColorDepth,
    theme: Theme,
    welcome: Option<Welcome>,
    overlay: OverlayHost,
    quit_confirmation: bool,
    should_quit: bool,
    status: Option<String>,
    recent: RecentFiles,
}

pub(crate) struct AppInit<'a> {
    pub(crate) path: Option<PathBuf>,
    pub(crate) root: PathBuf,
    pub(crate) root_is_explicit: bool,
    pub(crate) text: &'a str,
    pub(crate) compiler: Compiler,
    pub(crate) picker: Picker,
    pub(crate) runtime: Handle,
    pub(crate) config: Config,
    pub(crate) recent: RecentFiles,
    pub(crate) startup_status: Option<String>,
}

impl App {
    pub(crate) fn new(init: AppInit<'_>) -> Result<Self, String> {
        let AppInit {
            path,
            root,
            root_is_explicit,
            text,
            compiler,
            picker,
            runtime,
            config,
            recent,
            startup_status,
        } = init;
        let color_depth = ColorDepth::detect();
        let theme = Theme::named(&config.theme, color_depth).map_err(|error| error.to_string())?;
        let keymap = Keymap::new(&config)?;
        let overlay = OverlayHost::new(keymap.clone(), theme);
        let header = Header::new(theme, keymap.display(CommandId::Help));
        let status_bar = StatusBar::new(
            theme,
            keymap.display(CommandId::Confirm),
            keymap.display(CommandId::CancelConfirmation),
        );
        let welcome = path.is_none().then(|| {
            let mut welcome = Welcome::new(
                theme,
                keymap.display(CommandId::Newline),
                keymap.display(CommandId::Help),
            );
            welcome.set_recent_count(recent.entries().len());
            welcome
        });
        let (sender, internal_events) = channel();
        let watcher = ProjectWatcher::new(&root, path.as_deref(), sender.clone())?;
        let explorer_worker = ExplorerWorker::new(sender.clone(), runtime.clone());
        let mut source = compiler.main_source();
        source.replace(text);

        Ok(Self {
            editor: Editor::from_source(source, theme, config.soft_wrap),
            header,
            status_bar,
            clipboard: Clipboard::new(),
            preview: Preview::new(theme),
            explorer: FileExplorer::new(root.clone(), theme),
            focus: Pane::Editor,
            fullscreen: false,
            workspace: Workspace::new(path, root, root_is_explicit),
            pipeline: PreviewPipeline::new(compiler, picker, runtime.clone()),
            explorer_worker,
            explorer_scan_generation: 0,
            export_worker: ExportWorker::new(sender, runtime),
            watcher,
            internal_events,
            cursor_sync_deadline: None,
            diagnostics: Diagnostics::new(theme),
            keymap,
            color_depth,
            theme,
            welcome,
            overlay,
            quit_confirmation: false,
            should_quit: false,
            status: startup_status,
            recent,
        })
    }

    pub(crate) fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let mut initial_compile_requested = self.welcome.is_some();
        while !self.should_quit {
            self.poll_pipeline();
            terminal.draw(|frame| self.draw(frame))?;
            self.observe_preview_width();
            self.request_preview_pages();
            if !initial_compile_requested {
                self.start_compile();
                initial_compile_requested = true;
            }
            let event = event::read(&self.internal_events)?;
            match event {
                Event::ExplorerScanFinished(result) => {
                    self.finish_explorer_scan(result);
                    continue;
                }
                Event::ExportFinished(result) => {
                    self.finish_export(result);
                    continue;
                }
                Event::ProjectFilesChanged => {
                    self.project_files_changed();
                    continue;
                }
                Event::FileWatchFailed(error) => {
                    self.status = Some(format!("File watch failed: {error}"));
                    continue;
                }
                Event::Tick => {
                    self.tick();
                    continue;
                }
                Event::Resize => continue,
                event => {
                    let mode = self.input_mode();
                    let accepts_text = mode != InputMode::Normal || self.focus == Pane::Editor;
                    if let Some(action) = input::resolve(event, mode, &self.keymap, accepts_text) {
                        let clear_terminal =
                            self.overlay.is_open() && overlay_transition_requires_clear(&action);
                        self.update(action);
                        if clear_terminal {
                            terminal.clear()?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn input_mode(&self) -> InputMode {
        if self.quit_confirmation {
            return InputMode::QuitConfirmation;
        }
        if let Some(mode) = self.overlay.input_mode() {
            mode
        } else if self.welcome.is_some() {
            InputMode::Welcome
        } else {
            InputMode::Normal
        }
    }

    fn update(&mut self, action: Action) {
        match action {
            Action::Command(command) => self.execute_command(command),
            Action::OverlayInput(_)
            | Action::OverlayInputText(_)
            | Action::OverlayBackspace
            | Action::OverlayMove(_)
            | Action::OverlaySubmit => self.update_overlay(action),
            Action::Copy => self.copy_selection(),
            Action::Cut if self.focus == Pane::Editor => self.cut_selection(),
            Action::PasteClipboard if self.focus == Pane::Editor => self.paste_clipboard(),
            Action::ZoomPreview(direction) if self.preview.zoom(direction) => {
                self.focus = Pane::Preview;
                self.observe_preview_width();
            }
            Action::MouseDown { column, row } => self.mouse_down(column, row),
            Action::MouseDrag { column, row } => self.mouse_drag(column, row),
            Action::ScrollAt { column, row, lines } => self.scroll_at(column, row, lines),
            Action::Move(Motion::Up) if self.focus == Pane::Explorer => self.explorer.select(-1),
            Action::Move(Motion::Down) if self.focus == Pane::Explorer => self.explorer.select(1),
            Action::Insert('\n') if self.focus == Pane::Explorer => {
                if let Some(path) = self.explorer.selected_path() {
                    self.request_open(path);
                }
            }
            Action::Move(Motion::Up) if self.focus == Pane::Preview => {
                self.preview.scroll_lines(-1);
            }
            Action::Move(Motion::Down) if self.focus == Pane::Preview => {
                self.preview.scroll_lines(1);
            }
            Action::RequestQuit if self.editor.is_dirty() => {
                self.quit_confirmation = true;
                self.overlay.close();
                self.status = None;
            }
            Action::RequestQuit | Action::Quit => self.should_quit = true,
            Action::CancelQuit => self.quit_confirmation = false,
            action if self.focus == Pane::Editor => self.update_editor(action),
            _ => {}
        }
    }

    fn update_overlay(&mut self, action: Action) {
        if matches!(action, Action::OverlaySubmit) {
            self.submit_overlay();
            return;
        }
        if let Some(welcome) = &mut self.welcome
            && !self.overlay.is_open()
        {
            welcome.update(action);
            return;
        }
        let query_changed = self.overlay.search_query_is_active()
            && matches!(
                action,
                Action::OverlayInput(_) | Action::OverlayInputText(_) | Action::OverlayBackspace
            );
        let query_origin = query_changed
            .then(|| self.editor.selection_byte_range().map(|range| range.start))
            .flatten();
        self.overlay.update(action);
        if query_changed {
            if let Some(origin) = query_origin {
                let _ = self.editor.set_cursor_byte_index(origin);
                self.editor.reset_preferred_visual_column();
            }
            self.search_next(false);
        }
    }

    fn submit_overlay(&mut self) {
        if let Some(welcome) = &self.welcome
            && !self.overlay.is_open()
        {
            match welcome.selected() {
                WelcomeChoice::NewDocument => self.start_new_document(),
                WelcomeChoice::OpenFile => self.open_file_prompt(),
                WelcomeChoice::OpenRecent => {
                    if self.recent.entries().is_empty() {
                        self.status = Some("No recent files".to_owned());
                    } else {
                        self.overlay.open_recent(self.recent.entries().to_vec());
                    }
                }
            }
            return;
        }

        match self.overlay.submit() {
            Some(OverlaySubmission::Command(command)) => self.execute_command(command),
            Some(OverlaySubmission::Prompt(prompt)) => self.submit_prompt(prompt),
            Some(OverlaySubmission::Confirm(intent)) => self.confirm(intent),
            Some(OverlaySubmission::Recent(path)) => self.request_open(path),
            None => {}
        }
    }

    fn execute_command(&mut self, command: CommandId) {
        match command {
            CommandId::OpenFile | CommandId::WelcomeOpen => self.open_file_prompt(),
            CommandId::ExportPdf | CommandId::ExportPng | CommandId::ExportSvg => {
                let format = match command {
                    CommandId::ExportPdf => ExportFormat::Pdf,
                    CommandId::ExportPng => ExportFormat::Png,
                    CommandId::ExportSvg => ExportFormat::Svg,
                    _ => return,
                };
                let path = self.default_export_path(format);
                self.overlay
                    .open_prompt(PromptKind::Export(format), path.to_string_lossy());
            }
            CommandId::GoToLine => self.overlay.open_prompt(PromptKind::GoToLine, ""),
            CommandId::GoToPage => self.overlay.open_prompt(PromptKind::GoToPage, ""),
            CommandId::ToggleDiagnostics => self.diagnostics.toggle(),
            CommandId::ToggleFileExplorer => self.toggle_explorer(),
            CommandId::UseDarkTheme => self.set_theme(ThemeName::Dark),
            CommandId::UseLightTheme => self.set_theme(ThemeName::Light),
            CommandId::ReloadFonts => {
                self.start_compile_with_world();
                self.status = Some("Reloading fonts...".to_owned());
            }
            CommandId::Quit => self.update(Action::RequestQuit),
            CommandId::Save => self.save(),
            CommandId::Recompile => self.start_compile(),
            CommandId::Undo => self.update(Action::Undo),
            CommandId::Redo => self.update(Action::Redo),
            CommandId::MoveLeft => self.update(Action::Move(Motion::Left)),
            CommandId::MoveRight => self.update(Action::Move(Motion::Right)),
            CommandId::MoveUp => self.execute_direction(Motion::Up, -1),
            CommandId::MoveDown => self.execute_direction(Motion::Down, 1),
            CommandId::SelectLeft => self.update(Action::Select(Motion::Left)),
            CommandId::SelectRight => self.update(Action::Select(Motion::Right)),
            CommandId::SelectUp => self.update(Action::Select(Motion::Up)),
            CommandId::SelectDown => self.update(Action::Select(Motion::Down)),
            CommandId::WordLeft => self.update(Action::Move(Motion::WordLeft)),
            CommandId::WordRight => self.update(Action::Move(Motion::WordRight)),
            CommandId::SelectWordLeft => self.update(Action::Select(Motion::WordLeft)),
            CommandId::SelectWordRight => self.update(Action::Select(Motion::WordRight)),
            CommandId::LineStart => self.update(Action::Move(Motion::LineStart)),
            CommandId::LineEnd => self.update(Action::Move(Motion::LineEnd)),
            CommandId::SelectLineStart => self.update(Action::Select(Motion::LineStart)),
            CommandId::SelectLineEnd => self.update(Action::Select(Motion::LineEnd)),
            CommandId::DocumentStart => self.update(Action::Move(Motion::DocumentStart)),
            CommandId::DocumentEnd => self.update(Action::Move(Motion::DocumentEnd)),
            CommandId::SelectDocumentStart => self.update(Action::Select(Motion::DocumentStart)),
            CommandId::SelectDocumentEnd => self.update(Action::Select(Motion::DocumentEnd)),
            CommandId::SelectAll => self.update(Action::SelectAll),
            CommandId::Copy => self.update(Action::Copy),
            CommandId::Cut => self.update(Action::Cut),
            CommandId::Paste => self.update(Action::PasteClipboard),
            CommandId::Find => self.open_search(SearchMode::Find),
            CommandId::FindReplace => self.open_search(SearchMode::Replace),
            CommandId::FindNext => self.search_next(false),
            CommandId::FindPrevious => self.search_next(true),
            CommandId::SearchToggleField => self.overlay.toggle_search_field(),
            CommandId::ReplaceCurrent => self.replace_current(),
            CommandId::Backspace => {
                if matches!(
                    self.input_mode(),
                    InputMode::Overlay | InputMode::Search | InputMode::Welcome
                ) {
                    self.update(Action::OverlayBackspace);
                } else {
                    self.update(Action::Backspace);
                }
            }
            CommandId::Delete => self.update(Action::Delete),
            CommandId::Newline => {
                if matches!(self.input_mode(), InputMode::Overlay | InputMode::Welcome) {
                    self.update(Action::OverlaySubmit);
                } else {
                    self.update(Action::Insert('\n'));
                }
            }
            CommandId::SwitchFocus => self.switch_focus(),
            CommandId::Fullscreen => self.fullscreen = !self.fullscreen,
            CommandId::ZoomIn => self.update(Action::ZoomPreview(1)),
            CommandId::ZoomOut => self.update(Action::ZoomPreview(-1)),
            CommandId::PreviewPageUp => self.preview.scroll_pages(-1),
            CommandId::PreviewPageDown => self.preview.scroll_pages(1),
            CommandId::CommandPalette => self.overlay.open_palette(),
            CommandId::NextDiagnostic => self.navigate_diagnostic(1),
            CommandId::PreviousDiagnostic => self.navigate_diagnostic(-1),
            CommandId::Help => self.overlay.open_help(),
            CommandId::CloseOverlay => self.overlay.close(),
            CommandId::Confirm => {
                if self.input_mode() == InputMode::QuitConfirmation {
                    self.update(Action::Quit);
                } else {
                    self.update(Action::OverlaySubmit);
                }
            }
            CommandId::CancelConfirmation => {
                if self.input_mode() == InputMode::QuitConfirmation {
                    self.update(Action::CancelQuit);
                } else {
                    self.overlay.close();
                }
            }
            CommandId::WelcomeNew => self.start_new_document(),
        }
    }

    fn execute_direction(&mut self, motion: Motion, overlay_direction: isize) {
        if matches!(
            self.input_mode(),
            InputMode::Overlay | InputMode::Help | InputMode::Welcome
        ) {
            self.update(Action::OverlayMove(overlay_direction));
        } else {
            self.update(Action::Move(motion));
        }
    }

    fn submit_prompt(&mut self, prompt: Prompt) {
        let value = prompt.value().trim().trim_matches('"').to_owned();
        match prompt.kind() {
            PromptKind::OpenFile => {
                if let Some(path) = self.resolve_path(&value) {
                    self.request_open(path);
                }
            }
            PromptKind::SaveAs => {
                if let Some(path) = self.resolve_path(&value) {
                    if path.exists() {
                        self.confirm_overwrite(ConfirmIntent::SaveAs(path));
                    } else {
                        self.save_as(path);
                    }
                }
            }
            PromptKind::Export(format) => {
                if let Some(mut path) = self.resolve_path(&value) {
                    if path.extension().is_none() {
                        path.set_extension(format.extension());
                    }
                    if path.exists() {
                        self.confirm_overwrite(ConfirmIntent::Export(format, path));
                    } else {
                        self.start_export(format, path);
                    }
                }
            }
            PromptKind::GoToLine => self.go_to_line(&value),
            PromptKind::GoToPage => self.go_to_page(&value),
        }
    }

    fn confirm(&mut self, intent: ConfirmIntent) {
        match intent {
            ConfirmIntent::Open(path) => self.open_path(path),
            ConfirmIntent::SaveAs(path) => self.save_as(path),
            ConfirmIntent::Export(format, path) => self.start_export(format, path),
        }
    }

    fn confirm_overwrite(&mut self, intent: ConfirmIntent) {
        let path = match &intent {
            ConfirmIntent::Open(path) | ConfirmIntent::SaveAs(path) => path,
            ConfirmIntent::Export(_, path) => path,
        };
        self.overlay
            .confirm(format!("Overwrite {}?", path.display()), intent);
    }

    fn request_open(&mut self, path: PathBuf) {
        if self.editor.is_dirty() {
            self.overlay.confirm(
                "Discard unsaved changes and open file?".to_owned(),
                ConfirmIntent::Open(path),
            );
        } else {
            self.open_path(path);
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        let root = match self.workspace.root_for_document(&path) {
            Ok(root) => root,
            Err(error) => {
                self.status = Some(error);
                return;
            }
        };
        let opened = match Workspace::read_source(&path) {
            Ok(opened) => opened,
            Err(error) => {
                self.status = Some(error);
                return;
            }
        };
        let watch_error = self.watcher.retarget(&root, Some(&path)).err();
        self.editor.replace_document(&opened.text);
        self.preview.clear();
        self.diagnostics = Diagnostics::new(self.theme);
        self.workspace.opened(path.clone(), root.clone());
        self.set_explorer_root(root.clone());
        self.welcome = None;
        self.focus = Pane::Editor;
        self.fullscreen = false;
        self.status = None;
        if let Err(error) = self
            .pipeline
            .reset_for_world(root, path.clone(), Instant::now())
        {
            self.status = Some(error);
        } else {
            self.start_compile();
        }
        if opened.existed {
            self.record_recent(&path);
        }
        if let Some(error) = watch_error {
            self.status = Some(format!("File watch failed: {error}"));
        }
    }

    fn start_new_document(&mut self) {
        self.welcome = None;
        self.status = None;
        self.start_compile();
    }

    fn open_file_prompt(&mut self) {
        self.overlay.open_prompt(PromptKind::OpenFile, "");
    }

    fn save(&mut self) {
        let Some(path) = self.workspace.path().map(Path::to_owned) else {
            self.overlay.open_prompt(PromptKind::SaveAs, "");
            return;
        };
        if self.write_document(&path) {
            self.record_recent(&path);
        }
    }

    fn save_as(&mut self, path: PathBuf) {
        let root = match self.workspace.root_for_document(&path) {
            Ok(root) => root,
            Err(error) => {
                self.status = Some(error);
                return;
            }
        };
        if !self.write_document(&path) {
            return;
        }
        self.workspace.saved_as(path.clone(), root);
        let watch_error = self
            .watcher
            .retarget(self.workspace.root(), Some(&path))
            .err();
        self.set_explorer_root(self.workspace.root().to_owned());
        self.start_compile_with_world();
        self.record_recent(&path);
        if let Some(error) = watch_error {
            self.status = Some(format!("File watch failed: {error}"));
        }
    }

    fn write_document(&mut self, path: &Path) -> bool {
        match Workspace::write_source(path, &self.editor.text()) {
            Ok(()) => {
                self.editor.mark_saved();
                self.status = Some(format!("Saved {}", path.display()));
                true
            }
            Err(error) => {
                self.status = Some(error);
                false
            }
        }
    }

    fn record_recent(&mut self, path: &Path) {
        if let Err(error) = self.recent.record(path) {
            self.status = Some(error);
        }
    }

    fn resolve_path(&mut self, value: &str) -> Option<PathBuf> {
        match self.workspace.resolve_path(value) {
            Ok(path) => Some(path),
            Err(error) => {
                self.status = Some(error);
                None
            }
        }
    }

    fn default_export_path(&self, format: ExportFormat) -> PathBuf {
        self.workspace.default_export_path(format)
    }

    fn start_export(&mut self, format: ExportFormat, path: PathBuf) {
        let Some(document) = self.pipeline.compiled_document(self.editor.revision()) else {
            self.status = Some("Current document is not compiled yet".to_owned());
            return;
        };
        self.status = Some(format!("Exporting {}...", format.label()));
        self.export_worker.spawn(document.clone(), format, path);
    }

    fn finish_export(&mut self, result: ExportResult) {
        self.status = Some(match result.result {
            Ok(()) => format!(
                "Exported {} to {}",
                result.format.label(),
                result.path.display()
            ),
            Err(error) => format!("Export failed: {error}"),
        });
    }

    fn go_to_line(&mut self, value: &str) {
        let line = value.parse::<usize>().ok().filter(|line| *line > 0);
        if let Some(line) = line
            && self.editor.set_cursor_line_char(line - 1, 0)
        {
            self.editor.reset_preferred_visual_column();
            self.focus = Pane::Editor;
            self.status = None;
            return;
        }
        self.status = Some("Line is outside the document".to_owned());
    }

    fn go_to_page(&mut self, value: &str) {
        let page = value.parse::<usize>().ok().unwrap_or(0);
        if self.preview.go_to_page(page) {
            self.focus = Pane::Preview;
            self.status = None;
        } else {
            self.status = Some("Page is outside the preview".to_owned());
        }
    }

    fn set_theme(&mut self, name: ThemeName) {
        self.theme = Theme::new(name, self.color_depth);
        self.editor.set_theme(self.theme);
        self.preview.set_theme(self.theme);
        self.diagnostics.set_theme(self.theme);
        self.explorer.set_theme(self.theme);
        if let Some(welcome) = &mut self.welcome {
            welcome.set_theme(self.theme);
        }
        self.header.set_theme(self.theme);
        self.status_bar.set_theme(self.theme);
        self.overlay.set_theme(self.theme);
        self.status = Some(format!("Theme: {}", name.as_str()));
    }

    fn toggle_explorer(&mut self) {
        self.explorer.toggle();
        if self.explorer.is_visible() {
            self.focus = Pane::Explorer;
            if !self.explorer.is_loaded() {
                self.start_explorer_scan();
            }
        } else if self.focus == Pane::Explorer {
            self.focus = Pane::Editor;
        }
    }

    fn project_files_changed(&mut self) {
        match self.watcher.take_project_changes() {
            Ok(changes) => {
                let paths = changes.paths();
                if changes.requires_rescan() || self.explorer.apply_paths(&paths) {
                    self.explorer.invalidate();
                    self.explorer_scan_generation = 0;
                    if self.explorer.is_visible() {
                        self.start_explorer_scan();
                    }
                }
            }
            Err(error) => self.status = Some(error),
        }
        if let Err(error) = self.pipeline.project_files_changed(Instant::now()) {
            self.status = Some(error);
        }
    }

    fn set_explorer_root(&mut self, root: PathBuf) {
        self.explorer.set_root(root);
        self.explorer_scan_generation = 0;
        if self.explorer.is_visible() {
            self.start_explorer_scan();
        }
    }

    fn start_explorer_scan(&mut self) {
        self.explorer_scan_generation =
            self.explorer_worker.spawn(self.workspace.root().to_owned());
    }

    fn finish_explorer_scan(&mut self, result: ExplorerScanResult) {
        if result.generation == self.explorer_scan_generation
            && result.root == self.workspace.root()
        {
            self.explorer.install_files(result.files);
        }
    }

    fn switch_focus(&mut self) {
        self.focus = match (self.focus, self.explorer.is_visible()) {
            (Pane::Explorer, _) => Pane::Editor,
            (Pane::Editor, _) => Pane::Preview,
            (Pane::Preview, true) => Pane::Explorer,
            (Pane::Preview, false) => Pane::Editor,
        };
    }

    fn update_editor(&mut self, action: Action) {
        let revision = self.editor.revision();
        let cursor = self.editor.cursor_byte_index();
        self.editor.update(action);
        if self.editor.revision() != revision {
            if let Err(error) = self.pipeline.document_changed(Instant::now()) {
                self.status = Some(error);
                return;
            }
            self.cursor_sync_deadline = None;
        } else if self.editor.cursor_byte_index() != cursor {
            self.cursor_sync_deadline = Some(Instant::now() + CURSOR_SYNC_DELAY);
        }
        self.status = None;
    }

    fn open_search(&mut self, mode: SearchMode) {
        let query = self.editor.selected_text().unwrap_or_default();
        self.overlay.open_search(mode, query);
    }

    fn search_next(&mut self, reverse: bool) {
        let Some(query) = self.overlay.search_query().map(str::to_owned) else {
            return;
        };
        if query.is_empty() {
            return;
        }
        let Some(range) = self.editor.find(&query, reverse) else {
            self.status = Some(format!("No matches for {query}"));
            return;
        };
        if self.editor.select_byte_range(range) {
            self.editor.reset_preferred_visual_column();
            self.cursor_sync_deadline = Some(Instant::now() + CURSOR_SYNC_DELAY);
            self.status = None;
        }
    }

    fn replace_current(&mut self) {
        let Some((query, replacement, can_replace)) =
            self.overlay
                .search_replace()
                .map(|(query, replacement, can_replace)| {
                    (query.to_owned(), replacement.to_owned(), can_replace)
                })
        else {
            return;
        };
        if !can_replace || query.is_empty() {
            return;
        }
        if self.editor.selected_text().as_deref() != Some(query.as_str()) {
            self.search_next(false);
        }
        if self.editor.selected_text().as_deref() != Some(query.as_str()) {
            return;
        }
        if replacement.is_empty() {
            self.update_editor(Action::Delete);
        } else {
            self.update_editor(Action::InsertText(replacement));
        }
        self.search_next(false);
    }

    fn copy_selection(&mut self) {
        let Some(text) = self.editor.selected_text() else {
            self.status = Some("No selection to copy".to_owned());
            return;
        };
        let system = self.clipboard.copy(&text);
        self.status = Some(if system {
            "Copied selection".to_owned()
        } else {
            "Copied selection to internal register".to_owned()
        });
    }

    fn cut_selection(&mut self) {
        let Some(text) = self.editor.selected_text() else {
            self.status = Some("No selection to cut".to_owned());
            return;
        };
        let system = self.clipboard.copy(&text);
        self.update_editor(Action::Delete);
        self.status = Some(if system {
            "Cut selection".to_owned()
        } else {
            "Cut selection to internal register".to_owned()
        });
    }

    fn paste_clipboard(&mut self) {
        let Some((text, system)) = self.clipboard.paste() else {
            self.status = Some("Clipboard is empty or unavailable".to_owned());
            return;
        };
        self.update_editor(Action::InsertText(text));
        if !system {
            self.status = Some("Pasted from internal register".to_owned());
        }
    }

    fn mouse_down(&mut self, column: u16, row: u16) {
        if self.preview.contains(column, row) {
            self.click_preview(column, row);
        } else if self.editor.place_cursor(column, row, false) {
            self.focus = Pane::Editor;
            self.cursor_sync_deadline = Some(Instant::now() + CURSOR_SYNC_DELAY);
            self.status = None;
        }
    }

    fn mouse_drag(&mut self, column: u16, row: u16) {
        if self.editor.place_cursor(column, row, true) {
            self.focus = Pane::Editor;
            self.cursor_sync_deadline = Some(Instant::now() + CURSOR_SYNC_DELAY);
            self.status = None;
        }
    }

    fn scroll_at(&mut self, column: u16, row: u16, lines: isize) {
        if self.preview.contains(column, row) {
            self.preview.scroll_lines(lines);
            self.focus = Pane::Preview;
        } else if self.editor.contains(column, row) {
            self.editor.scroll_lines(lines);
            self.focus = Pane::Editor;
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if let Some(update) = self
            .pipeline
            .tick(now, self.editor.revision(), self.editor.source())
        {
            self.apply_pipeline_update(update);
        }
        if self
            .cursor_sync_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.cursor_sync_deadline = None;
            self.sync_cursor_to_preview();
        }
    }

    fn observe_preview_width(&mut self) {
        let width = self.preview.target_width();
        if let Some(update) = self
            .pipeline
            .set_preview_width(width, self.editor.revision())
        {
            self.apply_pipeline_update(update);
        }
    }

    fn start_compile(&mut self) {
        let _ = self
            .pipeline
            .set_preview_width(self.preview.target_width(), self.editor.revision());
        self.pipeline
            .start_compile(self.editor.revision(), self.editor.source(), Instant::now());
        self.status = None;
    }

    fn start_compile_with_world(&mut self) {
        let main = self.workspace.main_path();
        let _ = self
            .pipeline
            .set_preview_width(self.preview.target_width(), self.editor.revision());
        self.pipeline.start_world_compile(
            self.editor.revision(),
            self.editor.source(),
            self.workspace.root().to_owned(),
            main,
            Instant::now(),
        );
        self.status = None;
    }

    fn request_preview_pages(&mut self) {
        self.pipeline
            .request_pages(self.editor.revision(), self.preview.page_requests());
    }

    fn poll_pipeline(&mut self) {
        let updates = self
            .pipeline
            .poll(self.editor.revision(), self.editor.source_text());
        for update in updates {
            self.apply_pipeline_update(update);
        }
    }

    fn apply_pipeline_update(&mut self, update: PipelineUpdate) {
        match update {
            PipelineUpdate::SourceRebuilt(source) => {
                if let Err(error) = self.editor.replace_source(source) {
                    self.status = Some(error);
                }
            }
            PipelineUpdate::Compiled {
                diagnostics,
                manifest,
                width,
            } => {
                self.diagnostics.set_items(diagnostics);
                self.editor
                    .set_diagnostic_lines(self.diagnostics.line_severities());
                self.preview
                    .replace_render_manifest(self.pipeline.picker(), &manifest, width);
                self.status = None;
                self.sync_cursor_to_preview();
                self.request_preview_pages();
            }
            PipelineUpdate::Failed(diagnostics) => {
                self.status = diagnostics.first().map(format_diagnostic);
                self.diagnostics.set_items(diagnostics);
                self.editor
                    .set_diagnostic_lines(self.diagnostics.line_severities());
            }
            PipelineUpdate::ManifestRescaled { manifest, width } => {
                self.preview
                    .rescale_render_manifest(self.pipeline.picker(), &manifest, width);
                self.request_preview_pages();
            }
            PipelineUpdate::Pages(pages) => self.preview.install_pages(pages),
            PipelineUpdate::Stalled => {
                self.status = Some("Compilation is taking longer than expected".to_owned());
            }
            PipelineUpdate::Error(error) => self.status = Some(error),
        }
    }

    fn navigate_diagnostic(&mut self, direction: isize) {
        let Some(diagnostic) = self.diagnostics.select(direction).cloned() else {
            self.status = Some("No diagnostics".to_owned());
            return;
        };

        self.status = Some(format_diagnostic(&diagnostic));
        if diagnostic.is_main
            && let Some(line) = diagnostic.line
            && self
                .editor
                .set_cursor_line_char(line, diagnostic.column.unwrap_or(0))
        {
            self.editor.reset_preferred_visual_column();
            self.focus = Pane::Editor;
            self.cursor_sync_deadline = Some(Instant::now() + CURSOR_SYNC_DELAY);
        }
    }

    fn click_preview(&mut self, column: u16, row: u16) {
        let Some(position) = self.preview.position_at(column, row) else {
            return;
        };
        let Some(byte) = self
            .pipeline
            .document_sync(self.editor.revision())
            .and_then(|sync| sync.source_from_click(position))
        else {
            return;
        };
        if self.editor.set_cursor_byte_index(byte) {
            self.editor.reset_preferred_visual_column();
            self.focus = Pane::Editor;
            self.cursor_sync_deadline = None;
            self.status = None;
        }
    }

    fn sync_cursor_to_preview(&mut self) {
        let position = self
            .pipeline
            .document_sync(self.editor.revision())
            .and_then(|sync| sync.position_from_cursor(self.editor.cursor_byte_index()));
        if let Some(position) = position {
            self.preview.scroll_to(position);
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(base(&self.theme)), area);
        if let Some(welcome) = &mut self.welcome {
            self.editor.hide();
            self.preview.hide();
            welcome.draw(frame, area, true);
            if let Some(status) = &self.status {
                frame.render_widget(
                    Paragraph::new(status.as_str())
                        .centered()
                        .style(Style::default().fg(color(self.theme.warning))),
                    Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 1),
                );
            }
        } else {
            self.draw_editor(frame);
        }
        self.draw_overlay(frame);
    }

    fn draw_editor(&mut self, frame: &mut Frame) {
        let [header_area, content_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let header_state = HeaderState {
            display_name: self.workspace.display_name().to_owned(),
            dirty: self.editor.is_dirty(),
            compile_label: self.compile_label(),
            compile_color: self.compile_color(),
        };
        self.header.set_state(header_state);
        self.header.draw(frame, header_area, false);

        let (workspace_area, diagnostics_area) = if self.diagnostics.is_visible() {
            let height = self.diagnostics.drawer_height(content_area.height);
            let [workspace, drawer] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(height)])
                    .areas(content_area);
            (workspace, Some(drawer))
        } else {
            (content_area, None)
        };
        self.draw_workspace(frame, workspace_area);
        if let Some(area) = diagnostics_area {
            self.diagnostics.draw(frame, area, false);
        }
        self.draw_status(frame, status_area);
    }

    fn draw_workspace(&mut self, frame: &mut Frame, area: Rect) {
        let preview_dimmed = self.pipeline.is_stale();
        if self.fullscreen {
            match self.focus {
                Pane::Explorer => {
                    self.editor.hide();
                    self.preview.hide();
                    self.explorer.draw(frame, area, true);
                }
                Pane::Editor => {
                    self.preview.hide();
                    self.editor.draw(frame, area, true)
                }
                Pane::Preview => {
                    self.editor.hide();
                    self.preview.draw_preview(frame, area, true, preview_dimmed);
                }
            }
            return;
        }

        let (explorer_area, main_area) = if self.explorer.is_visible() && area.width >= NARROW_WIDTH
        {
            let [explorer, main] =
                Layout::horizontal([Constraint::Length(26), Constraint::Fill(1)]).areas(area);
            (Some(explorer), main)
        } else {
            (None, area)
        };
        if let Some(explorer_area) = explorer_area {
            self.explorer
                .draw(frame, explorer_area, self.focus == Pane::Explorer);
        }
        if area.width < NARROW_WIDTH && self.focus == Pane::Explorer {
            self.editor.hide();
            self.preview.hide();
            self.explorer.draw(frame, main_area, true);
        } else if main_area.width < NARROW_WIDTH {
            self.preview.set_viewport(main_area);
            match self.focus {
                Pane::Preview => {
                    self.editor.hide();
                    self.preview
                        .draw_preview(frame, main_area, true, preview_dimmed);
                }
                Pane::Explorer | Pane::Editor => {
                    self.preview.hide();
                    self.editor.draw(frame, main_area, true);
                }
            }
        } else {
            let [editor_area, preview_area] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(main_area);
            self.editor
                .draw(frame, editor_area, self.focus == Pane::Editor);
            self.preview.draw_preview(
                frame,
                preview_area,
                self.focus == Pane::Preview,
                preview_dimmed,
            );
        }
    }

    fn draw_status(&mut self, frame: &mut Frame, area: Rect) {
        self.status_bar.set_state(StatusBarState {
            cursor: self.editor.cursor_position(),
            selection_graphemes: self.editor.selection_graphemes(),
            word_count: self.editor.word_count(),
            errors: self.diagnostics.errors(),
            warnings: self.diagnostics.warnings(),
            compile_time: self.pipeline.last_compile_time(),
            message: self.status.clone(),
            quit_confirmation: self.quit_confirmation,
        });
        self.status_bar.draw(frame, area, false);
    }

    fn draw_overlay(&mut self, frame: &mut Frame) {
        self.overlay.draw(frame, frame.area(), true);
    }

    fn compile_label(&self) -> String {
        match self.pipeline.state() {
            CompileState::NotStarted => "not compiled".to_owned(),
            CompileState::Compiling => "● compiling".to_owned(),
            CompileState::Ready => "✓ up to date".to_owned(),
            CompileState::Stale => "● preview stale".to_owned(),
            CompileState::Stalled => "● compilation stalled".to_owned(),
            CompileState::Failed(errors) => format!("✕ {errors} errors"),
            CompileState::Error => "✕ preview error".to_owned(),
        }
    }

    fn compile_color(&self) -> Color {
        match self.pipeline.state() {
            CompileState::Failed(_) | CompileState::Error => self.theme.error,
            CompileState::Stale | CompileState::Compiling | CompileState::Stalled => {
                self.theme.warning
            }
            CompileState::Ready => self.theme.success,
            CompileState::NotStarted => self.theme.muted,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        path::PathBuf,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use oxyst_compiler::{CompileOutcome, Compiler};
    use oxyst_config::{CommandId, Config};
    use oxyst_pipeline::CompileState;
    use ratatui::{Terminal, backend::TestBackend};
    use ratatui_image::picker::Picker;

    use super::{App, AppInit, Preview, overlay_transition_requires_clear};
    use crate::action::Action;

    #[test]
    fn clearing_or_submitting_an_overlay_requests_a_full_redraw() {
        assert!(overlay_transition_requires_clear(&Action::Command(
            oxyst_config::CommandId::CloseOverlay
        )));
        assert!(overlay_transition_requires_clear(&Action::OverlaySubmit));
        assert!(!overlay_transition_requires_clear(&Action::OverlayInput(
            'x'
        )));
        assert!(!overlay_transition_requires_clear(&Action::OverlayMove(1)));
    }

    #[test]
    #[ignore = "manual release-mode performance probe"]
    fn scale_refresh_reports_visible_page_latency_without_compiling() -> Result<(), Box<dyn Error>>
    {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let source = (0..100)
            .map(|page| format!("#rect(width: 100%, height: 100%, fill: rgb({page}, 80, 120))"))
            .collect::<Vec<_>>()
            .join("\n#pagebreak()\n");
        let CompileOutcome::Success(document) = compiler.compile(&source) else {
            return Err("fixture did not compile".into());
        };
        let picker = Picker::halfblocks();
        let mut layout_samples = Vec::new();
        let mut render_samples = Vec::new();
        let mut encoding_samples = Vec::new();
        let mut total_samples = Vec::new();

        for target_width in 60..100 {
            let total_started = Instant::now();
            let layout_started = Instant::now();
            let width = target_width;
            let pixels = u32::from(width) * 4;
            let manifest = oxyst_render::render_manifest(&document, pixels)?;
            let page_sizes = Preview::page_sizes(&picker, &manifest, width);
            layout_samples.push(layout_started.elapsed());

            let render_started = Instant::now();
            let rendered =
                oxyst_render::render_pages_cancellable(&document, &manifest, [0], || false)?;
            render_samples.push(render_started.elapsed());

            let encoding_started = Instant::now();
            let encoded =
                Preview::encode_rendered_pages_cancellable(&picker, rendered, width, || false)?
                    .ok_or("encoding was cancelled")?;
            encoding_samples.push(encoding_started.elapsed());
            total_samples.push(total_started.elapsed());

            assert_eq!(page_sizes.len(), 100);
            assert_eq!(encoded.len(), 1);
        }

        let p95 = |samples: &mut Vec<Duration>| {
            samples.sort_unstable();
            samples[(samples.len() - 1) * 95 / 100]
        };
        eprintln!(
            "100-page scale refresh: layout_p95={:?}, render_p95={:?}, encoding_p95={:?}, total_p95={:?}",
            p95(&mut layout_samples),
            p95(&mut render_samples),
            p95(&mut encoding_samples),
            p95(&mut total_samples),
        );
        Ok(())
    }

    #[test]
    fn header_and_status_show_glyphs_and_word_count() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let main = root.join("simple.typ");
        let compiler = Compiler::new(&root, &main)?;
        let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
        let mut config = Config::default();
        config
            .keys
            .insert("confirm".to_owned(), vec!["alt+y".to_owned()]);
        config
            .keys
            .insert("cancel_confirmation".to_owned(), vec!["alt+n".to_owned()]);
        let mut app = App::new(AppInit {
            path: Some(main),
            root,
            root_is_explicit: false,
            text: "one two",
            compiler,
            picker: Picker::halfblocks(),
            runtime: runtime.handle().clone(),
            config,
            recent: crate::recent::RecentFiles::disabled(),
            startup_status: None,
        })
        .map_err(std::io::Error::other)?;
        app.editor.update(crate::action::Action::Insert('x'));
        app.start_compile();
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.pipeline.state() != CompileState::Ready && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            app.poll_pipeline();
        }
        assert_eq!(app.pipeline.state(), CompileState::Ready);
        let mut terminal = Terminal::new(TestBackend::new(100, 20))?;
        terminal.draw(|frame| app.draw(frame))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("●"));
        assert!(rendered.contains("✓ up to date"));
        assert!(rendered.contains("2 words"));

        app.quit_confirmation = true;
        terminal.draw(|frame| app.draw(frame))?;
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("alt+y quit"));
        assert!(rendered.contains("alt+n cancel"));

        drop(app);
        runtime.shutdown_timeout(Duration::from_millis(100));
        Ok(())
    }

    #[test]
    fn explicit_root_rejects_save_as_before_writing() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let compiler = Compiler::new(&root, root.join("untitled.typ"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let outside =
            std::env::temp_dir().join(format!("oxyst-save-as-{}-{unique}", std::process::id()));
        fs::create_dir_all(&outside)?;
        let destination = outside.join("report.typ");
        let mut app = App::new(AppInit {
            path: None,
            root: root.clone(),
            root_is_explicit: true,
            text: "= Draft",
            compiler,
            picker: Picker::halfblocks(),
            runtime: runtime.handle().clone(),
            config: Config::default(),
            recent: crate::recent::RecentFiles::disabled(),
            startup_status: None,
        })
        .map_err(std::io::Error::other)?;

        app.execute_command(CommandId::Backspace);
        assert_eq!(app.editor.text(), "= Draft");

        app.save_as(destination.clone());

        assert!(!destination.exists());
        assert!(app.workspace.path().is_none());
        assert_eq!(app.workspace.root(), root);
        assert!(
            app.status
                .as_deref()
                .is_some_and(|status| status.contains("outside project root"))
        );

        drop(app);
        runtime.shutdown_timeout(Duration::from_millis(100));
        fs::remove_dir_all(outside)?;
        Ok(())
    }
}
