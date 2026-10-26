use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: String,
    pub max_results: usize,
    pub index: IndexSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "alt+space".into(),
            max_results: 8,
            index: IndexSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexSettings {
    pub enabled: bool,
    pub max_entries: usize,
    pub max_depth: usize,
    pub ttl_hours: u64,
    pub extra_roots: Vec<PathBuf>,
    pub excluded_names: Vec<String>,
}

impl Default for IndexSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 150_000,
            max_depth: 8,
            ttl_hours: 12,
            extra_roots: Vec::new(),
            excluded_names: default_excluded_names(),
        }
    }
}

pub fn default_excluded_names() -> Vec<String> {
    [
        "$RECYCLE.BIN",
        "$Recycle.Bin",
        "System Volume Information",
        "Windows",
        "WinSxS",
        "Temp",
        "tmp",
        "node_modules",
        ".git",
        ".hg",
        ".svn",
        "target",
        "build",
        "dist",
        "out",
        ".next",
        ".nuxt",
        ".cache",
        ".parcel-cache",
        "__pycache__",
        ".venv",
        "venv",
        ".gradle",
        ".m2",
        ".nuget",
        ".cargo",
        ".rustup",
        "AppData",
        "Application Data",
        "Local Settings",
        "Cookies",
        "NetHood",
        "PrintHood",
        "Recent",
        "SendTo",
        "Start Menu",
        "Templates",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

impl Config {
    pub fn load_or_default() -> Self {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                eprintln!("luma: could not parse config {}: {error}", path.display());
                Self::default()
            }),
            Err(_) => {
                let config = Self::default();
                let _ = config.save();
                config
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        write_atomic(&config_path(), serde_json::to_vec_pretty(self)?.as_slice())
    }
}

pub fn data_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("Luma");
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Luma")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn usage_path() -> PathBuf {
    data_dir().join("usage.json")
}

pub fn cache_path() -> PathBuf {
    data_dir().join("file-index.bin")
}

pub fn icons_dir() -> PathBuf {
    data_dir().join("icons")
}

pub fn ensure_data_dirs() -> std::io::Result<()> {
    fs::create_dir_all(data_dir())?;
    fs::create_dir_all(icons_dir())?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Default)]
pub struct UsageRecord {
    pub uses: u32,
    pub last_used: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct UsageStore {
    pub records: HashMap<String, UsageRecord>,
}

impl UsageStore {
    pub fn load() -> Self {
        fs::read(usage_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = fs::create_dir_all(data_dir());
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = write_atomic(&usage_path(), &bytes);
        }
    }

    pub fn record_use(&mut self, key: &str) {
        let record = self.records.entry(key.to_owned()).or_default();
        record.uses = record.uses.saturating_add(1);
        record.last_used = unix_now();
    }

    pub fn boost(&self, key: &str) -> i64 {
        let Some(record) = self.records.get(key) else {
            return 0;
        };
        let uses = i64::from(record.uses.min(30)) * 4;
        let age = unix_now().saturating_sub(record.last_used);
        let recency = match age {
            0..=86_400 => 120,
            86_401..=604_800 => 80,
            604_801..=2_592_000 => 40,
            _ => 0,
        };
        uses + recency
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("bin")
    ));
    fs::write(&temp, contents)?;
    fs::rename(&temp, path).or_else(|_| {
        let _ = fs::remove_file(path);
        fs::rename(&temp, path)
    })
}
