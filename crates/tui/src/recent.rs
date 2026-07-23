use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const MAX_RECENT_FILES: usize = 10;

#[derive(Default, Deserialize, Serialize)]
struct RecentData {
    paths: Vec<PathBuf>,
}

pub(crate) struct RecentFiles {
    storage_path: Option<PathBuf>,
    entries: Vec<PathBuf>,
}

impl RecentFiles {
    pub(crate) fn load_default() -> (Self, Option<String>) {
        let Some(path) = state_path() else {
            return (Self::disabled(), None);
        };
        match Self::load_from(path.clone()) {
            Ok(recent) => (recent, None),
            Err(error) => (
                Self {
                    storage_path: Some(path),
                    entries: Vec::new(),
                },
                Some(error),
            ),
        }
    }

    pub(crate) fn disabled() -> Self {
        Self {
            storage_path: None,
            entries: Vec::new(),
        }
    }

    pub(crate) fn entries(&self) -> &[PathBuf] {
        &self.entries
    }

    pub(crate) fn record(&mut self, path: &Path) -> Result<(), String> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| format!("could not record recent file {}: {error}", path.display()))?;
        if !canonical.is_file() {
            return Err(format!(
                "could not record recent file {}: not a file",
                canonical.display()
            ));
        }
        self.entries.retain(|entry| entry != &canonical);
        self.entries.insert(0, canonical);
        self.entries.truncate(MAX_RECENT_FILES);
        self.persist()
    }

    fn load_from(storage_path: PathBuf) -> Result<Self, String> {
        let source = match fs::read_to_string(&storage_path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(format!(
                    "could not read recent files {}: {error}",
                    storage_path.display()
                ));
            }
        };
        let data = if source.is_empty() {
            RecentData::default()
        } else {
            toml::from_str(&source).map_err(|error| {
                format!(
                    "could not parse recent files {}: {error}",
                    storage_path.display()
                )
            })?
        };
        let mut entries = Vec::new();
        for path in data.paths {
            let Ok(path) = fs::canonicalize(path) else {
                continue;
            };
            if path.is_file() && !entries.contains(&path) {
                entries.push(path);
            }
            if entries.len() == MAX_RECENT_FILES {
                break;
            }
        }
        Ok(Self {
            storage_path: Some(storage_path),
            entries,
        })
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.storage_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create recent-files directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let source = toml::to_string(&RecentData {
            paths: self.entries.clone(),
        })
        .map_err(|error| format!("could not encode recent files: {error}"))?;
        fs::write(path, source)
            .map_err(|error| format!("could not write recent files {}: {error}", path.display()))
    }
}

fn state_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return Some(PathBuf::from(path).join("oxyst/recent.toml"));
    }
    if let Some(path) = env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(path).join("oxyst/recent.toml"));
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".local/state/oxyst/recent.toml"))
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::PathBuf};

    use super::{MAX_RECENT_FILES, RecentFiles};

    #[test]
    fn persists_deduplicates_bounds_and_prunes_recent_files() -> Result<(), Box<dyn Error>> {
        let temp = tempfile_dir("recent");
        fs::create_dir_all(&temp)?;
        let storage = temp.join("recent.toml");
        let mut recent = RecentFiles::load_from(storage.clone()).map_err(std::io::Error::other)?;
        for index in 0..=MAX_RECENT_FILES {
            let path = temp.join(format!("{index}.typ"));
            fs::write(&path, "text")?;
            recent.record(&path).map_err(std::io::Error::other)?;
        }
        let newest = temp.join(format!("{MAX_RECENT_FILES}.typ"));
        recent.record(&newest).map_err(std::io::Error::other)?;
        assert_eq!(recent.entries().len(), MAX_RECENT_FILES);
        assert_eq!(recent.entries().first(), Some(&fs::canonicalize(&newest)?));

        fs::remove_file(temp.join("5.typ"))?;
        let loaded = RecentFiles::load_from(storage).map_err(std::io::Error::other)?;
        assert_eq!(loaded.entries().len(), MAX_RECENT_FILES - 1);
        assert!(!loaded.entries().iter().any(|path| path.ends_with("5.typ")));

        fs::remove_dir_all(temp)?;
        Ok(())
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("oxyst-{label}-{}-{unique}", std::process::id()))
    }
}
