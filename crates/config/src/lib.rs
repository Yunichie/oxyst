#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

mod command;

pub use command::{CommandContext, CommandId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub theme: String,
    pub soft_wrap: bool,
    pub keys: BTreeMap<String, Vec<String>>,
}

impl Config {
    pub fn load(explicit_path: Option<&Path>) -> Result<Self, Error> {
        let environment_path = env::var_os("OXYST_CONFIG").map(PathBuf::from);
        let (path, required) = if let Some(path) = explicit_path {
            (Some(path.to_owned()), true)
        } else if let Some(path) = environment_path {
            (Some(path), true)
        } else {
            (default_path(), false)
        };
        let Some(path) = path else {
            return Ok(Self::default());
        };

        match fs::read_to_string(&path) {
            Ok(source) => parse(&source, &path),
            Err(error) if !required && error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read { path, source }),
        }
    }

    pub fn set_theme(&mut self, theme: impl Into<String>) {
        self.theme = theme.into();
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "dark".to_owned(),
            soft_wrap: false,
            keys: CommandId::all()
                .map(|command| {
                    (
                        command.name().to_owned(),
                        command
                            .default_bindings()
                            .iter()
                            .map(|binding| (*binding).to_owned())
                            .collect(),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to read config file {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("unknown keybinding action `{action}` in {path}")]
    UnknownAction { path: PathBuf, action: String },
    #[error("keybinding `{action}` in {path} contains an empty chord")]
    EmptyBinding { path: PathBuf, action: String },
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    theme: Option<String>,
    soft_wrap: Option<bool>,
    keys: BTreeMap<String, Vec<String>>,
}

fn parse(source: &str, path: &Path) -> Result<Config, Error> {
    let parsed: FileConfig = toml::from_str(source).map_err(|source| Error::Parse {
        path: path.to_owned(),
        source,
    })?;
    let mut config = Config::default();
    if let Some(theme) = parsed.theme {
        config.theme = theme;
    }
    if let Some(soft_wrap) = parsed.soft_wrap {
        config.soft_wrap = soft_wrap;
    }
    for (action, bindings) in parsed.keys {
        if CommandId::from_name(&action).is_none() {
            return Err(Error::UnknownAction {
                path: path.to_owned(),
                action,
            });
        }
        if bindings.iter().any(|binding| binding.trim().is_empty()) {
            return Err(Error::EmptyBinding {
                path: path.to_owned(),
                action,
            });
        }
        config.keys.insert(action, bindings);
    }
    Ok(config)
}

fn default_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(path).join("oxyst/config.toml"));
    }
    if let Some(path) = env::var_os("APPDATA") {
        return Some(PathBuf::from(path).join("oxyst/config.toml"));
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".config/oxyst/config.toml"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Config, Error, parse};

    #[test]
    fn partial_config_inherits_defaults() -> Result<(), Error> {
        let config = parse(
            "theme = \"light\"\nsoft_wrap = true\n[keys]\nsave = [\"alt+s\"]\n",
            Path::new("config.toml"),
        )?;

        assert_eq!(config.theme, "light");
        assert!(config.soft_wrap);
        assert_eq!(config.keys["save"], ["alt+s"]);
        assert_eq!(config.keys["quit"], Config::default().keys["quit"]);
        Ok(())
    }

    #[test]
    fn unknown_actions_are_rejected() {
        let result = parse(
            "[keys]\nteleport = [\"ctrl+t\"]\n",
            Path::new("config.toml"),
        );

        assert!(matches!(result, Err(Error::UnknownAction { .. })));
    }

    #[test]
    fn context_aware_commands_include_printable_defaults() {
        let config = Config::default();

        assert_eq!(config.keys["command_palette"], ["ctrl+shift+p", ":"]);
        assert_eq!(config.keys["help"], ["f1", "?"]);
    }
}
