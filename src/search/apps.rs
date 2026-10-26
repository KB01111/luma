use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

const MAX_SHORTCUT_DEPTH: usize = 5;

const PRIORITY_USER_START: u8 = 0;
const PRIORITY_USER_DESKTOP: u8 = 1;
const PRIORITY_PROGRAM_DATA: u8 = 2;
const PRIORITY_PUBLIC_DESKTOP: u8 = 3;
const PRIORITY_UWP: u8 = 4;

const HELPER_MARKERS: &[&str] = &[
    "uninstall",
    "readme",
    "help",
    "website",
    "documentation",
    "release notes",
    "license",
    "changelog",
    "update",
    "repair",
    "safe mode",
];

#[derive(Clone, Debug)]
pub enum AppLaunch {
    Path(PathBuf),
    Shell(String),
}

#[derive(Clone, Debug)]
pub struct AppRecord {
    pub name: String,
    pub path: PathBuf,
    pub launch: AppLaunch,
}

pub fn scan_apps() -> Vec<AppRecord> {
    let mut candidates = Vec::new();

    if let Some(appdata) = env::var_os("APPDATA") {
        let directory = PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        scan_directory(
            &directory,
            0,
            MAX_SHORTCUT_DEPTH,
            "lnk",
            PRIORITY_USER_START,
            &mut candidates,
        );
    }

    if let Some(program_data) = env::var_os("ProgramData") {
        let directory = PathBuf::from(program_data)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        scan_directory(
            &directory,
            0,
            MAX_SHORTCUT_DEPTH,
            "lnk",
            PRIORITY_PROGRAM_DATA,
            &mut candidates,
        );
    }

    if let Some(desktop) = user_desktop_directory() {
        scan_directory(
            &desktop,
            0,
            MAX_SHORTCUT_DEPTH,
            "lnk",
            PRIORITY_USER_DESKTOP,
            &mut candidates,
        );
    }

    if let Some(public) = env::var_os("PUBLIC") {
        let directory = PathBuf::from(public).join("Desktop");
        scan_directory(
            &directory,
            0,
            MAX_SHORTCUT_DEPTH,
            "lnk",
            PRIORITY_PUBLIC_DESKTOP,
            &mut candidates,
        );
    }

    for (name, aumid) in scan_apps_folder() {
        if name.is_empty() || is_helper_name(&name) {
            continue;
        }
        let shell = format!(r"shell:AppsFolder\{aumid}");
        candidates.push((
            PRIORITY_UWP,
            AppRecord {
                name,
                path: PathBuf::from(&shell),
                launch: AppLaunch::Shell(shell),
            },
        ));
    }

    dedupe_candidates(candidates)
}

#[cfg(windows)]
fn scan_apps_folder() -> Vec<(String, String)> {
    use windows::{
        Win32::{
            Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE},
            System::Com::{
                COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx,
            },
            UI::Shell::{
                BHID_EnumItems, IEnumShellItems, IShellItem, IShellItem2,
                SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY,
            },
        },
        core::{GUID, HSTRING, Interface, PWSTR},
    };

    const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };

    struct ComGuard;

    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn take_pwstr(value: PWSTR) -> Option<String> {
        unsafe {
            if value.is_null() {
                return None;
            }
            let text = value.to_string().ok();
            CoTaskMemFree(Some(value.0 as *const std::ffi::c_void));
            text
        }
    }

    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _com = if initialized == RPC_E_CHANGED_MODE {
        None
    } else if initialized.is_ok() {
        Some(ComGuard)
    } else {
        return Vec::new();
    };

    let mut found = Vec::new();
    unsafe {
        let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(
            &HSTRING::from("shell:AppsFolder"),
            None::<&IBindCtx>,
        ) else {
            return found;
        };
        let Ok(enumerator) =
            folder.BindToHandler::<_, IEnumShellItems>(None::<&IBindCtx>, &BHID_EnumItems)
        else {
            return found;
        };

        loop {
            let mut items: [Option<IShellItem>; 32] = Default::default();
            let mut fetched = 0u32;
            if enumerator.Next(&mut items, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }

            for item in items.iter().take(fetched as usize).flatten() {
                let Some(name) = item
                    .GetDisplayName(SIGDN_NORMALDISPLAY)
                    .ok()
                    .and_then(take_pwstr)
                else {
                    continue;
                };
                if name.is_empty() || name.starts_with('@') || name.starts_with("ms-resource:") {
                    continue;
                }

                let Ok(item2) = item.cast::<IShellItem2>() else {
                    continue;
                };
                let Some(aumid) = item2
                    .GetString(&PKEY_APP_USER_MODEL_ID)
                    .ok()
                    .and_then(take_pwstr)
                else {
                    continue;
                };
                if aumid.is_empty()
                    || aumid.contains('\\')
                    || aumid.starts_with('{')
                    || aumid.starts_with("::")
                {
                    continue;
                }
                found.push((name, aumid));
            }
        }
    }
    found
}

