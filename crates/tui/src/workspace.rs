use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use typst_tui_render::ExportFormat;

pub(crate) struct OpenedSource {
    pub(crate) text: String,
    pub(crate) existed: bool,
}

pub(crate) struct Workspace {
    path: Option<PathBuf>,
    root: PathBuf,
    root_is_explicit: bool,
    display_name: String,
}

impl Workspace {
    pub(crate) fn new(path: Option<PathBuf>, root: PathBuf, root_is_explicit: bool) -> Self {
        let display_name = display_name(path.as_deref());
        Self {
            path,
            root,
            root_is_explicit,
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
            Ok(text) => Ok(OpenedSource {
                text,
                existed: true,
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(OpenedSource {
                text: String::new(),
                existed: false,
            }),
            Err(error) => Err(format!("Open failed: {error}")),
        }
    }

    pub(crate) fn write_source(path: &Path, text: &str) -> Result<(), String> {
        fs::write(path, text).map_err(|error| format!("Save failed: {error}"))
    }

    pub(crate) fn root_for_document(&self, path: &Path) -> Result<PathBuf, String> {
        let root = fs::canonicalize(&self.root).map_err(|error| {
            format!(
                "Could not resolve project root {}: {error}",
                self.root.display()
            )
        })?;
        let document = canonical_document_path(path)?;
        if document.starts_with(&root) {
            return Ok(root);
        }

        if self.root_is_explicit {
            return Err(format!(
                "{} is outside project root {}",
                path.display(),
                self.root.display()
            ));
        }

        document
            .parent()
            .map(Path::to_owned)
            .ok_or_else(|| format!("Could not determine project root for {}", path.display()))
    }

    pub(crate) fn opened(&mut self, path: PathBuf, root: PathBuf) {
        self.display_name = display_name(Some(&path));
        self.path = Some(path);
        self.root = root;
    }

    pub(crate) fn saved_as(&mut self, path: PathBuf, root: PathBuf) {
        self.root = root;
        self.display_name = display_name(Some(&path));
        self.path = Some(path);
    }

    pub(crate) fn default_export_path(&self, format: ExportFormat) -> PathBuf {
        let mut path = self.main_path();
        path.set_extension(format.extension());
        path
    }
}

fn canonical_document_path(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path)
            .map_err(|error| format!("Could not resolve {}: {error}", path.display()));
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("Could not resolve {}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("Could not determine file name for {}", path.display()))?;
    Ok(parent.join(name))
}

fn display_name(path: Option<&Path>) -> String {
    path.and_then(Path::file_name).map_or_else(
        || "Untitled".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::Workspace;

    #[test]
    fn explicit_roots_stay_fixed_and_inferred_roots_follow_documents() -> Result<(), Box<dyn Error>>
    {
        let base = temporary_directory("root-policy")?;
        let root = base.join("project");
        let nested = root.join("chapters");
        let outside = base.join("outside");
        fs::create_dir_all(&nested)?;
        fs::create_dir_all(&outside)?;
        let nested_document = nested.join("..").join("chapters").join("report.typ");
        let outside_document = outside.join("report.typ");
        let canonical_root = fs::canonicalize(&root)?;
        let canonical_outside = fs::canonicalize(&outside)?;

        let fixed = Workspace::new(None, root.clone(), true);
        assert_eq!(fixed.root_for_document(&nested_document)?, canonical_root);
        assert!(fixed.root_for_document(&outside_document).is_err());

        let mut inferred = Workspace::new(None, root, false);
        assert_eq!(inferred.display_name(), "Untitled");
        assert_eq!(
            inferred.root_for_document(&nested_document)?,
            canonical_root
        );
        assert_eq!(
            inferred.root_for_document(&outside_document)?,
            canonical_outside
        );

        inferred.saved_as(outside_document.clone(), canonical_outside.clone());
        assert_eq!(inferred.display_name(), "report.typ");
        assert_eq!(inferred.root(), canonical_outside);

        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn explicit_root_rejects_symlink_escapes_when_supported() -> Result<(), Box<dyn Error>> {
        let base = temporary_directory("symlink-policy")?;
        let root = base.join("project");
        let outside = base.join("outside.typ");
        fs::create_dir_all(&root)?;
        fs::write(&outside, "= Outside")?;
        let link = root.join("linked.typ");
        let linked = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&outside, &link)
            }
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_file(&outside, &link)
            }
        };
        if linked.is_ok() {
            let workspace = Workspace::new(None, root, true);
            assert!(workspace.root_for_document(&link).is_err());
        }

        fs::remove_dir_all(base)?;
        Ok(())
    }

    fn temporary_directory(label: &str) -> Result<PathBuf, Box<dyn Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "typst-tui-workspace-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(path)
    }
}
