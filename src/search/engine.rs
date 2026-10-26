use std::{
    cmp::Ordering,
    collections::HashMap,
    path::{Path, PathBuf},
};

use gpui_kit::component::IconName;
use nucleo_matcher::{
    Config as MatcherConfig, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};

use crate::{
    config::UsageStore,
    model::{Action, CommandId, Entry, EntryIcon, ItemKind, app_letter, color_for_name},
    search::{
        apps::{AppLaunch, AppRecord},
        calculator::{is_expression, try_evaluate},
        files::FileRecord,
    },
};

#[derive(Clone)]
pub struct Hit {
    pub index: usize,
    pub entry: Entry,
}

#[derive(Clone, Default)]
pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Commands,
    Files,
    Apps,
    Web,
}

impl Filter {
    fn allows(self, kind: ItemKind) -> bool {
        match self {
            Self::All => true,
            Self::Commands => kind == ItemKind::Command,
            Self::Files => matches!(kind, ItemKind::File | ItemKind::Folder),
            Self::Apps => kind == ItemKind::App,
            Self::Web => kind == ItemKind::Web,
        }
    }
}

struct Candidate {
    index: usize,
    source: usize,
    local_index: usize,
    score: i64,
    kind: ItemKind,
    name_len: usize,
    name: String,
    usage_key: String,
    generated: Option<Entry>,
}

impl Candidate {
    fn from_source(
        index: usize,
        source: usize,
        local_index: usize,
        entry: &Entry,
        score: i64,
    ) -> Self {
        Self {
            index,
            source,
            local_index,
            score,
            kind: entry.kind,
            name_len: entry.name.chars().count(),
            name: entry.name.to_lowercase(),
            usage_key: entry.usage_key.clone(),
            generated: None,
        }
    }

    fn generated(index: usize, entry: &Entry, score: i64) -> Self {
        Self {
            index,
            source: usize::MAX,
            local_index: usize::MAX,
            score,
            kind: entry.kind,
            name_len: entry.name.chars().count(),
            name: entry.name.to_lowercase(),
            usage_key: entry.usage_key.clone(),
            generated: Some(entry.clone()),
        }
    }

    fn into_hit(self, sources: &[&[Entry]]) -> Option<Hit> {
        let entry = match self.generated {
            Some(entry) => entry,
            None => sources.get(self.source)?.get(self.local_index)?.clone(),
        };
        Some(Hit {
            index: self.index,
            entry,
        })
    }
}

pub fn app_entries(apps: &[AppRecord], icon_map: &HashMap<PathBuf, PathBuf>) -> Vec<Entry> {
    apps.iter()
        .map(|app| {
            let icon_path = icon_map.get(&app.path).cloned();
            let (action, usage_key) = match &app.launch {
                AppLaunch::Path(path) => (
                    Action::OpenPath(path.clone()),
                    format!("app:{}", path_lower(path)),
                ),
                AppLaunch::Shell(shell) => (
                    Action::OpenUrl(shell.clone()),
                    format!("app:{}", shell.to_lowercase()),
                ),
            };
            Entry::new(
                app.name.clone(),
                "Application",
                ItemKind::App,
                action,
                EntryIcon::App {
                    icon_path,
                    letter: app_letter(&app.name),
                    color: color_for_name(&app.name),
                },
                usage_key,
            )
        })
        .collect()
}

pub fn file_entries(files: &[FileRecord]) -> Vec<Entry> {
    let home = dirs::home_dir();
    files
        .iter()
        .map(|file| {
            let kind = if file.is_dir {
                ItemKind::Folder
            } else {
                ItemKind::File
            };
            let extension = file
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let keywords = if file.is_dir {
                "folder directory".to_owned()
            } else if extension.is_empty() {
                "file".to_owned()
            } else {
                format!("file {extension}")
            };
            Entry::new(
                file.name.clone(),
                shorten_path(&file.path, home.as_deref()),
                kind,
                Action::OpenPath(file.path.clone()),
                file_icon(&file.path, file.is_dir),
                format!("file:{}", path_lower(&file.path)),
            )
            .with_keywords(keywords)
        })
        .collect()
}

