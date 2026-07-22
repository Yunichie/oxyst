use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use ratatui_image::picker::Picker;
use tokio::runtime::Handle;
use typst_tui_compiler::{CompiledDocument, Compiler, DocumentSync, Severity};
use typst_tui_config::Config;
use typst_tui_document::{Document, Motion};
use typst_tui_render::ExportFormat;
use typst_tui_theme::{Color, ColorDepth, Theme, ThemeName};

use crate::{
    action::{Action, Pane},
    compile::{CompileResult, CompileResultKind, CompileWorker},
    components::{
        Command, CommandPalette, Diagnostics, Editor, FileExplorer, Help, Preview, Prompt,
        PromptKind, Welcome, WelcomeChoice, format_diagnostic, modal_area,
    },
    event::{self, Event},
    export::{ExportResult, ExportWorker},
    input::{self, InputMode, Keymap},
    style::{base, color},
};

const NARROW_WIDTH: u16 = 80;
const AUTO_COMPILE_DELAY: Duration = Duration::from_millis(150);
const CURSOR_SYNC_DELAY: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompileState {
    NotStarted,
    Compiling,
    Ready,
    Stale,
    Failed(usize),
    Error,
}

#[derive(Default)]
struct CompileDebounce {
    deadline: Option<Instant>,
}

impl CompileDebounce {
    fn schedule(&mut self, now: Instant) {
        self.deadline = Some(now + AUTO_COMPILE_DELAY);
    }

    fn cancel(&mut self) {
        self.deadline = None;
    }

    fn take_due(&mut self, now: Instant) -> bool {
        match self.deadline {
            Some(deadline) if now >= deadline => {
                self.deadline = None;
                true
            }
            _ => false,
        }
    }
}

#[derive(Debug, Default)]
enum Overlay {
    #[default]
    None,
    Palette(CommandPalette),
    Prompt(Prompt),
    Help(Help),
    Confirm(Confirmation),
}

#[derive(Debug)]
struct Confirmation {
    message: String,
    intent: ConfirmIntent,
}

#[derive(Debug)]
enum ConfirmIntent {
    Open(PathBuf),
    SaveAs(PathBuf),
    Export(ExportFormat, PathBuf),
}

pub(crate) struct App {
    document: Document,
    editor: Editor,
    preview: Preview,
    explorer: FileExplorer,
    focus: Pane,
    fullscreen: bool,
    path: Option<PathBuf>,
    root: PathBuf,
    display_name: String,
    picker: Picker,
    compile_worker: CompileWorker,
    export_worker: ExportWorker,
    internal_events: Receiver<Event>,
    compile_debounce: CompileDebounce,
    compile_generation: u64,
    compiled_document: Option<(u64, CompiledDocument)>,
    document_sync: Option<(u64, DocumentSync)>,
    cursor_sync_deadline: Option<Instant>,
    compile_state: CompileState,
    diagnostics: Diagnostics,
    last_compile_time: Option<Duration>,
    keymap: Keymap,
    color_depth: ColorDepth,
    theme: Theme,
    welcome: Option<Welcome>,
    overlay: Overlay,
    quit_confirmation: bool,
    should_quit: bool,
    status: Option<String>,
}

impl App {
    pub(crate) fn new(
        path: Option<PathBuf>,
        root: PathBuf,
        text: &str,
        compiler: Compiler,
        picker: Picker,
        runtime: Handle,
        config: Config,
    ) -> Result<Self, String> {
        let display_name = display_name(path.as_deref());
        let color_depth = ColorDepth::detect();
        let theme = Theme::named(&config.theme, color_depth).map_err(|error| error.to_string())?;
        let keymap = Keymap::new(&config)?;
        let welcome = path.is_none().then(Welcome::default);
        let (sender, internal_events) = channel();

        Ok(Self {
            document: Document::new(text),
            editor: Editor::new(text, theme),
            preview: Preview::new(),
            explorer: FileExplorer::new(root.clone()),
            focus: Pane::Editor,
            fullscreen: false,
            path,
            root,
            display_name,
            picker,
            compile_worker: CompileWorker::new(compiler, sender.clone(), runtime.clone()),
            export_worker: ExportWorker::new(sender, runtime),
            internal_events,
            compile_debounce: CompileDebounce::default(),
            compile_generation: 0,
            compiled_document: None,
            document_sync: None,
            cursor_sync_deadline: None,
            compile_state: CompileState::NotStarted,
            diagnostics: Diagnostics::default(),
            last_compile_time: None,
            keymap,
            color_depth,
            theme,
            welcome,
            overlay: Overlay::None,
            quit_confirmation: false,
            should_quit: false,
            status: None,
        })
    }