#[cfg(not(windows))]
fn scan_apps_folder() -> Vec<(String, String)> {
    Vec::new()
}

pub fn load_app_icons(apps: &[AppRecord], icons_dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut icons = Vec::new();
    for app in apps {
        if let Some(icon_path) = crate::win::icons::ensure_app_icon(&app.path, icons_dir, 64) {
            icons.push((app.path.clone(), icon_path));
        }
    }
    icons
}

fn user_desktop_directory() -> Option<PathBuf> {
    dirs::desktop_dir()
        .filter(|path| path.is_dir())
        .or_else(|| {
            let path = dirs::home_dir()?.join("Desktop");
            path.is_dir().then_some(path)
        })
}

fn scan_directory(
    directory: &Path,
    depth: usize,
    max_depth: usize,
    extension: &str,
    priority: u8,
    out: &mut Vec<(u8, AppRecord)>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();
        if file_type.is_dir() {
            if depth >= max_depth {
                continue;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            scan_directory(&path, depth + 1, max_depth, extension, priority, out);
            continue;
        }

        if !file_type.is_file() {
            continue;
        }
        let matches_extension = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension));
        if !matches_extension {
            continue;
        }

        let Some(name) = display_name_from_path(&path) else {
            continue;
        };
        if is_helper_name(&name) {
            continue;
        }

        out.push((
            priority,
            AppRecord {
                launch: AppLaunch::Path(path.clone()),
                name,
                path,
            },
        ));
    }
}

fn dedupe_candidates(candidates: Vec<(u8, AppRecord)>) -> Vec<AppRecord> {
    let mut candidates = candidates;
    candidates.sort_by(|(priority_a, a), (priority_b, b)| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| priority_a.cmp(priority_b))
            .then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for (_, app) in candidates {
        if seen.insert(app.name.to_lowercase()) {
            apps.push(app);
        }
    }

    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    apps
}

fn display_name_from_path(path: &Path) -> Option<String> {
    clean_display_name(path.file_stem()?.to_str()?)
}

fn clean_display_name(stem: &str) -> Option<String> {
    let stem = stem.trim();
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_owned())
    }
}

fn is_helper_name(name: &str) -> bool {
    let name = name.to_lowercase();
    HELPER_MARKERS.iter().any(|marker| name.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, path: &str) -> AppRecord {
        AppRecord {
            launch: AppLaunch::Path(PathBuf::from(path)),
            name: name.to_owned(),
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn cleans_display_names() {
        assert_eq!(clean_display_name("  My App  ").as_deref(), Some("My App"));
        assert_eq!(clean_display_name("   "), None);
        assert_eq!(
            display_name_from_path(Path::new("C:/Start Menu/My_App - Thing.lnk")).as_deref(),
            Some("My_App - Thing")
        );
    }

    #[test]
    fn skips_helper_shortcuts() {
        assert!(is_helper_name("Uninstall Foo"));
        assert!(is_helper_name("Foo Release Notes"));
        assert!(is_helper_name("Foo - Safe Mode"));
        assert!(!is_helper_name("Visual Studio Code"));
        assert!(!is_helper_name("Notepad"));
    }

    #[test]
    fn deduplicates_case_insensitively_and_prefers_user_start_menu() {
        let apps = dedupe_candidates(vec![
            (
                PRIORITY_PROGRAM_DATA,
                app(
                    "Visual Studio Code",
                    r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Visual Studio Code.lnk",
                ),
            ),
            (
                PRIORITY_USER_START,
                app(
                    "Visual Studio Code",
                    r"C:\Users\Me\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Visual Studio Code.lnk",
                ),
            ),
            (
                PRIORITY_USER_DESKTOP,
                app(
                    "visual studio code",
                    r"C:\Users\Me\Desktop\Visual Studio Code.lnk",
                ),
            ),
            (
                PRIORITY_USER_DESKTOP,
                app("Zed", r"C:\Users\Me\Desktop\Zed.lnk"),
            ),
        ]);

        assert_eq!(apps.len(), 2);
        assert!(apps[0].path.to_string_lossy().contains("Roaming"));
        assert_eq!(apps[1].name, "Zed");
    }

    #[test]
    fn prefers_shorter_path_for_equal_priority() {
        let apps = dedupe_candidates(vec![
            (
                PRIORITY_USER_DESKTOP,
                app("Alpha", r"C:\Users\Me\Desktop\Tools\Alpha.lnk"),
            ),
            (
                PRIORITY_USER_DESKTOP,
                app("Alpha", r"C:\Users\Me\Desktop\Alpha.lnk"),
            ),
        ]);

        assert_eq!(apps.len(), 1);
        assert_eq!(
            apps[0].path,
            PathBuf::from(r"C:\Users\Me\Desktop\Alpha.lnk")
        );
    }
}
