use std::{
    fs, io,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_kit::{
    datetime::Time,
    downloader::SystemDownloader,
    files::{FileLoader, FileStore},
    fonts::{self, FontStore},
    packages::SystemPackages,
};

use crate::Error;

pub(crate) struct TypstWorld {
    resources: Arc<WorldResources>,
    files: Arc<FileStore<SecureFiles>>,
    main: FileId,
    main_source: Source,
    now: Time,
}

struct WorldResources {
    root: PathBuf,
    library: LazyHash<Library>,
    fonts: FontStore,
    packages: Arc<SystemPackages>,
}

impl TypstWorld {
    pub(crate) fn new(root: &Path, main: &Path) -> Result<Self, Error> {
        let root = fs::canonicalize(root).map_err(|source| Error::ProjectRoot {
            path: root.to_owned(),
            source,
        })?;
        let main = resolve_main(main)?;
        if !main.starts_with(&root) {
            return Err(Error::MainOutsideRoot { main, root });
        }

        let vpath =
            VirtualPath::virtualize(&root, &main).map_err(|source| Error::InvalidMainPath {
                path: main.clone(),
                source,
            })?;
        let main = RootedPath::new(VirtualRoot::Project, vpath).intern();
        let mut font_store = FontStore::new();
        font_store.extend(fonts::system());
        font_store.extend(fonts::embedded());

        let packages = Arc::new(SystemPackages::new(SystemDownloader::new(concat!(
            "typst-tui/",
            env!("CARGO_PKG_VERSION")
        ))));
        let resources = Arc::new(WorldResources {
            root,
            library: LazyHash::new(Library::default()),
            fonts: font_store,
            packages,
        });

        Ok(Self {
            files: Arc::new(FileStore::new(SecureFiles::new(Arc::clone(&resources)))),
            resources,
            main,
            main_source: Source::new(main, String::new()),
            now: Time::system(),
        })
    }

    pub(crate) fn replace_source(&mut self, text: &str) {
        self.main_source.replace(text);
    }

    pub(crate) fn edit_source(&mut self, range: Range<usize>, replacement: &str) {
        self.main_source.edit(range, replacement);
    }

    pub(crate) fn snapshot(&self) -> Self {
        Self {
            resources: Arc::clone(&self.resources),
            files: Arc::clone(&self.files),
            main: self.main,
            main_source: self.main_source.clone(),
            now: Time::system(),
        }
    }

    pub(crate) fn main_source(&self) -> Source {
        self.main_source.clone()
    }

    pub(crate) fn source_text(&self) -> &str {
        self.main_source.text()
    }

    pub(crate) fn invalidate_files(&mut self) {
        self.files = Arc::new(FileStore::new(SecureFiles::new(Arc::clone(
            &self.resources,
        ))));
    }
}

impl World for TypstWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.resources.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.resources.fonts.book()
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.main_source.clone())
        } else {
            self.files.source(id)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main {
            Ok(Bytes::from_string(self.main_source.clone()))
        } else {
            self.files.file(id)
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.resources.fonts.font(index)
    }

    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        self.now.today(offset)
    }
}

fn resolve_main(main: &Path) -> Result<PathBuf, Error> {
    let absolute = if main.is_absolute() {
        main.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|source| Error::MainFile {
                path: main.to_owned(),
                source,
            })?
            .join(main)
    };

    match fs::canonicalize(&absolute) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = absolute.parent().unwrap_or(Path::new("."));
            let parent = fs::canonicalize(parent).map_err(|source| Error::MainFile {
                path: absolute.clone(),
                source,
            })?;
            let Some(name) = absolute.file_name() else {
                return Err(Error::MainFile {
                    path: absolute,
                    source: error,
                });
            };
            Ok(parent.join(name))
        }
        Err(source) => Err(Error::MainFile {
            path: absolute,
            source,
        }),
    }
}

struct SecureFiles {
    resources: Arc<WorldResources>,
}

impl SecureFiles {
    fn new(resources: Arc<WorldResources>) -> Self {
        Self { resources }
    }

    fn load_project(&self, vpath: &VirtualPath) -> FileResult<Bytes> {
        let lexical = vpath.realize(&self.resources.root)?;
        let canonical =
            fs::canonicalize(&lexical).map_err(|error| FileError::from_io(error, &lexical))?;
        if !canonical.starts_with(&self.resources.root) {
            return Err(FileError::AccessDenied);
        }

        let metadata =
            fs::metadata(&canonical).map_err(|error| FileError::from_io(error, &canonical))?;
        if metadata.is_dir() {
            return Err(FileError::IsDirectory);
        }

        fs::read(&canonical)
            .map(Bytes::new)
            .map_err(|error| FileError::from_io(error, &canonical))
    }
}

impl FileLoader for SecureFiles {
    fn load(&self, id: FileId) -> FileResult<Bytes> {
        match id.root() {
            VirtualRoot::Project => self.load_project(id.vpath()),
            VirtualRoot::Package(package) => {
                self.resources.packages.obtain(package)?.load(id.vpath())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf, sync::Arc};

    use super::TypstWorld;

    #[test]
    fn snapshots_reuse_files_until_resource_invalidation() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut world = TypstWorld::new(&root, &root.join("simple.typ"))?;
        let first = world.snapshot();
        let second = world.snapshot();
        assert!(Arc::ptr_eq(&first.files, &second.files));

        world.invalidate_files();
        let invalidated = world.snapshot();
        assert!(!Arc::ptr_eq(&first.files, &invalidated.files));
        Ok(())
    }
}
