use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{
            HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
            RegisterHotKey, UnregisterHotKey,
        },
        WindowsAndMessaging::{
            GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY,
        },
    },
};

use crate::events::AppEvent;

const HOTKEY_ID: i32 = 1;
const WM_STOP: u32 = WM_APP + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeySpec {
    pub mods: u32,
    pub vk: u32,
}

impl HotkeySpec {
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim().to_ascii_lowercase();
        if input.is_empty() {
            return None;
        }

        let mut mods = 0u32;
        let mut vk = None;

        for part in input.split('+') {
            let token = part.trim();
            if token.is_empty() {
                return None;
            }

            match token {
                "alt" => mods |= MOD_ALT.0,
                "ctrl" | "control" => mods |= MOD_CONTROL.0,
                "shift" => mods |= MOD_SHIFT.0,
                "win" | "windows" | "meta" | "super" => mods |= MOD_WIN.0,
                _ => {
                    if vk.is_some() {
                        return None;
                    }
                    vk = Some(key_from_token(token)?);
                }
            }
        }

        if mods == 0 {
            return None;
        }

        Some(Self { mods, vk: vk? })
    }

    pub fn default() -> Self {
        Self {
            mods: MOD_ALT.0,
            vk: 0x20,
        }
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::with_capacity(5);

        if self.mods & MOD_CONTROL.0 != 0 {
            parts.push("Ctrl".to_owned());
        }
        if self.mods & MOD_ALT.0 != 0 {
            parts.push("Alt".to_owned());
        }
        if self.mods & MOD_SHIFT.0 != 0 {
            parts.push("Shift".to_owned());
        }
        if self.mods & MOD_WIN.0 != 0 {
            parts.push("Win".to_owned());
        }

        parts.push(display_key(self.vk));
        parts.join(" + ")
    }
}

fn key_from_token(token: &str) -> Option<u32> {
    let bytes = token.as_bytes();

    if bytes.len() == 1 {
        let byte = bytes[0];
        return match byte {
            b'a'..=b'z' => Some(0x41 + u32::from(byte - b'a')),
            b'0'..=b'9' => Some(0x30 + u32::from(byte - b'0')),
            b';' => Some(0xBA),
            b'=' => Some(0xBB),
            b',' => Some(0xBC),
            b'-' => Some(0xBD),
            b'.' => Some(0xBE),
            b'/' => Some(0xBF),
            b'`' => Some(0xC0),
            b'[' => Some(0xDB),
            b'\\' => Some(0xDC),
            b']' => Some(0xDD),
            b'\'' => Some(0xDE),
            _ => None,
        };
    }

    let vk = match token {
        "space" | "spacebar" => 0x20,
        "enter" | "return" => 0x0D,
        "escape" | "esc" => 0x1B,
        "tab" => 0x09,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "left" | "arrowleft" => 0x25,
        "up" | "arrowup" => 0x26,
        "right" | "arrowright" => 0x27,
        "down" | "arrowdown" => 0x28,
        "printscreen" | "prtsc" | "snapshot" => 0x2C,
        "pause" | "break" => 0x13,
        "capslock" | "caps" => 0x14,
        "numlock" => 0x90,
        "scrolllock" => 0x91,
        "menu" | "apps" => 0x5D,
        "semicolon" => 0xBA,
        "equals" | "equal" | "plus" => 0xBB,
        "comma" => 0xBC,
        "minus" | "dash" => 0xBD,
        "period" | "dot" => 0xBE,
        "slash" => 0xBF,
        "backtick" | "grave" => 0xC0,
        "leftbracket" | "bracketleft" => 0xDB,
        "backslash" => 0xDC,
        "rightbracket" | "bracketright" => 0xDD,
        "apostrophe" | "quote" => 0xDE,
        _ => {
            let number = token.strip_prefix('f')?.parse::<u32>().ok()?;
            if (1..=24).contains(&number) {
                0x70 + number - 1
            } else {
                return None;
            }
        }
    };

    Some(vk)
}

