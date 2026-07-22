#![forbid(unsafe_code)]

use std::{fs, io::ErrorKind, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Typst source file to edit
    file: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let text = load(&cli.file)?;
    typst_tui_app::run(cli.file, &text)?;
    Ok(())
}

fn load(path: &Option<PathBuf>) -> Result<String> {
    let Some(path) = path else {
        return Ok(String::new());
    };

    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}
