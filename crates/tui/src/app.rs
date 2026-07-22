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
use typst_tui_compiler::{Compiler, Diagnostic, Severity};
use typst_tui_document::{Document, Motion};

use crate::{
    action::{Action, Pane},
    compile::{CompileResult, CompileResultKind, CompileWorker},
    components::{Editor, Preview},
    event::{self, Event},
    input,
};

const NARROW_WIDTH: u16 = 80;
const AUTO_COMPILE_DELAY: Duration = Duration::from_millis(150);

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
    compile_state: CompileState,
    diagnostics: Vec<Diagnostic>,
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
            compile_state: CompileState::NotStarted,
            diagnostics: Vec::new(),
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
            Action::Tick => self.start_compile_if_due(),
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
        self.editor.update(&action, &mut self.document);
        if self.document.revision() != revision {
            self.compile_generation = self.compile_worker.invalidate();
            self.compile_debounce.schedule(Instant::now());
            self.compile_state = CompileState::Stale;
        }
        self.status = None;
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
            CompileResultKind::Success { pages, diagnostics } => {
                self.preview.replace_pages(pages);
                self.diagnostics = diagnostics;
                self.compile_state = CompileState::Ready;
                self.status = None;
            }
            CompileResultKind::Diagnostics(diagnostics) => {
                let errors = diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.severity == Severity::Error)
                    .count();
                self.status = diagnostics.first().map(format_diagnostic);
                self.diagnostics = diagnostics;
                self.compile_state = CompileState::Failed(errors);
            }
            CompileResultKind::Error(error) => {
                self.compile_state = CompileState::Error;
                self.status = Some(format!("Preview failed: {error}"));
            }
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

        if content_area.width < NARROW_WIDTH {
            self.preview.set_viewport(content_area);
            match self.focus {
                Pane::Editor => self.editor.draw(frame, content_area, &self.document, true),
                Pane::Preview => self.preview.draw(frame, content_area, true),
            }
        } else {
            let [editor_area, preview_area] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(content_area);
            self.editor.draw(
                frame,
                editor_area,
                &self.document,
                self.focus == Pane::Editor,
            );
            self.preview
                .draw(frame, preview_area, self.focus == Pane::Preview);
        }

        let cursor = self.document.cursor_position();
        let errors = self
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count();
        let warnings = self.diagnostics.len().saturating_sub(errors);
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
                format!(
                    "Ln {}, Col {} | {errors} errors, {warnings} warnings | {compile_time}",
                    cursor.line + 1,
                    cursor.column + 1,
                ),
                Color::Gray,
            )
        };
        frame.render_widget(
            Paragraph::new(format!(" {status}"))
                .style(Style::default().fg(foreground).bg(Color::DarkGray)),
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

fn format_diagnostic(diagnostic: &Diagnostic) -> String {
    match (&diagnostic.path, diagnostic.line, diagnostic.column) {
        (Some(path), Some(line), Some(column)) => {
            format!("{path}:{}:{}: {}", line + 1, column + 1, diagnostic.message)
        }
        _ => diagnostic.message.clone(),
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
