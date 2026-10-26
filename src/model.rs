use std::path::PathBuf;

use gpui_kit::component::IconName;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    App,
    File,
    Folder,
    Command,
    Calculator,
    Web,
}

impl ItemKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::App => "Application",
            Self::File => "File",
            Self::Folder => "Folder",
            Self::Command => "Command",
            Self::Calculator => "Calculator",
            Self::Web => "Search the web",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Calculator => 0,
            Self::App => 1,
            Self::Command => 2,
            Self::Folder => 3,
            Self::File => 4,
            Self::Web => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandId {
    OpenHome,
    OpenDesktop,
    OpenDocuments,
    OpenDownloads,
    OpenPictures,
    OpenSettings,
    OpenRecycleBin,
    OpenExplorer,
    OpenTerminal,
    OpenCalculator,
    OpenNotepad,
    OpenPaint,
    Lock,
    TaskManager,
    Quit,
    ReindexFiles,
    EditConfig,
    OpenDataFolder,
    ToggleAutostart,
}

#[derive(Clone, Debug)]
pub enum Action {
    OpenPath(PathBuf),
    OpenUrl(String),
    Command(CommandId),
    CopyText(String),
}

#[derive(Clone)]
pub enum EntryIcon {
    App {
        icon_path: Option<PathBuf>,
        letter: char,
        color: u32,
    },
    Glyph(IconName),
}

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub subtitle: String,
    pub keywords: String,
    pub search: String,
    pub kind: ItemKind,
    pub action: Action,
    pub icon: EntryIcon,
    pub usage_key: String,
}

impl Entry {
    pub fn new(
        name: impl Into<String>,
        subtitle: impl Into<String>,
        kind: ItemKind,
        action: Action,
        icon: EntryIcon,
        usage_key: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let subtitle = subtitle.into();
        let keywords = String::new();
        let search = make_search_blob(&name, &subtitle, &keywords);
        Self {
            name,
            subtitle,
            keywords,
            search,
            kind,
            action,
            icon,
            usage_key: usage_key.into(),
        }
    }

    pub fn with_keywords(mut self, keywords: impl Into<String>) -> Self {
        self.keywords = keywords.into();
        self.search = make_search_blob(&self.name, &self.subtitle, &self.keywords);
        self
    }
}

pub fn make_search_blob(name: &str, subtitle: &str, keywords: &str) -> String {
    let mut blob = String::with_capacity(name.len() + subtitle.len() + keywords.len() + 4);
    blob.push_str(name);
    blob.push_str("  ");
    blob.push_str(subtitle);
    if !keywords.is_empty() {
        blob.push_str("  ");
        blob.push_str(keywords);
    }
    blob
}

pub fn app_letter(name: &str) -> char {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('?')
}

pub fn color_for_name(name: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in name.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    const PALETTE: [u32; 8] = [
        0x5B8DEF, 0x8B5CF6, 0xEC4899, 0xF59E0B, 0x10B981, 0x06B6D4, 0xEF4444, 0x84CC16,
    ];
    PALETTE[hash as usize % PALETTE.len()]
}
