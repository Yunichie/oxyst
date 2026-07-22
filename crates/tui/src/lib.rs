#![forbid(unsafe_code)]

mod action;
mod app;
mod clipboard;
mod compile;
mod components;
mod event;
mod export;
mod input;
mod recent;
mod style;
mod watcher;
mod workspace;

use std::{io, path::PathBuf, time::Duration};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not initialize Typst compiler")]
    Compiler(#[from] typst_tui_compiler::Error),
    #[error("terminal I/O failed")]
    Terminal(#[from] std::io::Error),
    #[error("invalid application configuration: {0}")]
    Configuration(String),
}

pub fn run(
    path: Option<PathBuf>,
    root: PathBuf,
    text: &str,
    config: typst_tui_config::Config,
) -> Result<(), Error> {
    let main = path.clone().unwrap_or_else(|| root.join("untitled.typ"));
    let compiler = typst_tui_compiler::Compiler::new(&root, main)?;
    let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
    let mut terminal = ratatui::try_init()?;
    let picker = match ratatui_image::picker::Picker::from_query_stdio() {
        Ok(picker) => picker,
        Err(_) => ratatui_image::picker::Picker::halfblocks(),
    };
    let (mut recent, mut startup_status) = recent::RecentFiles::load_default();
    if let Some(path) = path.as_deref().filter(|path| path.is_file())
        && let Err(error) = recent.record(path)
    {
        startup_status = Some(error);
    }
    if let Err(error) = execute!(io::stdout(), EnableMouseCapture) {
        let _ = ratatui::try_restore();
        runtime.shutdown_timeout(Duration::from_millis(100));
        return Err(Error::Terminal(error));
    }
    let mut app = match app::App::new(app::AppInit {
        path,
        root,
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
            let _ = execute!(io::stdout(), DisableMouseCapture);
            let _ = ratatui::try_restore();
            runtime.shutdown_timeout(Duration::from_millis(100));
            return Err(Error::Configuration(error));
        }
    };
    let run_result = app.run(&mut terminal);
    let mouse_result = execute!(io::stdout(), DisableMouseCapture);
    let restore_result = ratatui::try_restore();
    runtime.shutdown_timeout(Duration::from_millis(100));

    run_result?;
    mouse_result?;
    restore_result?;
    Ok(())
}