pub fn command_entries(autostart_enabled: bool) -> Vec<Entry> {
    let autostart_label = if autostart_enabled {
        "Disable launch at login"
    } else {
        "Enable launch at login"
    };

    vec![
        command(
            CommandId::OpenSettings,
            "Open Settings",
            "Open Windows Settings",
            IconName::Settings,
            "settings preferences windows",
        ),
        command(
            CommandId::Lock,
            "Lock Screen",
            "Lock this PC",
            IconName::Moon,
            "lock sign out security",
        ),
        command(
            CommandId::TaskManager,
            "Task Manager",
            "Manage running processes",
            IconName::Cpu,
            "processes performance cpu",
        ),
        command(
            CommandId::OpenExplorer,
            "File Explorer",
            "Open File Explorer",
            IconName::FolderOpen,
            "files folders explorer",
        ),
        command(
            CommandId::OpenTerminal,
            "Open Terminal",
            "Open Windows Terminal",
            IconName::SquareTerminal,
            "terminal console shell command line",
        ),
        command(
            CommandId::OpenCalculator,
            "Open Calculator",
            "Open the Windows calculator",
            IconName::Plus,
            "calculator math numbers",
        ),
        command(
            CommandId::OpenNotepad,
            "Open Notepad",
            "Open Notepad",
            IconName::FileText,
            "notepad text editor notes",
        ),
        command(
            CommandId::OpenPaint,
            "Open Paint",
            "Open Microsoft Paint",
            IconName::Palette,
            "paint drawing image editor",
        ),
        command(
            CommandId::OpenRecycleBin,
            "Recycle Bin",
            "Open the Recycle Bin",
            IconName::Delete,
            "trash deleted files",
        ),
        command(
            CommandId::OpenDesktop,
            "Open Desktop",
            "Open your Desktop folder",
            IconName::FolderOpen,
            "folder desktop",
        ),
        command(
            CommandId::OpenDocuments,
            "Open Documents",
            "Open your Documents folder",
            IconName::FolderOpen,
            "folder documents",
        ),
        command(
            CommandId::OpenDownloads,
            "Open Downloads",
            "Open your Downloads folder",
            IconName::FolderOpen,
            "folder downloads",
        ),
        command(
            CommandId::OpenPictures,
            "Open Pictures",
            "Open your Pictures folder",
            IconName::FolderOpen,
            "folder pictures images",
        ),
        command(
            CommandId::OpenHome,
            "Open Home",
            "Open your home folder",
            IconName::Folder,
            "folder home user profile",
        ),
        command(
            CommandId::EditConfig,
            "Edit Config",
            "Open config.json",
            IconName::Settings2,
            "settings json edit configuration",
        ),
        command(
            CommandId::OpenDataFolder,
            "Open Data Folder",
            "Open the Luma data folder",
            IconName::Folder,
            "data cache settings files",
        ),
        command(
            CommandId::ReindexFiles,
            "Reindex Files",
            "Rebuild the file index",
            IconName::RefreshCw,
            "index rebuild refresh files",
        ),
        command(
            CommandId::Quit,
            "Quit Luma",
            "Close the launcher",
            IconName::Close,
            "exit close quit",
        ),
        command(
            CommandId::ToggleAutostart,
            autostart_label,
            if autostart_enabled {
                "Stop Luma from starting with Windows"
            } else {
                "Start Luma when you sign in"
            },
            IconName::Check,
            "autostart startup login launch",
        ),
    ]
}

/// Search a single entry slice. Generated results are appended conceptually.
#[cfg(test)]
pub fn search(entries: &[Entry], query: &str, usage: &UsageStore, limit: usize) -> SearchOutcome {
    search_sources(&[entries], query, usage, limit)
}

