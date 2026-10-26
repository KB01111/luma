use std::{io::ErrorKind, path::Path};

use windows::{
    Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    core::PCWSTR,
};
use winreg::{RegKey, enums::*};

use crate::model::Action;

const AUTOSTART_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_VALUE: &str = "Luma";

pub fn open_action(action: &Action) -> Result<(), String> {
    match action {
        Action::OpenPath(path) => open_path(path),
        Action::OpenUrl(url) => open_url(url),
        Action::CopyText(_) => Err("clipboard action should be handled by UI".to_owned()),
        Action::Command(_) => Err("command action should be handled by UI".to_owned()),
    }
}

pub fn open_path(path: &Path) -> Result<(), String> {
    let file = path.to_string_lossy();
    if file.is_empty() {
        return Err("cannot open an empty path".to_owned());
    }
    shell_execute("open", file.as_ref(), None)
}

pub fn open_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("cannot open an empty URL".to_owned());
    }
    shell_execute("open", url, None)
}

pub fn open_shell(uri: &str) -> Result<(), String> {
    let uri = uri.trim();
    if uri.is_empty() {
        return Err("cannot open an empty shell URI".to_owned());
    }
    shell_execute("open", uri, None)
}

pub fn reveal(path: &Path) -> Result<(), String> {
    let parameters = format!("/select,\"{}\"", path.display());
    run_program("explorer.exe", Some(parameters.as_str()), false)
}

pub fn run_program(program: &str, args: Option<&str>, run_as_admin: bool) -> Result<(), String> {
    if program.trim().is_empty() {
        return Err("cannot run an empty program path".to_owned());
    }

    let operation = if run_as_admin { "runas" } else { "open" };
    shell_execute(operation, program, args)
}

pub fn lock_workstation() -> Result<(), String> {
    run_program("rundll32.exe", Some("user32.dll,LockWorkStation"), false)
}

pub fn autostart_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags(AUTOSTART_RUN_KEY, KEY_READ)
        .and_then(|key| key.get_raw_value(AUTOSTART_VALUE))
        .is_ok()
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    if enabled {
        let command = autostart_command()?;
        let (key, _) = hkcu
            .create_subkey(AUTOSTART_RUN_KEY)
            .map_err(|error| format!("could not open the autostart registry key: {error}"))?;
        key.set_value(AUTOSTART_VALUE, &command)
            .map_err(|error| format!("could not enable autostart: {error}"))
    } else {
        let key = match hkcu.open_subkey_with_flags(AUTOSTART_RUN_KEY, KEY_SET_VALUE) {
            Ok(key) => key,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "could not open the autostart registry key: {error}"
                ));
            }
        };

        match key.delete_value(AUTOSTART_VALUE) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not disable autostart: {error}")),
        }
    }
}

fn autostart_command() -> Result<String, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not resolve the current executable: {error}"))?;
    Ok(format!("\"{}\" --hidden", executable.display()))
}

fn shell_execute(operation: &str, file: &str, parameters: Option<&str>) -> Result<(), String> {
    let operation = wide(operation);
    let file = wide(file);
    let parameters = parameters.map(wide);

    let instance = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(file.as_ptr()),
            parameters
                .as_ref()
                .map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr())),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    let code = instance.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(shell_error(code))
    }
}

fn shell_error(code: isize) -> String {
    let reason = match code {
        0 | 8 => "out of memory",
        2 => "file not found",
        3 => "path not found",
        5 => "access denied",
        26 => "sharing violation",
        27 => "incomplete association",
        28 => "DDE timeout",
        29 => "DDE failure",
        30 => "DDE busy",
        31 => "no associated application",
        32 => "DLL not found",
        _ => "unknown error",
    };

    format!("ShellExecute failed with code {code}: {reason}")
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
