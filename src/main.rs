#![forbid(unsafe_code)]

use std::{fs, io::ErrorKind, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Typst source file to edit
    file: Option<PathBuf>,
    /// Project root used to resolve imports and assets
    #[arg(long)]
    root: Option<PathBuf>,
    /// Color theme: dark or light
    #[arg(long)]
    theme: Option<String>,
    /// Load configuration from this TOML file
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let text = load(&cli.file)?;
    let root_is_explicit = cli.root.is_some();
    let root = project_root(cli.root, cli.file.as_deref())?;
    let mut config = oxyst_config::Config::load(cli.config.as_deref())?;
    if let Some(theme) = cli.theme {
        config.set_theme(theme);
    }
    oxyst_app::run(cli.file, root, root_is_explicit, &text, config)?;
    Ok(())
}

fn project_root(root: Option<PathBuf>, file: Option<&std::path::Path>) -> Result<PathBuf> {
    if let Some(root) = root {
        return Ok(root);
    }
    if let Some(parent) = file.and_then(std::path::Path::parent)
        && !parent.as_os_str().is_empty()
    {
        return Ok(parent.to_owned());
    }
    std::env::current_dir().context("failed to determine current directory")
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
