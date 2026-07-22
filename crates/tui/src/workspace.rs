use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use typst_tui_render::ExportFormat;

pub(crate) struct OpenedSource {
    pub(crate) text: String,
}

pub(crate) struct Workspace {
    path: Option<PathBuf>,
    root: PathBuf,
    display_name: String,
}

impl Workspace {
    pub(crate) fn new(path: Option<PathBuf>, root: PathBuf) -> Self {
        let display_name = display_name(path.as_deref());
        Self {
            path,
            root,
            display_name,
        }
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }

    pub(crate) fn main_path(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| self.root.join("untitled.typ"))
    }

    pub(crate) fn resolve_path(&self, value: &str) -> Result<PathBuf, String> {
        if value.is_empty() {
            return Err("A path is required".to_owned());
        }
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Ok(path);
        }
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("Could not resolve path: {error}"))
    }

    pub(crate) fn read_source(path: &Path) -> Result<OpenedSource, String> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(OpenedSource { text }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(OpenedSource {
                text: String::new(),
            }),
            Err(error) => Err(format!("Open failed: {error}")),
        }
    }

    pub(crate) fn write_source(path: &Path, text: &str) -> Result<(), String> {
        fs::write(path, text).map_err(|error| format!("Save failed: {error}"))
    }

    pub(crate) fn root_for_open(&self, path: &Path) -> PathBuf {
        if path.starts_with(&self.root) {
            self.root.clone()
        } else {
            path.parent()
                .map_or_else(|| self.root.clone(), Path::to_owned)
        }
    }

    pub(crate) fn opened(&mut self, path: PathBuf, root: PathBuf) {
        self.display_name = display_name(Some(&path));
        self.path = Some(path);
        self.root = root;
    }

    pub(crate) fn saved_as(&mut self, path: PathBuf) {
        self.root = path
            .parent()
            .map_or_else(|| self.root.clone(), Path::to_owned);
        self.display_name = display_name(Some(&path));
        self.path = Some(path);
    }

    pub(crate) fn default_export_path(&self, format: ExportFormat) -> PathBuf {
        let mut path = self.main_path();
        path.set_extension(format.extension());
        path
    }
}

fn display_name(path: Option<&Path>) -> String {
    path.and_then(Path::file_name).map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::Workspace;

    #[test]
    fn updates_root_and_display_name_for_opened_files() {
        let root = PathBuf::from("project");
        let mut workspace = Workspace::new(None, root.clone());
        assert_eq!(workspace.display_name(), "Untitled");
        assert_eq!(workspace.root_for_open(Path::new("project/main.typ")), root);

        workspace.opened(PathBuf::from("other/report.typ"), PathBuf::from("other"));
        assert_eq!(workspace.display_name(), "report.typ");
        assert_eq!(workspace.root(), Path::new("other"));
    }
}