    pub(crate) fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let mut initial_compile_requested = self.welcome.is_some();
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            if !initial_compile_requested {
                self.start_compile();
                initial_compile_requested = true;
            }
            if let Some(action) = input::resolve(
                event::read(&self.internal_events)?,
                self.input_mode(),
                &self.keymap,
            ) {
                self.update(action);
            }
        }

        Ok(())
    }

    fn input_mode(&self) -> InputMode {
        if self.quit_confirmation {
            return InputMode::QuitConfirmation;
        }
        match self.overlay {
            Overlay::Palette(_) | Overlay::Prompt(_) => InputMode::Overlay,
            Overlay::Help(_) => InputMode::Help,
            Overlay::Confirm(_) => InputMode::Confirmation,
            Overlay::None if self.welcome.is_some() => InputMode::Welcome,
            Overlay::None => InputMode::Normal,
        }
    }

    fn update(&mut self, action: Action) {
        match action {
            Action::CloseOverlay => self.overlay = Overlay::None,
            Action::OverlayInput(_)
            | Action::OverlayInputText(_)
            | Action::OverlayBackspace
            | Action::OverlayMove(_)
            | Action::OverlaySubmit => self.update_overlay(action),
            Action::OpenCommandPalette => {
                self.overlay = Overlay::Palette(CommandPalette::default());
            }
            Action::OpenHelp => self.overlay = Overlay::Help(Help::default()),
            Action::OpenGoToLine => {
                self.overlay = Overlay::Prompt(Prompt::new(PromptKind::GoToLine, ""));
            }
            Action::Save => self.save(),
            Action::Recompile => self.start_compile(),
            Action::CompileFinished(result) => self.finish_compile(result),
            Action::ExportFinished(result) => self.finish_export(result),
            Action::Tick => self.tick(),
            Action::ToggleDiagnostics => self.diagnostics.toggle(),
            Action::ToggleFileExplorer => self.toggle_explorer(),
            Action::ToggleFullscreen => self.fullscreen = !self.fullscreen,
            Action::ZoomPreview(direction) if self.preview.zoom(direction) => {
                self.focus = Pane::Preview;
                self.start_compile();
            }
            Action::NavigateDiagnostic(direction) => self.navigate_diagnostic(direction),
            Action::Click { column, row } => self.click_preview(column, row),
            Action::ScrollAt { column, row, lines } if self.preview.contains(column, row) => {
                self.preview.scroll_lines(lines);
                self.focus = Pane::Preview;
            }
            Action::SwitchFocus => self.switch_focus(),
            Action::ScrollPreviewPages(pages) => self.preview.scroll_pages(pages),
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
            Action::RequestQuit if self.document.is_dirty() => {
                self.quit_confirmation = true;
                self.overlay = Overlay::None;
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
            && matches!(self.overlay, Overlay::None)
        {
            match action {
                Action::OverlayMove(direction) => welcome.move_selection(direction),
                Action::OverlayInput('n' | 'N') => self.start_new_document(),
                Action::OverlayInput('o' | 'O') => self.open_file_prompt(),
                _ => {}
            }
            return;
        }
        match (&mut self.overlay, action) {
            (Overlay::Palette(palette), Action::OverlayInput(character)) => {
                palette.input(character);
            }
            (Overlay::Palette(palette), Action::OverlayInputText(text)) => {
                palette.input_text(&text);
            }
            (Overlay::Palette(palette), Action::OverlayBackspace) => palette.backspace(),
            (Overlay::Palette(palette), Action::OverlayMove(direction)) => {
                palette.move_selection(direction);
            }
            (Overlay::Prompt(prompt), Action::OverlayInput(character)) => {
                prompt.input(character);
            }
            (Overlay::Prompt(prompt), Action::OverlayInputText(text)) => {
                prompt.input_text(&text);
            }
            (Overlay::Prompt(prompt), Action::OverlayBackspace) => prompt.backspace(),
            (Overlay::Help(help), Action::OverlayMove(direction)) => help.scroll(direction),
            _ => {}
        }
    }

    fn submit_overlay(&mut self) {
        if let Some(welcome) = &self.welcome
            && matches!(self.overlay, Overlay::None)
        {
            match welcome.selected() {
                WelcomeChoice::NewDocument => self.start_new_document(),
                WelcomeChoice::OpenFile => self.open_file_prompt(),
                WelcomeChoice::OpenRecent => {
                    self.status = Some("No recent files".to_owned());
                }
            }
            return;
        }

        match std::mem::take(&mut self.overlay) {
            Overlay::Palette(palette) => {
                if let Some(command) = palette.selected() {
                    self.execute_command(command);
                }
            }
            Overlay::Prompt(prompt) => self.submit_prompt(prompt),
            Overlay::Confirm(confirmation) => self.confirm(confirmation.intent),
            Overlay::Help(_) | Overlay::None => {}
        }
    }

    fn execute_command(&mut self, command: Command) {
        match command {
            Command::OpenFile => self.open_file_prompt(),
            Command::Export(format) => {
                let path = self.default_export_path(format);
                self.overlay = Overlay::Prompt(Prompt::new(
                    PromptKind::Export(format),
                    path.to_string_lossy(),
                ));
            }
            Command::GoToLine => {
                self.overlay = Overlay::Prompt(Prompt::new(PromptKind::GoToLine, ""));
            }
            Command::GoToPage => {
                self.overlay = Overlay::Prompt(Prompt::new(PromptKind::GoToPage, ""));
            }
            Command::ToggleDiagnostics => self.diagnostics.toggle(),
            Command::ToggleFileExplorer => self.toggle_explorer(),
            Command::UseDarkTheme => self.set_theme(ThemeName::Dark),
            Command::UseLightTheme => self.set_theme(ThemeName::Light),
            Command::ReloadFonts => {
                self.start_compile_with_world();
                self.status = Some("Reloading fonts...".to_owned());
            }
            Command::Quit => self.update(Action::RequestQuit),
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
        self.overlay = Overlay::Confirm(Confirmation {
            message: format!("Overwrite {}? Y/N", path.display()),
            intent,
        });
    }

    fn request_open(&mut self, path: PathBuf) {
        if self.document.is_dirty() {
            self.overlay = Overlay::Confirm(Confirmation {
                message: "Discard unsaved changes and open file? Y/N".to_owned(),
                intent: ConfirmIntent::Open(path),
            });
        } else {
            self.open_path(path);
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => {
                self.status = Some(format!("Open failed: {error}"));
                return;
            }
        };
        let root = if path.starts_with(&self.root) {
            self.root.clone()
        } else {
            path.parent()
                .map_or_else(|| self.root.clone(), Path::to_owned)
        };
        self.document = Document::new(&text);
        self.editor = Editor::new(&text, self.theme);
        self.preview.clear();
        self.diagnostics = Diagnostics::default();
        self.document_sync = None;
        self.compiled_document = None;
        self.path = Some(path.clone());
        self.root = root.clone();
        self.display_name = display_name(Some(&path));
        self.explorer.set_root(root);
        self.welcome = None;
        self.focus = Pane::Editor;
        self.fullscreen = false;
        self.status = None;
        self.start_compile_with_world();
    }

    fn start_new_document(&mut self) {
        self.welcome = None;
        self.status = None;
        self.start_compile();
    }

    fn open_file_prompt(&mut self) {
        self.overlay = Overlay::Prompt(Prompt::new(PromptKind::OpenFile, ""));
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            self.overlay = Overlay::Prompt(Prompt::new(PromptKind::SaveAs, ""));
            return;
        };
        self.write_document(&path);
    }

    fn save_as(&mut self, path: PathBuf) {
        if !self.write_document(&path) {
            return;
        }
        self.path = Some(path.clone());
        self.root = path
            .parent()
            .map_or_else(|| self.root.clone(), Path::to_owned);
        self.display_name = display_name(Some(&path));
        self.explorer.set_root(self.root.clone());
        self.start_compile_with_world();
    }

    fn write_document(&mut self, path: &Path) -> bool {
        match fs::write(path, self.document.text()) {
            Ok(()) => {
                self.document.mark_saved();
                self.status = Some(format!("Saved {}", path.display()));
                true
            }
            Err(error) => {
                self.status = Some(format!("Save failed: {error}"));
                false
            }
        }
    }

    fn resolve_path(&mut self, value: &str) -> Option<PathBuf> {
        if value.is_empty() {
            self.status = Some("A path is required".to_owned());
            return None;
        }
        let path = PathBuf::from(value);
        if path.is_absolute() {
            Some(path)
        } else {
            match std::env::current_dir() {
                Ok(current) => Some(current.join(path)),
                Err(error) => {
                    self.status = Some(format!("Could not resolve path: {error}"));
                    None
                }
            }
        }
    }

    fn default_export_path(&self, format: ExportFormat) -> PathBuf {
        let mut path = self
            .path
            .clone()
            .unwrap_or_else(|| self.root.join("untitled.typ"));
        path.set_extension(format.extension());
        path
    }

    fn start_export(&mut self, format: ExportFormat, path: PathBuf) {
        let Some((_, document)) = self
            .compiled_document
            .as_ref()
            .filter(|(revision, _)| *revision == self.document.revision())
        else {
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
            && self.document.set_cursor_line_char(line - 1, 0)
        {
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
        self.status = Some(format!("Theme: {}", name.as_str()));
    }

    fn toggle_explorer(&mut self) {
        self.explorer.toggle();
        if self.explorer.is_visible() {
            self.focus = Pane::Explorer;
        } else if self.focus == Pane::Explorer {
            self.focus = Pane::Editor;
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
        let revision = self.document.revision();
        let cursor = self.document.cursor_byte_index();
        self.editor.update(&action, &mut self.document);
        if self.document.revision() != revision {
            self.compile_generation = self.compile_worker.invalidate();
            self.compile_debounce.schedule(Instant::now());
            self.cursor_sync_deadline = None;
            self.compile_state = CompileState::Stale;
        } else if self.document.cursor_byte_index() != cursor {
            self.cursor_sync_deadline = Some(Instant::now() + CURSOR_SYNC_DELAY);
        }
        self.status = None;
    }

    fn tick(&mut self) {
        if self.compile_debounce.take_due(Instant::now()) {
            self.start_compile();
        }
        if self
            .cursor_sync_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.cursor_sync_deadline = None;
            self.sync_cursor_to_preview();
        }
    }

    fn start_compile(&mut self) {
        self.compile_debounce.cancel();
        self.compile_generation = self.compile_worker.spawn(
            self.document.revision(),
            self.document.text(),
            self.picker.clone(),
            self.preview.target_width(),
        );
        self.compile_state = CompileState::Compiling;
        self.status = None;
    }

    fn start_compile_with_world(&mut self) {
        self.compile_debounce.cancel();
        let main = self
            .path
            .clone()
            .unwrap_or_else(|| self.root.join("untitled.typ"));
        self.compile_generation = self.compile_worker.spawn_with_world(
            self.document.revision(),
            self.document.text(),
            self.picker.clone(),
            self.preview.target_width(),
            self.root.clone(),
            main,
        );
        self.compile_state = CompileState::Compiling;
        self.status = None;
    }

    fn finish_compile(&mut self, result: CompileResult) {
        if result.generation != self.compile_generation {
            return;
        }
        if result.revision != self.document.revision() {
            self.compile_state = CompileState::Stale;
            self.status = None;
            return;
        }

        self.last_compile_time = Some(result.elapsed);
        match result.outcome {
            CompileResultKind::Success {
                pages,
                diagnostics,
                sync,
                document,
            } => {
                self.preview.replace_pages(pages);
                self.diagnostics.set_items(diagnostics);
                self.document_sync = Some((result.revision, sync));
                self.compiled_document = Some((result.revision, *document));
                self.compile_state = CompileState::Ready;
                self.status = None;
                self.sync_cursor_to_preview();
            }
            CompileResultKind::Diagnostics(diagnostics) => {
                let errors = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.severity == Severity::Error)
                    .count();
                self.status = diagnostics.first().map(format_diagnostic);
                self.diagnostics.set_items(diagnostics);
                self.compile_state = CompileState::Failed(errors);
            }
            CompileResultKind::Error(error) => {
                self.compile_state = CompileState::Error;
                self.status = Some(format!("Preview failed: {error}"));
            }
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
                .document
                .set_cursor_line_char(line, diagnostic.column.unwrap_or(0))
        {
            self.focus = Pane::Editor;
            self.cursor_sync_deadline = Some(Instant::now() + CURSOR_SYNC_DELAY);
        }
    }

    fn click_preview(&mut self, column: u16, row: u16) {
        let Some(position) = self.preview.position_at(column, row) else {
            return;
        };
        let Some(byte) = self
            .document_sync
            .as_ref()
            .filter(|(revision, _)| *revision == self.document.revision())
            .and_then(|(_, sync)| sync.source_from_click(position))
        else {
            return;
        };
        if self.document.set_cursor_byte_index(byte) {
            self.focus = Pane::Editor;
            self.cursor_sync_deadline = None;
            self.status = None;
        }
    }

    fn sync_cursor_to_preview(&mut self) {
        let position = self
            .document_sync
            .as_ref()
            .filter(|(revision, _)| *revision == self.document.revision())
            .and_then(|(_, sync)| sync.position_from_cursor(self.document.cursor_byte_index()));
        if let Some(position) = position {
            self.preview.scroll_to(position);
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(base(&self.theme)), area);
        if let Some(welcome) = &self.welcome {
            welcome.draw(frame, area, &self.theme);
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

        let dirty_marker = if self.document.is_dirty() { " *" } else { "" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " typst-tui ",
                    Style::default()
                        .fg(color(self.theme.accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{}{}", self.display_name, dirty_marker)),
                Span::styled(
                    format!(" | {}", self.compile_label()),
                    Style::default().fg(color(self.compile_color())),
                ),
                Span::styled(" | ? help", Style::default().fg(color(self.theme.muted))),
            ])),
            header_area,
        );

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
            self.diagnostics.draw(frame, area, &self.theme);
        }
        self.draw_status(frame, status_area);
    }

    fn draw_workspace(&mut self, frame: &mut Frame, area: Rect) {
        let preview_dimmed = self
            .document_sync
            .as_ref()
            .is_some_and(|(revision, _)| *revision != self.document.revision());
        if self.fullscreen {
            match self.focus {
                Pane::Explorer => self.explorer.draw(frame, area, true, &self.theme),
                Pane::Editor => {
                    self.editor
                        .draw(frame, area, &self.document, &self.diagnostics, true)
                }
                Pane::Preview => self
                    .preview
                    .draw(frame, area, true, preview_dimmed, &self.theme),
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
            self.explorer.draw(
                frame,
                explorer_area,
                self.focus == Pane::Explorer,
                &self.theme,
            );
        }
        if area.width < NARROW_WIDTH && self.focus == Pane::Explorer {
            self.preview.hide();
            self.explorer.draw(frame, main_area, true, &self.theme);
        } else if main_area.width < NARROW_WIDTH {
            self.preview.set_viewport(main_area);
            match self.focus {
                Pane::Preview => {
                    self.preview
                        .draw(frame, main_area, true, preview_dimmed, &self.theme);
                }
                Pane::Explorer | Pane::Editor => {
                    self.preview.hide();
                    self.editor
                        .draw(frame, main_area, &self.document, &self.diagnostics, true);
                }
            }
        } else {
            let [editor_area, preview_area] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(main_area);
            self.editor.draw(
                frame,
                editor_area,
                &self.document,
                &self.diagnostics,
                self.focus == Pane::Editor,
            );
            self.preview.draw(
                frame,
                preview_area,
                self.focus == Pane::Preview,
                preview_dimmed,
                &self.theme,
            );
        }
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let cursor = self.document.cursor_position();
        let errors = self.diagnostics.errors();
        let warnings = self.diagnostics.warnings();
        let compile_time = self.last_compile_time.map_or_else(
            || "--".to_owned(),
            |time| format!("{} ms", time.as_millis()),
        );
        let (status, foreground) = if self.quit_confirmation {
            (
                "Unsaved changes. Press Y to quit, N or Esc to cancel".to_owned(),
                self.theme.warning,
            )
        } else if let Some(status) = &self.status {
            (status.clone(), self.theme.foreground)
        } else {
            (
                format!("Ln {}, Col {}", cursor.line + 1, cursor.column + 1),
                self.theme.muted,
            )
        };
        let diagnostic_color = if errors > 0 {
            self.theme.error
        } else if warnings > 0 {
            self.theme.warning
        } else {
            self.theme.muted
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {status} | "),
                    Style::default().fg(color(foreground)),
                ),
                Span::styled(
                    format!("{errors} errors, {warnings} warnings"),
                    Style::default().fg(color(diagnostic_color)),
                ),
                Span::styled(
                    format!(" | {compile_time}"),
                    Style::default().fg(color(self.theme.muted)),
                ),
            ]))
            .style(Style::default().bg(color(self.theme.surface))),
            area,
        );
    }

    fn draw_overlay(&self, frame: &mut Frame) {
        match &self.overlay {
            Overlay::None => {}
            Overlay::Palette(palette) => palette.draw(frame, &self.theme),
            Overlay::Prompt(prompt) => prompt.draw(frame, &self.theme),
            Overlay::Help(help) => help.draw(frame, frame.area(), &self.keymap, &self.theme),
            Overlay::Confirm(confirmation) => {
                let area = modal_area(frame.area(), 72, 5);
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(confirmation.message.as_str())
                        .centered()
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_type(BorderType::Rounded)
                                .border_style(Style::default().fg(color(self.theme.warning)))
                                .style(base(&self.theme))
                                .title(" Confirm "),
                        ),
                    area,
                );
            }
        }
    }

    fn compile_label(&self) -> String {
        match self.compile_state {
            CompileState::NotStarted => "not compiled".to_owned(),
            CompileState::Compiling => "compiling".to_owned(),
            CompileState::Ready => "up to date".to_owned(),
            CompileState::Stale => "preview stale".to_owned(),
            CompileState::Failed(errors) => format!("{errors} errors"),
            CompileState::Error => "preview error".to_owned(),
        }
    }

    fn compile_color(&self) -> Color {
        match self.compile_state {
            CompileState::Failed(_) | CompileState::Error => self.theme.error,
            CompileState::Stale | CompileState::Compiling => self.theme.warning,
            CompileState::Ready => self.theme.success,
            CompileState::NotStarted => self.theme.muted,
        }
    }
}

fn display_name(path: Option<&Path>) -> String {
    path.and_then(Path::file_name).map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::CompileDebounce;

    #[test]
    fn compile_debounce_restarts_after_each_edit() {
        let start = Instant::now();
        let mut debounce = CompileDebounce::default();

        debounce.schedule(start);
        assert!(!debounce.take_due(start + Duration::from_millis(149)));

        debounce.schedule(start + Duration::from_millis(100));
        assert!(!debounce.take_due(start + Duration::from_millis(249)));
        assert!(debounce.take_due(start + Duration::from_millis(250)));
        assert!(!debounce.take_due(start + Duration::from_millis(300)));
    }
}
