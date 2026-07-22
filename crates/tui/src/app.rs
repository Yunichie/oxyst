use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use ratatui_image::picker::Picker;
use tokio::runtime::Handle;
use typst_tui_compiler::{Compiler, DocumentSync, Severity};
use typst_tui_document::{Document, Motion};

use crate::{
    action::{Action, Pane},
    compile::{CompileResult, CompileResultKind, CompileWorker},
    components::{Diagnostics, Editor, Preview, format_diagnostic},
    event::{self, Event},
    input,
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

pub(crate) struct App {
    document: Document,
    editor: Editor,
    preview: Preview,
    focus: Pane,
    path: Option<PathBuf>,
    display_name: String,
    picker: Picker,
    compile_worker: CompileWorker,
    internal_events: Receiver<Event>,
    compile_debounce: CompileDebounce,
    compile_generation: u64,
    document_sync: Option<(u64, DocumentSync)>,
    cursor_sync_deadline: Option<Instant>,
    compile_state: CompileState,
    diagnostics: Diagnostics,
    last_compile_time: Option<Duration>,
    quit_confirmation: bool,
    should_quit: bool,
    status: Option<String>,
}

impl App {
    pub(crate) fn new(
        path: Option<PathBuf>,
        text: &str,
        compiler: Compiler,
        picker: Picker,
        runtime: Handle,
    ) -> Self {
        let display_name = path
            .as_deref()
            .and_then(|path| path.file_name())
            .map_or_else(
                || "Untitled".to_owned(),
                |name| name.to_string_lossy().into(),
            );
        let (sender, internal_events) = channel();

        Self {
            document: Document::new(text),
            editor: Editor::new(text),
            preview: Preview::new(),
            focus: Pane::Editor,
            path,
            display_name,
            picker,
            compile_worker: CompileWorker::new(compiler, sender, runtime),
            internal_events,
            compile_debounce: CompileDebounce::default(),
            compile_generation: 0,
            document_sync: None,
            cursor_sync_deadline: None,
            compile_state: CompileState::NotStarted,
            diagnostics: Diagnostics::default(),
            last_compile_time: None,
            quit_confirmation: false,
            should_quit: false,
            status: None,
        }
    }

    pub(crate) fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let mut initial_compile_requested = false;
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            if !initial_compile_requested {
                self.start_compile();
                initial_compile_requested = true;
            }
            if let Some(action) =
                input::resolve(event::read(&self.internal_events)?, self.quit_confirmation)
            {
                self.update(action);
            }
        }

        Ok(())
    }

    fn update(&mut self, action: Action) {
        match action {
            Action::Save => self.save(),
            Action::Recompile => self.start_compile(),
            Action::CompileFinished(result) => self.finish_compile(result),
            Action::Tick => self.tick(),
            Action::ToggleDiagnostics => self.diagnostics.toggle(),
            Action::NavigateDiagnostic(direction) => self.navigate_diagnostic(direction),
            Action::Click { column, row } => self.click_preview(column, row),
            Action::ScrollAt { column, row, lines } if self.preview.contains(column, row) => {
                self.preview.scroll_lines(lines);
                self.focus = Pane::Preview;
            }
            Action::SwitchFocus => {
                self.focus = match self.focus {
                    Pane::Editor => Pane::Preview,
                    Pane::Preview => Pane::Editor,
                };
            }
            Action::ScrollPreviewPages(pages) => self.preview.scroll_pages(pages),
            Action::Move(Motion::Up) if self.focus == Pane::Preview => {
                self.preview.scroll_lines(-1);
            }
            Action::Move(Motion::Down) if self.focus == Pane::Preview => {
                self.preview.scroll_lines(1);
            }
            Action::RequestQuit if self.document.is_dirty() => {
                self.quit_confirmation = true;
                self.status = None;
            }
            Action::RequestQuit | Action::Quit => self.should_quit = true,
            Action::CancelQuit => self.quit_confirmation = false,
            action if self.focus == Pane::Editor => self.update_editor(action),
            _ => {}
        }
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
        self.start_compile_if_due();
        if self
            .cursor_sync_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.cursor_sync_deadline = None;
            self.sync_cursor_to_preview();
        }
    }

    fn start_compile_if_due(&mut self) {
        if self.compile_debounce.take_due(Instant::now()) {
            self.start_compile();
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
            } => {
                self.preview.replace_pages(pages);
                self.diagnostics.set_items(diagnostics);
                self.document_sync = Some((result.revision, sync));
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

    fn save(&mut self) {
        let Some(path) = self.path.as_deref() else {
            self.status = Some("Cannot save an untitled buffer".to_owned());
            return;
        };

        match fs::write(path, self.document.text()) {
            Ok(()) => {
                self.document.mark_saved();
                self.status = Some("Saved".to_owned());
            }
            Err(error) => self.status = Some(format!("Save failed: {error}")),
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [header_area, content_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let dirty_marker = if self.document.is_dirty() { " *" } else { "" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" typst-tui ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("{}{}", self.display_name, dirty_marker)),
                Span::styled(
                    format!(" | {}", self.compile_label()),
                    Style::default().fg(self.compile_color()),
                ),
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

        let preview_dimmed = self
            .document_sync
            .as_ref()
            .is_some_and(|(revision, _)| *revision != self.document.revision());
        if workspace_area.width < NARROW_WIDTH {
            self.preview.set_viewport(workspace_area);
            match self.focus {
                Pane::Editor => {
                    self.preview.hide();
                    self.editor.draw(
                        frame,
                        workspace_area,
                        &self.document,
                        &self.diagnostics,
                        true,
                    );
                }
                Pane::Preview => {
                    self.preview
                        .draw(frame, workspace_area, true, preview_dimmed);
                }
            }
        } else {
            let [editor_area, preview_area] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(workspace_area);
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
            );
        }
        if let Some(area) = diagnostics_area {
            self.diagnostics.draw(frame, area);
        }

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
                Color::Yellow,
            )
        } else if let Some(status) = &self.status {
            (status.clone(), Color::White)
        } else {
            (
                format!("Ln {}, Col {}", cursor.line + 1, cursor.column + 1),
                Color::Gray,
            )
        };
        let diagnostic_color = if errors > 0 {
            Color::Red
        } else if warnings > 0 {
            Color::Yellow
        } else {
            Color::Gray
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {status} | "), Style::default().fg(foreground)),
                Span::styled(
                    format!("{errors} errors, {warnings} warnings"),
                    Style::default().fg(diagnostic_color),
                ),
                Span::styled(
                    format!(" | {compile_time}"),
                    Style::default().fg(Color::Gray),
                ),
            ]))
            .style(Style::default().bg(Color::DarkGray)),
            status_area,
        );
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
            CompileState::Failed(_) | CompileState::Error => Color::Red,
            CompileState::Stale | CompileState::Compiling => Color::Yellow,
            CompileState::Ready => Color::Green,
            CompileState::NotStarted => Color::Gray,
        }
    }
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