fn display_key(vk: u32) -> String {
    match vk {
        0x20 => "Space".to_owned(),
        0x0D => "Enter".to_owned(),
        0x1B => "Escape".to_owned(),
        0x09 => "Tab".to_owned(),
        0x08 => "Backspace".to_owned(),
        0x2E => "Delete".to_owned(),
        0x2D => "Insert".to_owned(),
        0x24 => "Home".to_owned(),
        0x23 => "End".to_owned(),
        0x21 => "Page Up".to_owned(),
        0x22 => "Page Down".to_owned(),
        0x25 => "Left".to_owned(),
        0x26 => "Up".to_owned(),
        0x27 => "Right".to_owned(),
        0x28 => "Down".to_owned(),
        0x2C => "Print Screen".to_owned(),
        0x13 => "Pause".to_owned(),
        0x14 => "Caps Lock".to_owned(),
        0x90 => "Num Lock".to_owned(),
        0x91 => "Scroll Lock".to_owned(),
        0x5D => "Menu".to_owned(),
        0x41..=0x5A | 0x30..=0x39 => char::from(vk as u8).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0xBA => "Semicolon".to_owned(),
        0xBB => "Equals".to_owned(),
        0xBC => "Comma".to_owned(),
        0xBD => "Minus".to_owned(),
        0xBE => "Period".to_owned(),
        0xBF => "Slash".to_owned(),
        0xC0 => "Backtick".to_owned(),
        0xDB => "Left Bracket".to_owned(),
        0xDC => "Backslash".to_owned(),
        0xDD => "Right Bracket".to_owned(),
        0xDE => "Apostrophe".to_owned(),
        _ => format!("VK 0x{vk:02X}"),
    }
}

pub struct HotkeyService {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl HotkeyService {
    pub fn spawn(
        spec: HotkeySpec,
        events: async_channel::Sender<AppEvent>,
    ) -> Result<Self, String> {
        if spec.mods == 0 {
            return Err("hotkey must include at least one modifier".to_owned());
        }
        if spec.vk == 0 {
            return Err("hotkey key is invalid".to_owned());
        }

        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();

        let join = thread::Builder::new()
            .name("luma-hotkey".to_owned())
            .spawn(move || hotkey_thread(spec, events, ready_tx))
            .map_err(|error| format!("could not start the hotkey thread: {error}"))?;

        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                join: Some(join),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                let _ = join.join();
                Err("the hotkey thread stopped before it was ready".to_owned())
            }
        }
    }

    pub fn stop(&mut self) {
        let Some(join) = self.join.take() else {
            return;
        };

        if self.thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_STOP, WPARAM(0), LPARAM(0));
            }
        }

        let _ = join.join();
        self.thread_id = 0;
    }
}

impl Drop for HotkeyService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn hotkey_thread(
    spec: HotkeySpec,
    events: async_channel::Sender<AppEvent>,
    ready: mpsc::Sender<Result<u32, String>>,
) {
    let thread_id = unsafe { GetCurrentThreadId() };
    let modifiers = HOT_KEY_MODIFIERS(spec.mods | MOD_NOREPEAT.0);

    match unsafe { RegisterHotKey(None, HOTKEY_ID, modifiers, spec.vk) } {
        Ok(()) => {
            let mut message = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            }

            if ready.send(Ok(thread_id)).is_err() {
                unsafe {
                    let _ = UnregisterHotKey(None, HOTKEY_ID);
                }
                return;
            }

            loop {
                let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
                if result.0 == 0 || result.0 == -1 {
                    break;
                }
                if message.message == WM_STOP {
                    break;
                }
                if message.message == WM_HOTKEY && message.wParam.0 == HOTKEY_ID as usize {
                    let _ = events.try_send(AppEvent::Toggle);
                }
            }

            unsafe {
                let _ = UnregisterHotKey(None, HOTKEY_ID);
            }
        }
        Err(error) => {
            let _ = ready.send(Err(format!(
                "could not register hotkey {}: {}. It may already be in use by another application.",
                spec.display(),
                error.message()
            )));
        }
    }
}
