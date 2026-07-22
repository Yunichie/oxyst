use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    sync::mpsc::Sender,
};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind};

use crate::event::Event as AppEvent;

pub(crate) struct ProjectWatcher {
    _watcher: RecommendedWatcher,
    sender: Sender<AppEvent>,
}

impl ProjectWatcher {
    pub(crate) fn new(
        root: &Path,
        main: Option<&Path>,
        sender: Sender<AppEvent>,
    ) -> Result<Self, String> {
        let root = fs::canonicalize(root)
            .map_err(|error| format!("could not watch project root {}: {error}", root.display()))?;
        let main = main.map(|path| absolute_path(path, &root));
        let callback_sender = sender.clone();
        let callback_root = root.clone();
        let mut watcher = notify::recommended_watcher(move |result| {
            forward(result, &callback_root, main.as_deref(), &callback_sender);
        })
        .map_err(|error| format!("could not initialize project watcher: {error}"))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| format!("could not watch project root {}: {error}", root.display()))?;

        Ok(Self {
            _watcher: watcher,
            sender,
        })
    }

    pub(crate) fn retarget(&mut self, root: &Path, main: Option<&Path>) -> Result<(), String> {
        *self = Self::new(root, main, self.sender.clone())?;
        Ok(())
    }
}

fn forward(
    result: notify::Result<notify::Event>,
    root: &Path,
    main: Option<&Path>,
    sender: &Sender<AppEvent>,
) {
    match result {
        Ok(event) if event_requires_compile(&event, root, main) => {
            let _ = sender.send(AppEvent::ProjectFilesChanged);
        }
        Ok(_) => {}
        Err(error) => {
            let _ = sender.send(AppEvent::FileWatchFailed(error.to_string()));
        }
    }
}

fn event_requires_compile(event: &notify::Event, root: &Path, main: Option<&Path>) -> bool {
    if event.need_rescan() {
        return true;
    }
    let changes_content = match event.kind {
        EventKind::Any | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Other => true,
        EventKind::Modify(ModifyKind::Metadata(_)) | EventKind::Access(_) => false,
        EventKind::Modify(_) => true,
    };
    changes_content
        && event
            .paths
            .iter()
            .any(|path| is_project_resource(path, root, main))
}

fn is_project_resource(path: &Path, root: &Path, main: Option<&Path>) -> bool {
    let path = absolute_path(path, root);
    if main.is_some_and(|main| path == main) {
        return false;
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    !relative.components().next().is_some_and(|component| {
        matches!(component, Component::Normal(name) if name == ".git" || name == "target")
    })
}

fn absolute_path(path: &Path, root: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| root.join(path))
    };
    fs::canonicalize(&absolute).unwrap_or(absolute)
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs,
        sync::mpsc::channel,
        time::{Duration, SystemTime},
    };

    use notify::{
        Event, EventKind,
        event::{AccessKind, AccessMode, DataChange, Flag, MetadataKind, ModifyKind},
    };

    use super::{ProjectWatcher, event_requires_compile};
    use crate::event::Event as AppEvent;

    #[test]
    fn filters_main_file_noise_and_paths_outside_the_project() -> Result<(), Box<dyn Error>> {
        let root = std::env::current_dir()?.join("watch-filter-root");
        let main = root.join("main.typ");
        let changed = |path| {
            Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(path)
        };

        assert!(event_requires_compile(
            &changed(root.join("included.typ")),
            &root,
            Some(&main)
        ));
        assert!(!event_requires_compile(
            &changed(main),
            &root,
            Some(&root.join("main.typ"))
        ));
        assert!(!event_requires_compile(
            &changed(root.join("target/generated.typ")),
            &root,
            None
        ));
        assert!(!event_requires_compile(
            &changed(root.join(".git/index")),
            &root,
            None
        ));
        assert!(!event_requires_compile(
            &changed(
                root.parent()
                    .ok_or("root has no parent")?
                    .join("outside.typ")
            ),
            &root,
            None
        ));

        let access = Event::new(EventKind::Access(AccessKind::Open(AccessMode::Read)))
            .add_path(root.join("included.typ"));
        let metadata = Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Permissions,
        )))
        .add_path(root.join("included.typ"));
        assert!(!event_requires_compile(&access, &root, None));
        assert!(!event_requires_compile(&metadata, &root, None));

        let rescan = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert!(event_requires_compile(&rescan, &root, None));
        Ok(())
    }

    #[test]
    fn forwards_real_project_file_changes() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("typst-tui-watch-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root)?;
        let (sender, receiver) = channel();
        let watcher = ProjectWatcher::new(&root, None, sender).map_err(std::io::Error::other)?;

        fs::write(root.join("included.typ"), "watched")?;
        let event = receiver.recv_timeout(Duration::from_secs(5))?;
        assert!(matches!(event, AppEvent::ProjectFilesChanged));

        drop(watcher);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
