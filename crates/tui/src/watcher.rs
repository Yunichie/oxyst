use std::{
    collections::BTreeSet,
    env, fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, mpsc::Sender},
};

use notify::{
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind, RenameMode},
};

use crate::event::Event as AppEvent;

pub(crate) struct ProjectWatcher {
    _watcher: RecommendedWatcher,
    sender: Sender<AppEvent>,
    pending: Arc<Mutex<PendingProjectChanges>>,
}

#[derive(Default)]
pub(crate) struct ProjectChanges {
    paths: BTreeSet<PathBuf>,
    rescan: bool,
}

impl ProjectChanges {
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        self.paths.iter().cloned().collect()
    }

    pub(crate) fn requires_rescan(&self) -> bool {
        self.rescan
    }
}

#[derive(Default)]
struct PendingProjectChanges {
    changes: ProjectChanges,
    notified: bool,
}

impl ProjectWatcher {
    pub(crate) fn new(root: &Path, sender: Sender<AppEvent>) -> Result<Self, String> {
        let root = fs::canonicalize(root)
            .map_err(|error| format!("could not watch project root {}: {error}", root.display()))?;
        let callback_sender = sender.clone();
        let callback_root = root.clone();
        let pending = Arc::new(Mutex::new(PendingProjectChanges::default()));
        let callback_pending = Arc::clone(&pending);
        let mut watcher = notify::recommended_watcher(move |result| {
            forward(result, &callback_root, &callback_sender, &callback_pending);
        })
        .map_err(|error| format!("could not initialize project watcher: {error}"))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|error| format!("could not watch project root {}: {error}", root.display()))?;

        Ok(Self {
            _watcher: watcher,
            sender,
            pending,
        })
    }

    pub(crate) fn retarget(&mut self, root: &Path) -> Result<(), String> {
        *self = Self::new(root, self.sender.clone())?;
        Ok(())
    }

    pub(crate) fn take_project_changes(&self) -> Result<ProjectChanges, String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "project watcher change queue is unavailable".to_owned())?;
        pending.notified = false;
        Ok(std::mem::take(&mut pending.changes))
    }
}

fn forward(
    result: notify::Result<notify::Event>,
    root: &Path,
    sender: &Sender<AppEvent>,
    pending: &Mutex<PendingProjectChanges>,
) {
    match result {
        Ok(event) if event_requires_compile(&event, root) => {
            let should_notify = match pending.lock() {
                Ok(mut pending) => {
                    merge_explorer_changes(&event, root, &mut pending.changes);
                    if pending.notified {
                        false
                    } else {
                        pending.notified = true;
                        true
                    }
                }
                Err(_) => {
                    let _ = sender.send(AppEvent::FileWatchFailed(
                        "project watcher change queue is unavailable".to_owned(),
                    ));
                    false
                }
            };
            if should_notify
                && sender.send(AppEvent::ProjectFilesChanged).is_err()
                && let Ok(mut pending) = pending.lock()
            {
                pending.notified = false;
            }
        }
        Ok(_) => {}
        Err(error) => {
            let _ = sender.send(AppEvent::FileWatchFailed(error.to_string()));
        }
    }
}

fn merge_explorer_changes(event: &notify::Event, root: &Path, changes: &mut ProjectChanges) {
    if event.need_rescan() {
        changes.rescan = true;
        return;
    }
    match event.kind {
        EventKind::Create(CreateKind::File) | EventKind::Remove(RemoveKind::File) => {
            record_typst_paths(event, root, changes);
        }
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => {
            changes.rescan = true;
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::Both | RenameMode::To)) => {
            if rename_is_typst_file(event, root) {
                record_typst_paths(event, root, changes);
            } else {
                changes.rescan = true;
            }
        }
        EventKind::Modify(ModifyKind::Name(_)) => changes.rescan = true,
        EventKind::Create(CreateKind::Any | CreateKind::Other) => {
            for path in &event.paths {
                if is_typst_path(path) {
                    record_typst_path(path, root, changes);
                } else if match fs::symlink_metadata(absolute_path(path, root)) {
                    Ok(metadata) => !metadata.is_file(),
                    Err(_) => true,
                } {
                    changes.rescan = true;
                }
            }
        }
        EventKind::Remove(RemoveKind::Any | RemoveKind::Other) => {
            if event.paths.iter().all(|path| is_typst_path(path)) {
                record_typst_paths(event, root, changes);
            } else {
                changes.rescan = true;
            }
        }
        EventKind::Any | EventKind::Other => changes.rescan = true,
        EventKind::Access(_) | EventKind::Modify(_) => {}
    }
}

fn rename_is_typst_file(event: &notify::Event, root: &Path) -> bool {
    let mut found_file = false;
    for path in &event.paths {
        if !is_typst_path(path) {
            return false;
        }
        match fs::symlink_metadata(absolute_path(path, root)) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                found_file = true;
            }
            Ok(_) => return false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    found_file
}

fn record_typst_paths(event: &notify::Event, root: &Path, changes: &mut ProjectChanges) {
    for path in &event.paths {
        record_typst_path(path, root, changes);
    }
}

fn record_typst_path(path: &Path, root: &Path, changes: &mut ProjectChanges) {
    if let Some(path) = project_path(path, root)
        && is_typst_path(&path)
    {
        changes.paths.insert(path);
    }
}

fn event_requires_compile(event: &notify::Event, root: &Path) -> bool {
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
            .any(|path| is_project_resource(path, root))
}

fn is_project_resource(path: &Path, root: &Path) -> bool {
    project_path(path, root).is_some()
}