/// Search across independently owned entry slices (apps, files, commands).
pub fn search_sources(
    sources: &[&[Entry]],
    query: &str,
    usage: &UsageStore,
    limit: usize,
) -> SearchOutcome {
    let total_entries: usize = sources.iter().map(|source| source.len()).sum();
    let (filter, term) = parse_query(query);
    if term.is_empty() {
        return suggestions_for(sources, usage, limit, filter);
    }

    if filter == Filter::All
        && is_expression(term)
        && let Some(entry) = calculator_entry(term)
    {
        let score = kind_boost(ItemKind::Calculator) + usage.boost("calc")
            - short_name_penalty(entry.name.chars().count());
        let candidate = Candidate::generated(total_entries, &entry, score);
        return SearchOutcome {
            hits: candidate.into_hit(sources).into_iter().collect(),
            total: 1,
        };
    }

    let term_lower = term.to_lowercase();
    let pattern = Pattern::parse(term, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(MatcherConfig::DEFAULT.match_paths());
    let mut buf = Vec::new();
    let mut candidates = Vec::new();

    if !pattern.atoms.is_empty() {
        let mut index = 0;
        for (source_index, source) in sources.iter().enumerate() {
            for (local_index, entry) in source.iter().enumerate() {
                let global_index = index;
                index += 1;

                if !filter.allows(entry.kind) {
                    continue;
                }

                let is_path = matches!(entry.kind, ItemKind::File | ItemKind::Folder);
                let is_app = entry.kind == ItemKind::App;
                let is_command = entry.kind == ItemKind::Command;
                let name_score = if is_path || is_app {
                    let name_hay = Utf32Str::new(&entry.name, &mut buf);
                    pattern.score(name_hay, &mut matcher)
                } else {
                    None
                };

                if is_app && name_score.is_none() {
                    continue;
                }

                let base = if is_app {
                    0
                } else {
                    let hay = Utf32Str::new(&entry.search, &mut buf);
                    let Some(base) = pattern.score(hay, &mut matcher) else {
                        continue;
                    };
                    base
                };

                let name = entry.name.to_lowercase();
                let rescore = || {
                    kind_boost(entry.kind) + usage.boost(&entry.usage_key)
                        - depth_penalty(entry)
                        - short_name_penalty(entry.name.chars().count())
                };
                let mut score = i64::from(base) + rescore();

                if is_app {
                    let name_contains = contains_ignore_ascii_case(&entry.name, &term_lower);
                    let multiword = term.contains(' ');
                    let short_query = term.chars().count() <= 3;
                    if !name_contains && !multiword && !short_query {
                        continue;
                    }
                    if let Some(name_score) = name_score {
                        score = i64::from(name_score) * 2 + rescore()
                            - if name_contains { 0 } else { 120 };
                    }
                } else if is_command {
                    let name_contains = contains_ignore_ascii_case(&entry.name, &term_lower);
                    let keyword_contains = contains_ignore_ascii_case(&entry.keywords, &term_lower);
                    let multiword = term.contains(' ');
                    let short_query = term.chars().count() <= 3;
                    if !name_contains && !keyword_contains && !multiword && !short_query {
                        continue;
                    }
                    if !name_contains && !keyword_contains {
                        score -= 120;
                    }
                } else if is_path {
                    let name_contains = contains_ignore_ascii_case(&entry.name, &term_lower);
                    let path_like = term.contains(' ') || term.contains('/') || term.contains('\\');
                    let path_contains =
                        !name_contains && contains_ignore_ascii_case(&entry.subtitle, &term_lower);
                    let short_query = term.chars().count() <= 3;

                    if name_contains {
                        if let Some(name_score) = name_score {
                            score = i64::from(name_score) * 2 + rescore();
                        }
                    } else if path_contains {
                        score -= 180;
                    } else if path_like || short_query {
                        match name_score {
                            Some(name_score) => score = i64::from(name_score) * 2 + rescore() - 120,
                            None => score -= 180,
                        }
                    } else {
                        continue;
                    }
                }

                if name == term_lower {
                    score += 1_000;
                }
                if name.starts_with(term_lower.as_str()) {
                    score += 400;
                }

                candidates.push(Candidate::from_source(
                    global_index,
                    source_index,
                    local_index,
                    entry,
                    score,
                ));
            }
        }
    }

    if candidates.is_empty() && matches!(filter, Filter::All | Filter::Web) {
        let entry = web_search_entry(term);
        let score = kind_boost(ItemKind::Web) + usage.boost(&entry.usage_key)
            - short_name_penalty(entry.name.chars().count());
        candidates.push(Candidate::generated(total_entries + 1, &entry, score));
    }

    let total = candidates.len();
    candidates.sort_by(compare_search);
    if let Some(position) = candidates
        .iter()
        .position(|candidate| candidate.kind == ItemKind::Calculator)
        && position != 0
    {
        let calculator = candidates.remove(position);
        candidates.insert(0, calculator);
    }
    candidates.truncate(limit);

    SearchOutcome {
        hits: candidates
            .into_iter()
            .filter_map(|candidate| candidate.into_hit(sources))
            .collect(),
        total,
    }
}
#[cfg(test)]
pub fn suggestions(entries: &[Entry], usage: &UsageStore, limit: usize) -> SearchOutcome {
    suggestions_sources(&[entries], usage, limit)
}

pub fn suggestions_sources(
    sources: &[&[Entry]],
    usage: &UsageStore,
    limit: usize,
) -> SearchOutcome {
    suggestions_for(sources, usage, limit, Filter::All)
}

/// Generated results are not part of the input slice. Calculator hits use
/// index `entries.len()`; resolve them with this function.
pub fn calculator_entry(term: &str) -> Option<Entry> {
    let result = try_evaluate(term)?;
    Some(
        Entry::new(
            result.display,
            "Copy result to clipboard",
            ItemKind::Calculator,
            Action::CopyText(result.copy),
            EntryIcon::Glyph(IconName::Plus),
            "calc",
        )
        .with_keywords("calculator math evaluate result"),
    )
}

pub fn web_search_entry(term: &str) -> Entry {
    Entry::new(
        format!("Search the web for \"{term}\""),
        "Open in your default browser",
        ItemKind::Web,
        Action::OpenUrl(format!(
            "https://www.google.com/search?q={}",
            percent_encode(term)
        )),
        EntryIcon::Glyph(IconName::Globe),
        format!("web:{term}"),
    )
    .with_keywords("web search google internet")
}

fn suggestions_for(
    sources: &[&[Entry]],
    usage: &UsageStore,
    limit: usize,
    filter: Filter,
) -> SearchOutcome {
    let include_commands = matches!(filter, Filter::All | Filter::Commands);
    let include_apps = matches!(filter, Filter::All | Filter::Apps);
    if !include_commands && !include_apps {
        return SearchOutcome::default();
    }

    let mut commands = Vec::new();
    let mut apps = Vec::new();
    let mut index = 0;
    for (source_index, source) in sources.iter().enumerate() {
        for (local_index, entry) in source.iter().enumerate() {
            let global_index = index;
            index += 1;

            if entry.kind == ItemKind::Command && include_commands {
                commands.push(Candidate::from_source(
                    global_index,
                    source_index,
                    local_index,
                    entry,
                    usage.boost(&entry.usage_key),
                ));
            } else if entry.kind == ItemKind::App && include_apps {
                apps.push(Candidate::from_source(
                    global_index,
                    source_index,
                    local_index,
                    entry,
                    usage.boost(&entry.usage_key),
                ));
            }
        }
    }

    apps.sort_by(compare_suggestions);
    if filter == Filter::All {
        commands.truncate(3);
    } else {
        commands.sort_by(compare_suggestions);
    }

    let total = commands.len() + apps.len();
    let mut candidates = commands;
    candidates.extend(apps);
    candidates.truncate(limit);

    SearchOutcome {
        hits: candidates
            .into_iter()
            .filter_map(|candidate| candidate.into_hit(sources))
            .collect(),
        total,
    }
}

fn parse_query(query: &str) -> (Filter, &str) {
    let query = query.trim();
    let (filter, query) = match query.as_bytes().first().copied() {
        Some(b'>') => (Filter::Commands, &query[1..]),
        Some(b'/') => (Filter::Files, &query[1..]),
        Some(b'@') => (Filter::Apps, &query[1..]),
        Some(b'?') => (Filter::Web, &query[1..]),
        _ => (Filter::All, query),
    };
    (filter, query.trim())
}

fn command(id: CommandId, name: &str, subtitle: &str, icon: IconName, keywords: &str) -> Entry {
    Entry::new(
        name,
        subtitle,
        ItemKind::Command,
        Action::Command(id),
        EntryIcon::Glyph(icon),
        format!("cmd:{id:?}"),
    )
    .with_keywords(keywords)
}

fn file_icon(path: &Path, is_dir: bool) -> EntryIcon {
    if is_dir {
        return EntryIcon::Glyph(IconName::Folder);
    }

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let icon = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" => IconName::Palette,
        "mp4" | "mkv" | "mov" | "avi" | "webm" => IconName::Play,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" => IconName::Mic,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "cpp" | "h" | "cs" | "java"
        | "json" | "toml" | "yaml" | "yml" | "md" | "txt" | "pdf" | "doc" | "docx" | "xls"
        | "xlsx" | "ppt" | "pptx" => IconName::FileText,
        _ => IconName::File,
    };
    EntryIcon::Glyph(icon)
}

