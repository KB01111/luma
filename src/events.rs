use std::path::PathBuf;

use crate::{model::Entry, search::apps::AppRecord};

#[derive(Clone)]
pub enum AppEvent {
    Toggle,
    FilesProgress {
        count: usize,
    },
    FilesLoaded {
        entries: Vec<Entry>,
        count: usize,
        needs_rebuild: bool,
    },
    FilesIndexed {
        entries: Vec<Entry>,
        count: usize,
        elapsed_ms: u64,
    },
    FilesFailed(String),
    AppsReady {
        apps: Vec<AppRecord>,
    },
    AppIconsReady {
        icons: Vec<(PathBuf, PathBuf)>,
    },
}
