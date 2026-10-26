use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use async_channel::{Receiver, Sender};
use gpui_kit::component::{
    Icon, IconName, Sizable as _,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{
    config::{Config, UsageStore},
    events::AppEvent,
    model::{Action, CommandId, Entry, EntryIcon},
    search::apps::AppRecord,
    search::{self, SearchOutcome},
    win,
};

const PANEL_BG: u32 = 0x0F1116F5;
const ACCENT: u32 = 0x8AB4F8;
const TEXT: u32 = 0xF3F6FC;
const MUTED: u32 = 0x9BA4B4;

pub struct Launcher {
    input: Entity<InputState>,
    config: Config,
    usage: UsageStore,
    apps: Vec<AppRecord>,
    app_icons: HashMap<PathBuf, PathBuf>,
    app_entries: Arc<Vec<Entry>>,
    file_entries: Arc<Vec<Entry>>,
    command_entries: Arc<Vec<Entry>>,
    results: SearchOutcome,
    selected: usize,
    query: String,
    autostart: bool,
    indexing: Option<usize>,
    indexed_files: usize,
    status: Option<String>,
    hotkey_note: Option<String>,
    search_generation: u64,
    search_task: Option<Task<()>>,
    event_task: Option<Task<()>>,
    focus_task: Option<Task<()>>,
    _input_sub: Option<Subscription>,
    _intercept_sub: Option<Subscription>,
    last_shown: Instant,
    was_foreground: bool,
    tx: Sender<AppEvent>,
}

impl Launcher {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        config: Config,
        rx: Receiver<AppEvent>,
        tx: Sender<AppEvent>,
        hotkey_note: Option<String>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search apps, files, and commands…")
                .context_menu(false)
        });

        let input_sub = cx.subscribe_in(
            &input,
            window,
            |this, state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.query = state.read(cx).value().to_string();
                    this.selected = 0;
                    this.status = None;
                    this.schedule_search(window, cx);
                }
                InputEvent::PressEnter { secondary, .. } => {
                    this.activate(*secondary, window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        );

        let apps = Vec::new();
        let autostart = win::launch::autostart_enabled();
        let command_entries = Arc::new(search::engine::command_entries(autostart));

        let mut launcher = Self {
            input,
            config,
            usage: UsageStore::load(),
            apps,
            app_icons: HashMap::new(),
            app_entries: Arc::new(Vec::new()),
            file_entries: Arc::new(Vec::new()),
            command_entries,
            results: SearchOutcome::default(),
            selected: 0,
            query: String::new(),
            autostart,
            indexing: None,
            indexed_files: 0,
            status: None,
            hotkey_note,
            search_generation: 0,
            search_task: None,
            event_task: None,
            focus_task: None,
            _input_sub: Some(input_sub),
            _intercept_sub: None,
            last_shown: Instant::now(),
            was_foreground: false,
            tx,
        };

        if let Some(hwnd) = win::window::hwnd_of(window) {
            win::window::polish(hwnd);
            win::window::position_top_center_on_cursor_monitor(hwnd);
        }

        let sources = [
            launcher.app_entries.as_slice(),
            launcher.file_entries.as_slice(),
            launcher.command_entries.as_slice(),
        ];
        launcher.results =
            search::engine::suggestions_sources(&sources, &launcher.usage, launcher.result_limit());

        launcher
            .input
            .update(cx, |state, cx| state.focus(window, cx));

        let weak = cx.entity().downgrade();
        let this_window = window.window_handle();
        launcher._intercept_sub = Some(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != this_window {
                return;
            }

            let key = event.keystroke.key.as_str();
            let modifiers = event.keystroke.modifiers;
            let Some(launcher) = weak.upgrade() else {
                return;
            };

            match key {
                "up" if !modifiers.modified() => {
                    launcher.update(cx, |this, cx| this.select(-1, cx));
                    cx.stop_propagation();
                }
                "down" if !modifiers.modified() => {
                    launcher.update(cx, |this, cx| this.select(1, cx));
                    cx.stop_propagation();
                }
                "escape" => {
                    launcher.update(cx, |this, cx| this.on_escape(window, cx));
                    cx.stop_propagation();
                }
                "enter" if !modifiers.alt && !modifiers.shift && !modifiers.platform => {
                    launcher.update(cx, |this, cx| this.activate(modifiers.control, window, cx));
                    cx.stop_propagation();
                }
                _ => {}
            }
        }));

        launcher.event_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                if this
                    .update_in(cx, |this, window, cx| this.handle_event(event, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));

        launcher.focus_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(180))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.poll_focus(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));

        window.on_window_should_close(cx, |window, _| {
            if let Some(hwnd) = win::window::hwnd_of(window) {
                win::window::hide(hwnd);
            }
            false
        });

        let apps_tx = launcher.tx.clone();
        let file_tx = launcher.tx.clone();
        let icons_dir = crate::config::icons_dir();
        let file_settings = launcher.config.index.clone();
        std::thread::Builder::new()
            .name("luma-startup".into())
            .spawn(move || {
                // Give the first frame time to present before the shell/COM app
                // scan and file cache load compete for CPU. This also preloads
                // data when the app was started hidden at login.
                std::thread::sleep(Duration::from_millis(800));

                std::thread::Builder::new()
                    .name("luma-apps".into())
                    .spawn(move || scan_apps_and_icons(apps_tx, icons_dir))
                    .ok();

                if file_settings.enabled {
                    std::thread::Builder::new()
                        .name("luma-files".into())
                        .spawn(move || load_and_index_files(file_tx, file_settings))
                        .ok();
                }
            })
            .ok();

        launcher
    }

    fn result_limit(&self) -> usize {
        self.config.max_results.clamp(1, 8)
    }

    fn refresh_app_entries(&mut self) {
        self.app_entries = Arc::new(search::engine::app_entries(&self.apps, &self.app_icons));
        self.search_generation = self.search_generation.wrapping_add(1);
    }

    fn show_suggestions(&mut self, cx: &mut Context<Self>) {
        let sources = [
            self.app_entries.as_slice(),
            self.file_entries.as_slice(),
            self.command_entries.as_slice(),
        ];
        self.results =
            search::engine::suggestions_sources(&sources, &self.usage, self.result_limit());
        self.selected = self.selected.min(self.results.hits.len().saturating_sub(1));
        cx.notify();
    }

    fn refresh_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_task = None;
        if self.query.trim().is_empty() {
            self.show_suggestions(cx);
        } else {
            self.schedule_search(window, cx);
        }
    }

    fn schedule_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        let app_entries = self.app_entries.clone();
        let file_entries = self.file_entries.clone();
        let command_entries = self.command_entries.clone();
        let usage = self.usage.clone();
        let query = self.query.clone();
        let limit = self.result_limit();

        self.search_task = Some(cx.spawn_in(window, async move |this, cx| {
            if !query.trim().is_empty() {
                cx.background_executor()
                    .timer(Duration::from_millis(55))
                    .await;
            }

            let outcome = cx
                .background_spawn(async move {
                    let sources = [
                        app_entries.as_slice(),
                        file_entries.as_slice(),
                        command_entries.as_slice(),
                    ];
                    search::engine::search_sources(&sources, &query, &usage, limit)
                })
                .await;

            _ = this.update_in(cx, |this, _window, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.results = outcome;
                this.selected = this.selected.min(this.results.hits.len().saturating_sub(1));
                cx.notify();
            });
        }));

        cx.notify();
    }

    fn select(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.results.hits.len();
        if len == 0 {
            return;
        }
        let current = self.selected.min(len - 1);
        self.selected = if delta < 0 {
            (current + len - 1) % len
        } else {
            (current + 1) % len
        };
        self.status = None;
        cx.notify();
    }

    fn on_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query.is_empty() {
            self.hide(window, cx);
        } else {
            self.clear_query(window, cx);
        }
    }

    fn clear_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.query.clear();
        self.selected = 0;
        self.status = None;
        self.show_suggestions(cx);
    }

    fn reset_for_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.query.clear();
        self.selected = 0;
        self.status = None;
        self.show_suggestions(cx);
    }

    fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hwnd) = win::window::hwnd_of(window) else {
            return;
        };

        if win::window::is_visible(hwnd) && win::window::is_foreground(hwnd) {
            self.hide(window, cx);
            return;
        }

        if !win::window::is_visible(hwnd) {
            win::window::position_top_center_on_cursor_monitor(hwnd);
        }

        win::window::show_and_activate(hwnd);
        window.activate_window();
        self.last_shown = Instant::now();
        self.was_foreground = false;

        if self.query.is_empty() {
            self.focus_input(window, cx);
        } else {
            self.reset_for_open(window, cx);
        }
        cx.notify();
    }

    fn hide(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(hwnd) = win::window::hwnd_of(window) {
            win::window::hide(hwnd);
        }
        self.usage.save();
    }

    fn poll_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hwnd) = win::window::hwnd_of(window) else {
            return;
        };
        if !win::window::is_visible(hwnd) {
            return;
        }
        if win::window::is_foreground(hwnd) {
            self.was_foreground = true;
            return;
        }
        if self.was_foreground && self.last_shown.elapsed() >= Duration::from_millis(350) {
            win::window::hide(hwnd);
            self.was_foreground = false;
            self.usage.save();
            cx.notify();
        }
    }

    fn handle_event(&mut self, event: AppEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            AppEvent::Toggle => self.toggle(window, cx),
            AppEvent::FilesProgress { count } => {
                self.indexing = Some(count);
                cx.notify();
            }
            AppEvent::FilesLoaded {
                entries,
                count,
                needs_rebuild,
            } => {
                self.file_entries = Arc::new(entries);
                self.indexed_files = count;
                self.indexing = needs_rebuild.then_some(0);
                self.refresh_results(window, cx);
            }
            AppEvent::FilesIndexed {
                entries,
                count,
                elapsed_ms,
            } => {
                self.file_entries = Arc::new(entries);
                self.indexing = None;
                self.indexed_files = count;
                let seconds = elapsed_ms as f64 / 1000.0;
                self.status = Some(format!("Indexed {} items in {:.1}s", count, seconds));
                self.refresh_results(window, cx);
            }
            AppEvent::FilesFailed(error) => {
                self.indexing = None;
                self.status = Some(format!("File index failed: {error}"));
                cx.notify();
            }
            AppEvent::AppsReady { apps } => {
                self.apps = apps;
                self.refresh_app_entries();
                self.refresh_results(window, cx);
            }
            AppEvent::AppIconsReady { icons } => {
                self.app_icons.clear();
                for (app, icon) in icons {
                    self.app_icons.insert(app, icon);
                }
                self.refresh_app_entries();
                self.refresh_results(window, cx);
            }
        }
    }

    fn activate(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.results.hits.get(self.selected).cloned() else {
            return;
        };
        let entry = hit.entry;
        self.usage.record_use(&entry.usage_key);
        self.usage.save();

        match &entry.action {
            Action::CopyText(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.hide(window, cx);
            }
            Action::Command(id) => {
                let id = *id;
                self.run_command(id, window, cx);
            }
            Action::OpenPath(path) if secondary => {
                self.hide(window, cx);
                if let Err(error) = win::launch::reveal(path) {
                    self.recover(error, window, cx);
                }
            }
            action => {
                self.hide(window, cx);
                if let Err(error) = win::launch::open_action(action) {
                    self.recover(error, window, cx);
                }
            }
        }
    }

    fn run_command(&mut self, id: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            CommandId::Quit => {
                self.usage.save();
                let _ = self.config.save();
                cx.quit();
            }
            CommandId::ReindexFiles => {
                if self.indexing.is_some() {
                    self.status = Some("The index is already being rebuilt".into());
                    cx.notify();
                    return;
                }
                self.status = Some("Rebuilding the file index…".into());
                self.start_index_build();
                self.refresh_results(window, cx);
            }
            CommandId::ToggleAutostart => {
                let enabled = !self.autostart;
                match win::launch::set_autostart(enabled) {
                    Ok(()) => {
                        self.autostart = enabled;
                        self.status = Some(if enabled {
                            "Luma will start with Windows".into()
                        } else {
                            "Luma will no longer start with Windows".into()
                        });
                        self.command_entries = Arc::new(search::engine::command_entries(enabled));
                        self.search_generation = self.search_generation.wrapping_add(1);
                        self.refresh_results(window, cx);
                    }
                    Err(error) => {
                        self.status = Some(error);
                        cx.notify();
                    }
                }
            }
            CommandId::EditConfig => {
                let _ = self.config.save();
                let path = crate::config::config_path();
                self.hide(window, cx);
                if let Err(error) = win::launch::open_path(&path) {
                    self.recover(error, window, cx);
                }
            }
            CommandId::OpenDataFolder => {
                let _ = crate::config::ensure_data_dirs();
                self.hide(window, cx);
                if let Err(error) = win::launch::open_path(&crate::config::data_dir()) {
                    self.recover(error, window, cx);
                }
            }
            CommandId::OpenSettings => {
                self.hide(window, cx);
                if let Err(error) = win::launch::open_shell("ms-settings:") {
                    self.recover(error, window, cx);
                }
            }
            CommandId::OpenRecycleBin => {
                self.hide(window, cx);
                if let Err(error) = win::launch::open_shell("shell:RecycleBinFolder") {
                    self.recover(error, window, cx);
                }
            }
            CommandId::Lock => {
                self.hide(window, cx);
                if let Err(error) = win::launch::lock_workstation() {
                    self.recover(error, window, cx);
                }
            }
            CommandId::TaskManager => {
                self.hide(window, cx);
                if let Err(error) = win::launch::run_program("taskmgr.exe", None, false) {
                    self.recover(error, window, cx);
                }
            }
            CommandId::OpenExplorer => self.launch_program("explorer.exe", window, cx),
            CommandId::OpenTerminal => self.launch_program("wt.exe", window, cx),
            CommandId::OpenCalculator => self.launch_program("calc.exe", window, cx),
            CommandId::OpenNotepad => self.launch_program("notepad.exe", window, cx),
            CommandId::OpenPaint => self.launch_program("mspaint.exe", window, cx),
            CommandId::OpenHome => self.open_directory(dirs::home_dir(), window, cx),
            CommandId::OpenDesktop => self.open_directory(
                dirs::desktop_dir().or_else(|| dirs::home_dir().map(|home| home.join("Desktop"))),
                window,
                cx,
            ),
            CommandId::OpenDocuments => self.open_directory(
                dirs::document_dir()
                    .or_else(|| dirs::home_dir().map(|home| home.join("Documents"))),
                window,
                cx,
            ),
            CommandId::OpenDownloads => self.open_directory(
                dirs::download_dir()
                    .or_else(|| dirs::home_dir().map(|home| home.join("Downloads"))),
                window,
                cx,
            ),
            CommandId::OpenPictures => self.open_directory(
                dirs::picture_dir().or_else(|| dirs::home_dir().map(|home| home.join("Pictures"))),
                window,
                cx,
            ),
        }
    }

    fn launch_program(&mut self, program: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.hide(window, cx);
        if let Err(error) = win::launch::run_program(program, None, false) {
            self.recover(error, window, cx);
        }
    }

    fn open_directory(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = path else {
            self.status = Some("That folder could not be found".into());
            cx.notify();
            return;
        };
        self.hide(window, cx);
        if let Err(error) = win::launch::open_path(&path) {
            self.recover(error, window, cx);
        }
    }

    fn recover(&mut self, error: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hwnd) = win::window::hwnd_of(window) {
            win::window::position_top_center_on_cursor_monitor(hwnd);
            win::window::show_and_activate(hwnd);
            window.activate_window();
        }
        self.last_shown = Instant::now();
        self.status = Some(error);
        self.focus_input(window, cx);
        cx.notify();
    }

    fn start_index_build(&mut self) {
        if !self.config.index.enabled {
            return;
        }
        if self.indexing.is_some() {
            return;
        }
        self.indexing = Some(0);
        self.status = None;

        let tx = self.tx.clone();
        let settings = self.config.index.clone();
        std::thread::Builder::new()
            .name("luma-index".into())
            .spawn(move || index_files(tx, settings))
            .ok();
    }

    fn status_text(&self) -> String {
        if let Some(count) = self.indexing {
            if count == 0 {
                return "Indexing your files…".into();
            }
            return format!("Indexing your files · {} found", group_count(count));
        }
        if let Some(status) = &self.status {
            return status.clone();
        }
        if let Some(note) = &self.hotkey_note {
            return note.clone();
        }
        if self.query.trim().is_empty() {
            if self.indexed_files > 0 {
                return format!(
                    "{} items indexed · type to search",
                    group_count(self.indexed_files)
                );
            }
            return "Type to search apps, files, and commands".into();
        }
        let count = group_count(self.results.total);
        let result_label = if self.results.total == 1 {
            "result"
        } else {
            "results"
        };
        if self.indexed_files > 0 {
            let item_label = if self.indexed_files == 1 {
                "item"
            } else {
                "items"
            };
            format!(
                "{count} {result_label} · {} {item_label} indexed",
                group_count(self.indexed_files)
            )
        } else {
            format!("{count} {result_label}")
        }
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self
            .results
            .hits
            .iter()
            .enumerate()
            .map(|(index, hit)| {
                let entry = hit.entry.clone();
                let selected = index == self.selected;
                let click = cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.selected = index;
                    this.activate(false, window, cx);
                });

                let trailing = if selected {
                    keycap("Enter")
                } else {
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(MUTED))
                        .child(entry.kind.label())
                        .into_any_element()
                };

                div()
                    .id(("luma-result", hit.index))
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(50.))
                    .mx(px(6.))
                    .px(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgba(0x00000000))
                    .when(selected, |row| {
                        row.bg(rgba(0x8AB4F81C)).border_color(rgba(0x8AB4F830))
                    })
                    .when(!selected, |row| {
                        row.hover(|style| style.bg(rgba(0xFFFFFF08)))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(click)
                    .child(icon_tile(&entry.icon))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .truncate()
                                    .child(SharedString::from(entry.name.clone())),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .truncate()
                                    .child(SharedString::from(entry.subtitle.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .min_w(px(70.))
                            .child(trailing),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let header = if self.results.hits.is_empty() {
            None
        } else {
            Some(
                div()
                    .h(px(20.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .text_size(px(9.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0x7E8797))
                    .child(if self.query.is_empty() {
                        "SUGGESTIONS"
                    } else {
                        "RESULTS"
                    }),
            )
        };

        let results_area = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .px(px(6.))
            .py(px(6.))
            .when_some(header, |area, header| area.child(header))
            .when(self.results.hits.is_empty(), |area| {
                area.child(empty_state(&self.query))
            })
            .children(rows);

        let leading = if self.indexing.is_some() {
            Spinner::new()
                .with_size(px(11.))
                .color(rgb(ACCENT).into())
                .into_any_element()
        } else {
            div()
                .size(px(6.))
                .rounded_full()
                .bg(rgb(ACCENT))
                .into_any_element()
        };

        let footer = div()
            .flex()
            .items_center()
            .gap_3()
            .h(px(40.))
            .px(px(16.))
            .border_t_1()
            .border_color(rgba(0xFFFFFF08))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .min_w_0()
                    .child(leading)
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(ACCENT))
                            .child("LUMA"),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(self.status_text()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(hint("↑↓", "Navigate"))
                    .child(hint("Enter", "Open"))
                    .child(hint("Ctrl Enter", "Reveal"))
                    .child(hint("Esc", "Close")),
            );

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .rounded(px(18.))
            .bg(rgba(PANEL_BG))
            .border_1()
            .border_color(rgba(0xFFFFFF14))
            .shadow(vec![
                BoxShadow::new(px(0.), px(24.), hsla(0., 0., 0., 0.52))
                    .blur_radius(px(56.))
                    .spread_radius(px(-8.)),
                BoxShadow::new(px(0.), px(2.), hsla(0., 0., 0., 0.35)).blur_radius(px(10.)),
            ])
            .overflow_hidden()
            .font_family("Segoe UI Variable Text")
            .text_color(rgb(TEXT))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().h(px(1.)).w_full().bg(rgba(0x8AB4F82E)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(64.))
                    .px(px(20.))
                    .child(
                        Icon::new(IconName::Search)
                            .with_size(px(20.))
                            .text_color(rgb(MUTED)),
                    )
                    .child(
                        div().flex_1().min_w_0().text_size(px(18.)).child(
                            Input::new(&self.input)
                                .appearance(false)
                                .cleanable(true)
                                .h(px(40.)),
                        ),
                    )
                    .child(keycap("ESC")),
            )
            .child(div().h(px(1.)).mx(px(16.)).bg(rgba(0xFFFFFF0A)))
            .child(results_area)
            .child(footer)
            .into_any_element()
    }
}

impl Render for Launcher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = self.render_panel(cx);
        div()
            .size_full()
            .p(px(18.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.hide(window, cx);
                }),
            )
            .child(panel)
    }
}

