mod config;
mod events;
mod model;
mod search;
mod ui;
mod win;

use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::{
    config::Config,
    ui::Launcher,
    win::hotkey::{HotkeyService, HotkeySpec},
    win::single_instance::{InstanceGuard, acquire_instance},
};

static WINDOW_COUNTER: AtomicU64 = AtomicU64::new(0);

fn main() {
    if let Err(error) = config::ensure_data_dirs() {
        eprintln!("luma: could not create the data directory: {error}");
    }

    let config = Config::load_or_default();
    let start_hidden = std::env::args().any(|argument| argument == "--hidden");

    match acquire_instance() {
        Ok(InstanceGuard::Primary(guard)) => std::mem::forget(guard),
        Ok(InstanceGuard::Secondary) => {
            return;
        }
        Err(error) => {
            eprintln!("luma: {error}");
        }
    }

    let (tx, rx) = async_channel::bounded(256);

    let requested = HotkeySpec::parse(&config.hotkey).unwrap_or_else(HotkeySpec::default);
    let (hotkey_note, hotkey_service) = match HotkeyService::spawn(requested, tx.clone()) {
        Ok(service) => (None, Some(service)),
        Err(primary_error) => {
            let fallback = HotkeySpec::parse("ctrl+alt+space").expect("hard-coded hotkey");
            match HotkeyService::spawn(fallback, tx.clone()) {
                Ok(service) => (
                    Some(format!(
                        "{} is unavailable ({}); using {}",
                        requested.display(),
                        primary_error,
                        fallback.display()
                    )),
                    Some(service),
                ),
                Err(fallback_error) => (
                    Some(format!("Global hotkey unavailable: {fallback_error}")),
                    None,
                ),
            }
        }
    };
    std::mem::forget(hotkey_service);

    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        Theme::change(ThemeMode::Dark, None, cx);

        let options = launcher_window_options(cx, !start_hidden);
        let config_for_view = config.clone();
        let note_for_view = hotkey_note.clone();

        let handle = cx
            .open_window(options, |window, cx| {
                cx.new(|cx| {
                    Launcher::new(window, cx, config_for_view, rx, tx.clone(), note_for_view)
                })
            })
            .expect("failed to open the Luma window");

        if !start_hidden {
            let _ = handle.update(cx, |_, window, _| {
                window.activate_window();
            });
        }
    });
}

fn launcher_window_options(cx: &mut App, show: bool) -> WindowOptions {
    let width = px(760.);
    let height = px(576.);

    let (bounds, display_id) = match cx.primary_display() {
        Some(display) => {
            let visible = display.visible_bounds();
            let origin = point(visible.center().x - width / 2.0, visible.origin.y + px(84.));
            (
                WindowBounds::Windowed(Bounds {
                    origin,
                    size: size(width, height),
                }),
                Some(display.id()),
            )
        }
        None => (WindowBounds::centered(size(width, height), cx), None),
    };

    WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed);

    WindowOptions {
        window_bounds: Some(bounds),
        titlebar: None,
        focus: show,
        show,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some("dev.luma.launcher".into()),
        ..Default::default()
    }
}
