#![forbid(unsafe_code)]

mod action;
mod app;
mod clipboard;
mod components;
mod event;
mod explorer;
mod export;
mod input;
mod message;
mod recent;
mod style;
mod watcher;
mod workspace;

use std::{io, io::ErrorKind, path::PathBuf, time::Duration};

use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
};
use ratatui_image::picker::Picker;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not initialize Typst compiler")]
    Compiler(#[from] oxyst_compiler::Error),
    #[error("terminal I/O failed")]
    Terminal(#[from] std::io::Error),
    #[error("invalid application configuration: {0}")]
    Configuration(String),
}

pub fn run(
    path: Option<PathBuf>,
    root: PathBuf,
    root_is_explicit: bool,
    text: &str,
    config: oxyst_config::Config,
) -> Result<(), Error> {
    let main = path.clone().unwrap_or_else(|| root.join("untitled.typ"));
    let compiler = oxyst_compiler::Compiler::new(&root, main)?;
    let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
    let mut terminal = ratatui::try_init()?;
    let picker = match Picker::from_query_stdio() {
        Ok(picker) => picker,
        Err(_) => Picker::halfblocks(),
    };
    let (mut recent, mut startup_status) = recent::RecentFiles::load_default();
    if let Some(path) = path.as_deref().filter(|path| path.is_file())
        && let Err(error) = recent.record(path)
    {
        startup_status = Some(error);
    }
    let bracketed_paste = match enable_input_modes() {
        Ok(bracketed_paste) => bracketed_paste,
        Err(error) => {
            let _ = ratatui::try_restore();
            runtime.shutdown_timeout(Duration::from_millis(100));
            return Err(Error::Terminal(error));
        }
    };
    let mut app = match app::App::new(app::AppInit {
        path,
        root,
        root_is_explicit,
        text,
        compiler,
        picker,
        runtime: runtime.handle().clone(),
        config,
        recent,
        startup_status,
    }) {
        Ok(app) => app,
        Err(error) => {
            let _ = disable_input_modes(bracketed_paste);
            let _ = ratatui::try_restore();
            runtime.shutdown_timeout(Duration::from_millis(100));
            return Err(Error::Configuration(error));
        }
    };
    let run_result = app.run(&mut terminal);
    let input_result = disable_input_modes(bracketed_paste);
    let restore_result = ratatui::try_restore();
    runtime.shutdown_timeout(Duration::from_millis(100));

    run_result?;
    input_result?;
    restore_result?;
    Ok(())
}

fn enable_input_modes() -> io::Result<bool> {
    execute!(io::stdout(), EnableMouseCapture)?;
    match execute!(io::stdout(), EnableBracketedPaste) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == ErrorKind::Unsupported => Ok(false),
        Err(error) => {
            let _ = execute!(io::stdout(), DisableMouseCapture);
            Err(error)
        }
    }
}

fn disable_input_modes(bracketed_paste: bool) -> io::Result<()> {
    let paste_result = if bracketed_paste {
        execute!(io::stdout(), DisableBracketedPaste)
    } else {
        Ok(())
    };
    let mouse_result = execute!(io::stdout(), DisableMouseCapture);
    paste_result.and(mouse_result)
}
