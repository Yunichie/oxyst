#![forbid(unsafe_code)]

mod action;
mod app;
mod components;
mod event;
mod input;

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("terminal I/O failed")]
    Terminal(#[from] std::io::Error),
}

pub fn run(path: Option<PathBuf>, text: &str) -> Result<(), Error> {
    let mut terminal = ratatui::try_init()?;
    let mut app = app::App::new(path, text);
    let run_result = app.run(&mut terminal);
    let restore_result = ratatui::try_restore();

    run_result?;
    restore_result?;
    Ok(())
}