fn scan_apps_and_icons(tx: Sender<AppEvent>, icons_dir: PathBuf) {
    win::thread::lower_current_thread_priority();
    let apps = search::apps::scan_apps();
    let _ = tx.send_blocking(AppEvent::AppsReady { apps: apps.clone() });
    let icons = search::apps::load_app_icons(&apps, &icons_dir);
    let _ = tx.send_blocking(AppEvent::AppIconsReady { icons });
}

fn load_and_index_files(tx: Sender<AppEvent>, settings: crate::config::IndexSettings) {
    win::thread::lower_current_thread_priority();
    let (records, needs_rebuild) = match search::files::load_cached(&settings) {
        Some((records, built_at)) => {
            let needs_rebuild = !search::files::cache_is_fresh(built_at, settings.ttl_hours);
            (records, needs_rebuild)
        }
        None => (Vec::new(), true),
    };

    let entries = search::engine::file_entries(&records);
    let count = entries.len();
    if tx
        .send_blocking(AppEvent::FilesLoaded {
            entries,
            count,
            needs_rebuild,
        })
        .is_err()
    {
        return;
    }

    if needs_rebuild {
        index_files(tx, settings);
    }
}

fn index_files(tx: Sender<AppEvent>, settings: crate::config::IndexSettings) {
    win::thread::lower_current_thread_priority();
    let progress_tx = tx.clone();
    let started = Instant::now();
    let result = search::files::build_file_index(&settings, move |count| {
        let _ = progress_tx.send_blocking(AppEvent::FilesProgress { count });
    });

    match result {
        Ok(records) => {
            let built_at = crate::config::unix_now();
            let _ = search::files::save_cache(&records, built_at);
            let entries = search::engine::file_entries(&records);
            let count = entries.len();
            let _ = tx.send_blocking(AppEvent::FilesIndexed {
                entries,
                count,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        Err(error) => {
            let _ = tx.send_blocking(AppEvent::FilesFailed(error));
        }
    }
}

fn icon_tile(icon: &EntryIcon) -> AnyElement {
    match icon {
        EntryIcon::App {
            icon_path: Some(path),
            ..
        } => img(path.clone())
            .w(px(32.))
            .h(px(32.))
            .rounded(px(8.))
            .into_any_element(),
        EntryIcon::App { letter, color, .. } => {
            let fill = (*color << 8) | 0x2E;
            let border = (*color << 8) | 0x55;
            div()
                .size(px(32.))
                .rounded(px(8.))
                .bg(rgba(fill))
                .border_1()
                .border_color(rgba(border))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(*color))
                        .child(letter.to_string()),
                )
                .into_any_element()
        }
        EntryIcon::Glyph(name) => div()
            .size(px(32.))
            .rounded(px(8.))
            .bg(rgba(0xFFFFFF0A))
            .border_1()
            .border_color(rgba(0xFFFFFF0F))
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(name.clone())
                    .with_size(px(17.))
                    .text_color(rgb(MUTED)),
            )
            .into_any_element(),
    }
}

fn keycap(label: impl Into<SharedString>) -> AnyElement {
    div()
        .h(px(18.))
        .px(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgba(0xFFFFFF14))
        .bg(rgba(0xFFFFFF08))
        .text_size(px(9.5))
        .text_color(rgb(MUTED))
        .flex()
        .items_center()
        .child(label.into())
        .into_any_element()
}

fn hint(key: &str, label: &str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(keycap(key))
        .child(
            div()
                .text_size(px(10.))
                .text_color(rgb(MUTED))
                .child(label.to_string()),
        )
        .into_any_element()
}

fn empty_state(query: &str) -> AnyElement {
    let title = if query.trim().is_empty() {
        "Start typing to search".to_string()
    } else {
        format!("No results for “{query}”")
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .py(px(20.))
        .child(
            Icon::new(IconName::Search)
                .with_size(px(28.))
                .text_color(rgba(0xFFFFFF28)),
        )
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(title),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child("Try an app, file, folder, or command name"),
        )
        .into_any_element()
}

fn group_count(value: usize) -> String {
    let text = value.to_string();
    let mut output = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(ch);
    }
    output
}