fn shorten_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home {
        let home = home.to_string_lossy();
        let text = path.to_string_lossy();
        if let Some(prefix) = text.get(..home.len())
            && prefix.eq_ignore_ascii_case(home.as_ref())
        {
            let rest = &text[home.len()..];
            if rest.is_empty() || rest.starts_with('\\') || rest.starts_with('/') {
                return format!("~{rest}");
            }
        }
    }
    path.to_string_lossy().into_owned()
}

fn path_lower(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle = needle.as_bytes();
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn kind_boost(kind: ItemKind) -> i64 {
    match kind {
        ItemKind::Calculator => 5_000,
        ItemKind::App => 650,
        ItemKind::Command => 600,
        ItemKind::Folder => 40,
        ItemKind::File => 0,
        ItemKind::Web => 10,
    }
}

fn depth_penalty(entry: &Entry) -> i64 {
    if !matches!(entry.kind, ItemKind::File | ItemKind::Folder) {
        return 0;
    }
    let Action::OpenPath(path) = &entry.action else {
        return 0;
    };
    (path.components().count() as i64 * 4).min(80)
}

fn short_name_penalty(name_len: usize) -> i64 {
    (name_len as i64 / 8).min(40)
}

fn compare_search(a: &Candidate, b: &Candidate) -> Ordering {
    b.score
        .cmp(&a.score)
        .then_with(|| a.kind.rank().cmp(&b.kind.rank()))
        .then_with(|| a.name_len.cmp(&b.name_len))
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.usage_key.cmp(&b.usage_key))
        .then_with(|| a.index.cmp(&b.index))
}

