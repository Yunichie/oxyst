#![forbid(unsafe_code)]

mod action;
mod app;
mod compile;
mod components;
mod event;
mod input;

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
}

pub fn run(path: Option<PathBuf>, root: PathBuf, text: &str) -> Result<(), Error> {
    let main = path.clone().unwrap_or_else(|| root.join("untitled.typ"));
    let compiler = typst_tui_compiler::Compiler::new(&root, main)?;
    let runtime = tokio::runtime::Builder::new_multi_thread().build()?;
    let mut terminal = ratatui::try_init()?;
    let picker = match ratatui_image::picker::Picker::from_query_stdio() {
        Ok(picker) => picker,
        Err(_) => ratatui_image::picker::Picker::halfblocks(),
    };
    if let Err(error) = execute!(io::stdout(), EnableMouseCapture) {
        let _ = ratatui::try_restore();
        runtime.shutdown_timeout(Duration::from_millis(100));
        return Err(Error::Terminal(error));
    }
    let mut app = app::App::new(path, text, compiler, picker, runtime.handle().clone());
    let run_result = app.run(&mut terminal);
    let mouse_result = execute!(io::stdout(), DisableMouseCapture);
    let restore_result = ratatui::try_restore();
    runtime.shutdown_timeout(Duration::from_millis(100));

    run_result?;
    mouse_result?;
    restore_result?;
    Ok(())
}