fn project_path(path: &Path, root: &Path) -> Option<PathBuf> {
    let path = absolute_path(path, root);
    let relative = path.strip_prefix(root).ok()?;
    (!relative.components().next().is_some_and(|component| {
        matches!(component, Component::Normal(name) if name == ".git" || name == "target")
    }))
    .then_some(path)
}

fn is_typst_path(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "typ")
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
        sync::{
            Mutex,
            mpsc::{TryRecvError, channel},
        },
        time::{Duration, Instant, SystemTime},
    };

    use notify::{
        Event, EventKind,
        event::{
            AccessKind, AccessMode, CreateKind, DataChange, Flag, MetadataKind, ModifyKind,
            RenameMode,
        },
    };

    use super::{
        PendingProjectChanges, ProjectChanges, ProjectWatcher, event_requires_compile, forward,
        merge_explorer_changes,
    };
    use crate::event::Event as AppEvent;

    #[test]
    fn accepts_open_files_and_filters_paths_outside_the_project() -> Result<(), Box<dyn Error>> {
        let root = std::env::current_dir()?.join("watch-filter-root");
        let main = root.join("main.typ");
        let changed = |path| {
            Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(path)
        };

        assert!(event_requires_compile(
            &changed(root.join("included.typ")),
            &root
        ));
        assert!(event_requires_compile(&changed(main), &root));
        assert!(!event_requires_compile(
            &changed(root.join("target/generated.typ")),
            &root
        ));
        assert!(!event_requires_compile(
            &changed(root.join(".git/index")),
            &root
        ));
        assert!(!event_requires_compile(
            &changed(
                root.parent()
                    .ok_or("root has no parent")?
                    .join("outside.typ")
            ),
            &root,
        ));

        let access = Event::new(EventKind::Access(AccessKind::Open(AccessMode::Read)))
            .add_path(root.join("included.typ"));
        let metadata = Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Permissions,
        )))
        .add_path(root.join("included.typ"));
        assert!(!event_requires_compile(&access, &root));
        assert!(!event_requires_compile(&metadata, &root));

        let rescan = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert!(event_requires_compile(&rescan, &root));
        Ok(())
    }

    #[test]
    fn coalesces_project_change_bursts_without_losing_paths() -> Result<(), Box<dyn Error>> {
        let root = std::env::current_dir()?;
        let changed = |index| {
            Event::new(EventKind::Create(CreateKind::File))
                .add_path(root.join(format!("included-{index}.typ")))
        };
        let (sender, receiver) = channel();
        let pending = Mutex::new(PendingProjectChanges::default());

        for index in 0..100 {
            forward(Ok(changed(index)), &root, &sender, &pending);
        }

        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1))?,
            AppEvent::ProjectFilesChanged
        ));
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));

        {
            let mut pending = pending
                .lock()
                .map_err(|_| "test watcher queue is unavailable")?;
            assert_eq!(pending.changes.paths.len(), 100);
            pending.changes = super::ProjectChanges::default();
            pending.notified = false;
        }
        forward(Ok(changed(100)), &root, &sender, &pending);
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1))?,
            AppEvent::ProjectFilesChanged
        ));
        Ok(())
    }

    #[test]
    fn explorer_changes_use_paths_for_files_and_rescan_for_directories()
    -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("oxyst-watch-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(root)?;
        let typst = root.join("added.typ");
        let mut changes = ProjectChanges::default();
        merge_explorer_changes(
            &Event::new(EventKind::Create(CreateKind::File)).add_path(typst.clone()),
            &root,
            &mut changes,
        );
        assert_eq!(changes.paths(), vec![typst]);
        assert!(!changes.requires_rescan());

        let renamed = root.join("renamed.typ");
        fs::write(&renamed, "")?;
        merge_explorer_changes(
            &Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(root.join("added.typ"))
                .add_path(renamed.clone()),
            &root,
            &mut changes,
        );
        assert_eq!(changes.paths(), vec![root.join("added.typ"), renamed]);
        assert!(!changes.requires_rescan());

        merge_explorer_changes(
            &Event::new(EventKind::Create(CreateKind::Folder)).add_path(root.join("chapter")),
            &root,
            &mut changes,
        );
        assert!(changes.requires_rescan());

        let mut rescan = ProjectChanges::default();
        merge_explorer_changes(
            &Event::new(EventKind::Other).set_flag(Flag::Rescan),
            &root,
            &mut rescan,
        );
        assert!(rescan.requires_rescan());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn typst_directory_rename_requests_a_rescan() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("oxyst-watch-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(root)?;
        let renamed = root.join("renamed.typ");
        fs::create_dir_all(&renamed)?;
        let mut changes = ProjectChanges::default();

        merge_explorer_changes(
            &Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(root.join("old.typ"))
                .add_path(renamed),
            &root,
            &mut changes,
        );

        assert!(changes.requires_rescan());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn forwards_real_project_file_changes() -> Result<(), Box<dyn Error>> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("oxyst-watch-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root)?;
        let (sender, receiver) = channel();
        let watcher = ProjectWatcher::new(&root, sender).map_err(std::io::Error::other)?;

        fs::write(root.join("included.typ"), "watched")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let event =
                receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
            assert!(matches!(event, AppEvent::ProjectFilesChanged));
            let changes = watcher
                .take_project_changes()
                .map_err(std::io::Error::other)?;
            if changes
                .paths()
                .iter()
                .any(|path| path.ends_with("included.typ"))
            {
                break;
            }
        }

        drop(watcher);
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