fn compare_suggestions(a: &Candidate, b: &Candidate) -> Ordering {
    b.score
        .cmp(&a.score)
        .then_with(|| a.kind.rank().cmp(&b.kind.rank()))
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.usage_key.cmp(&b.usage_key))
        .then_with(|| a.index.cmp(&b.index))
}

fn percent_encode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push_str("%20"),
            _ => {
                encoded.push('%');
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::UsageRecord;

    fn test_entry(name: &str, kind: ItemKind, usage_key: &str) -> Entry {
        Entry::new(
            name,
            "",
            kind,
            Action::CopyText(String::new()),
            EntryIcon::Glyph(IconName::Info),
            usage_key,
        )
    }

    fn usage_with(entries: &[(&str, u32)]) -> UsageStore {
        let mut usage = UsageStore::default();
        for (key, uses) in entries {
            usage.records.insert(
                (*key).to_owned(),
                UsageRecord {
                    uses: *uses,
                    last_used: crate::config::unix_now(),
                },
            );
        }
        usage
    }

    #[test]
    fn parses_query_prefixes() {
        assert_eq!(parse_query("  hello  "), (Filter::All, "hello"));
        assert_eq!(parse_query(">ls"), (Filter::Commands, "ls"));
        assert_eq!(parse_query("/notes"), (Filter::Files, "notes"));
        assert_eq!(parse_query("@code"), (Filter::Apps, "code"));
        assert_eq!(parse_query("?weather"), (Filter::Web, "weather"));
        assert_eq!(parse_query(">"), (Filter::Commands, ""));
        assert_eq!(parse_query("  /  "), (Filter::Files, ""));
    }

    #[test]
    fn ranks_apps_ahead_of_files_with_the_same_name() {
        let entries = vec![
            Entry::new(
                "notes",
                "Application",
                ItemKind::App,
                Action::OpenPath(PathBuf::from(r"C:\notes.lnk")),
                EntryIcon::Glyph(IconName::Info),
                "app:notes",
            ),
            Entry::new(
                "notes",
                r"C:\Users\Me\notes.txt",
                ItemKind::File,
                Action::OpenPath(PathBuf::from(r"C:\Users\Me\notes.txt")),
                EntryIcon::Glyph(IconName::FileText),
                "file:notes",
            ),
        ];

        let outcome = search(&entries, "notes", &UsageStore::default(), 10);
        assert_eq!(outcome.total, 2);
        assert_eq!(outcome.hits[0].index, 0);
        assert_eq!(outcome.hits[0].entry.kind, ItemKind::App);
    }

    #[test]
    fn suggestions_mix_commands_and_apps() {
        let mut entries = Vec::new();
        for index in 0..4 {
            entries.push(test_entry(
                &format!("Command {index}"),
                ItemKind::Command,
                &format!("cmd:{index}"),
            ));
        }
        for index in 0..4 {
            entries.push(test_entry(
                &format!("App {index}"),
                ItemKind::App,
                &format!("app:{index}"),
            ));
        }

        let usage = usage_with(&[
            ("cmd:0", 10),
            ("cmd:1", 9),
            ("cmd:2", 8),
            ("cmd:3", 7),
            ("app:0", 1),
            ("app:1", 1),
            ("app:2", 1),
            ("app:3", 1),
        ]);

        let outcome = suggestions(&entries, &usage, 20);
        assert_eq!(outcome.total, 7);
        assert_eq!(outcome.hits.len(), 7);
        assert_eq!(outcome.hits.iter().filter(|hit| hit.index < 4).count(), 3);
        assert!(outcome.hits.iter().any(|hit| hit.index >= 4));

        let commands = search(&entries, ">", &usage, 20);
        assert_eq!(commands.total, 4);
        assert!(commands.hits.iter().all(|hit| hit.index < 4));

        let apps = search(&entries, "@", &usage, 20);
        assert_eq!(apps.total, 4);
        assert!(apps.hits.iter().all(|hit| hit.index >= 4));
    }

    #[test]
    fn searches_across_sources() {
        let apps = vec![test_entry("Notes", ItemKind::App, "app:notes")];
        let files = vec![test_entry("notes.txt", ItemKind::File, "file:notes")];
        let commands = vec![command(
            CommandId::OpenDownloads,
            "Open Downloads",
            "Open your Downloads folder",
            IconName::FolderOpen,
            "folder downloads",
        )];
        let sources: [&[Entry]; 3] = [&apps, &files, &commands];

        let outcome = search_sources(&sources, "notes", &UsageStore::default(), 8);
        assert_eq!(outcome.total, 2);
        assert_eq!(outcome.hits[0].entry.name, "Notes");

        let suggestions = suggestions_sources(&sources, &UsageStore::default(), 8);
        assert_eq!(suggestions.total, 2);
        assert!(suggestions.hits.iter().any(|hit| hit.entry.name == "Notes"));
        assert!(
            suggestions
                .hits
                .iter()
                .any(|hit| hit.entry.name == "Open Downloads")
        );
    }

    #[test]
    fn percent_encodes_web_queries() {
        assert_eq!(percent_encode("rust lang"), "rust%20lang");
        assert_eq!(percent_encode("A-Z_a.b~c&d/e"), "A-Z_a.b~c%26d%2Fe");
        assert_eq!(percent_encode("café"), "caf%C3%A9");
    }

    #[test]
    fn path_only_matches_need_a_path_like_query() {
        let entries = vec![
            test_entry("Notepad++", ItemKind::App, "app:notepad"),
            Entry::new(
                "data-schemas",
                r"C:\Projekt\NorthBridge-Chat\packages\data-schemas",
                ItemKind::Folder,
                Action::OpenPath(PathBuf::from(
                    r"C:\Projekt\NorthBridge-Chat\packages\data-schemas",
                )),
                EntryIcon::Glyph(IconName::Folder),
                "file:data-schemas",
            ),
        ];

        let name_query = search(&entries, "notepad", &UsageStore::default(), 8);
        assert_eq!(name_query.hits.len(), 1);
        assert_eq!(name_query.hits[0].entry.name, "Notepad++");

        let path_query = search(&entries, "northbridge data", &UsageStore::default(), 8);
        assert!(
            path_query
                .hits
                .iter()
                .any(|hit| hit.entry.name == "data-schemas")
        );
    }

    #[test]
    fn scattered_app_letters_do_not_match() {
        let entries = vec![
            test_entry("Epic Games Launcher", ItemKind::App, "app:epic"),
            command(
                CommandId::OpenCalculator,
                "Open Calculator",
                "Open the Windows calculator",
                IconName::Plus,
                "calculator math numbers",
            ),
        ];

        let outcome = search(&entries, "calc", &UsageStore::default(), 8);
        assert_eq!(outcome.hits[0].entry.name, "Open Calculator");
        assert!(
            outcome
                .hits
                .iter()
                .all(|hit| hit.entry.name != "Epic Games Launcher")
        );
    }

    #[test]
    fn command_keywords_still_match() {
        let entries = vec![command(
            CommandId::OpenRecycleBin,
            "Recycle Bin",
            "Open the Recycle Bin",
            IconName::Delete,
            "trash deleted files",
        )];

        let outcome = search(&entries, "trash", &UsageStore::default(), 8);
        assert_eq!(outcome.hits[0].entry.name, "Recycle Bin");
    }

    #[test]
    fn calculator_short_circuits_file_matches() {
        let entries = vec![test_entry("2+2-notes", ItemKind::File, "file:notes")];
        let outcome = search(&entries, "2+2", &UsageStore::default(), 8);
        assert_eq!(outcome.total, 1);
        assert_eq!(outcome.hits[0].entry.kind, ItemKind::Calculator);
    }

    #[test]
    fn falls_back_to_web_search_without_matches() {
        let entries = vec![test_entry("Notes", ItemKind::App, "app:notes")];
        let outcome = search(&entries, "weather", &UsageStore::default(), 5);

        assert_eq!(outcome.total, 1);
        assert_eq!(outcome.hits.len(), 1);
        assert_eq!(outcome.hits[0].index, entries.len() + 1);
    }
}
