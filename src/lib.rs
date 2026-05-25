use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use serde::{Deserialize, Serialize};

mod ui;

const APP_DIR: &str = "window-jump";
const CONFIG_FILE: &str = "config.json";
const STATE_FILE: &str = "state.json";
const LOCK_FILE: &str = ".window-jump.lock";
const SLOT_MIN: u8 = 1;
const SLOT_MAX: u8 = 9;
const REFRESH_INTERVAL: Duration = Duration::from_millis(1000);
const REPAINT_INTERVAL: Duration = Duration::from_millis(250);
const GUI_COMPACT_INNER_SIZE: [f32; 2] = [1120.0, 860.0];
const GUI_EXPANDED_INNER_SIZE: [f32; 2] = [1450.0, 860.0];
const GUI_MIN_INNER_SIZE: [f32; 2] = [980.0, 720.0];

#[derive(Parser, Debug)]
#[command(name = "window-jump")]
#[command(version)]
#[command(about = "X11 window slot jumper for Ubuntu GNOME")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Activate the window assigned to a slot.
    Activate {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=9))]
        slot: u8,
    },
    /// Capture the current active window into a slot.
    CaptureActive {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=9))]
        slot: u8,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        title_contains: Option<String>,
    },
    /// Ask the user to click a window and capture it into a slot.
    CaptureClick {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=9))]
        slot: u8,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        title_contains: Option<String>,
    },
    /// Remove the rule stored in a slot.
    Clear {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=9))]
        slot: u8,
    },
    /// Print all currently visible top-level windows.
    ListWindows,
    /// Print the active window details.
    InspectActive,
    /// Print stored slot rules.
    ListSlots,
    /// Print the GNOME Custom Shortcuts commands for slots 1..9.
    ShortcutSpecs,
    /// Check which Japanese UI font will be used.
    CheckFonts,
}

pub fn run_cli() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Activate { slot }) => cmd_activate(slot),
        Some(Commands::CaptureActive {
            slot,
            label,
            title_contains,
        }) => cmd_capture_active(slot, label, title_contains),
        Some(Commands::CaptureClick {
            slot,
            label,
            title_contains,
        }) => cmd_capture_click(slot, label, title_contains),
        Some(Commands::Clear { slot }) => cmd_clear(slot),
        Some(Commands::ListWindows) => cmd_list_windows(),
        Some(Commands::InspectActive) => cmd_inspect_active(),
        Some(Commands::ListSlots) => cmd_list_slots(),
        Some(Commands::ShortcutSpecs) => cmd_shortcut_specs(),
        Some(Commands::CheckFonts) => cmd_check_fonts(),
        None => {
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}

pub fn run_gui() -> Result<()> {
    let (config, state, read_only, startup_status) = match load_store() {
        Ok((config, state)) => {
            let status = format!(
                "設定を読み込みました: {} / {}",
                config_path()?.display(),
                state_path()?.display()
            );
            (config, state, false, status)
        }
        Err(err) => (
            AppConfig::default(),
            AppState::default(),
            true,
            format!("設定読み込みに失敗したため read-only で起動しました。保存は無効です: {err}"),
        ),
    };

    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(GUI_COMPACT_INNER_SIZE)
            .with_min_inner_size(GUI_MIN_INNER_SIZE),
        ..Default::default()
    };

    eframe::run_native(
        "Window Jump",
        native_options,
        Box::new(move |cc| {
            let font_check = ui::configure(&cc.egui_ctx);
            Ok(Box::new(WindowJumpApp::new(
                config,
                state,
                read_only,
                startup_status,
                font_check.status_line(),
            )))
        }),
    )
    .map_err(|err| anyhow!("GUI の起動に失敗しました: {err}"))
}

fn cmd_activate(slot: u8) -> Result<()> {
    let (config, mut state) = load_store().with_context(|| "設定ファイルを読み込めませんでした")?;
    let rule = config
        .slots
        .get(&slot)
        .cloned()
        .ok_or_else(|| anyhow!("slot {slot} は未登録です"))?;

    if !rule.is_usable() {
        bail!("slot {slot} のルールが不完全です。GUI から class を設定してください");
    }

    let windows = list_windows()?;
    let slot_state = state.slots.get(&slot);
    let target = resolve_slot(&rule, slot_state, &windows)?.clone();
    activate_window(&target)?;

    update_slot_state(&mut state, slot, &target);
    save_state(&state)?;

    println!(
        "slot {} -> {} [{}] {}",
        slot, target.id_hex, target.wm_class, target.title
    );
    Ok(())
}

fn cmd_capture_active(
    slot: u8,
    label: Option<String>,
    title_contains: Option<String>,
) -> Result<()> {
    let window = active_window()?;
    let (mut config, mut state) = load_store()?;
    let windows = list_windows().unwrap_or_default();
    let rule = apply_capture(
        &mut config,
        &mut state,
        slot,
        &window,
        &windows,
        label,
        title_contains,
    );
    save_store(&config, &state)?;

    println!(
        "slot {} を更新しました: {} [{}] {}",
        slot, window.id_hex, rule.wm_class, window.title
    );
    Ok(())
}

fn cmd_capture_click(
    slot: u8,
    label: Option<String>,
    title_contains: Option<String>,
) -> Result<()> {
    eprintln!("slot {} 用の対象ウィンドウをクリックしてください…", slot);
    let selected_id = choose_window_by_click()?;
    let window = window_by_id(selected_id)?;

    let (mut config, mut state) = load_store()?;
    let windows = list_windows().unwrap_or_default();
    let rule = apply_capture(
        &mut config,
        &mut state,
        slot,
        &window,
        &windows,
        label,
        title_contains,
    );
    save_store(&config, &state)?;

    println!(
        "slot {} を更新しました: {} [{}] {}",
        slot, window.id_hex, rule.wm_class, window.title
    );
    Ok(())
}

fn cmd_clear(slot: u8) -> Result<()> {
    let (mut config, mut state) = load_store()?;
    config.slots.remove(&slot);
    state.slots.remove(&slot);
    save_store(&config, &state)?;
    println!("slot {} を削除しました", slot);
    Ok(())
}

fn cmd_list_windows() -> Result<()> {
    let windows = list_windows()?;
    if windows.is_empty() {
        println!("ウィンドウは見つかりませんでした");
        return Ok(());
    }

    println!(
        "{:<12} {:<4} {:<7} {:<28} {:<18} TITLE",
        "WINDOW_ID", "DSK", "PID", "WM_CLASS", "GEOMETRY"
    );
    for window in windows {
        println!(
            "{:<12} {:<4} {:<7} {:<28} {:<18} {}",
            window.id_hex,
            window.desktop,
            window
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            truncate(&window.wm_class, 28),
            format!(
                "{}x{}+{}+{}",
                window.width, window.height, window.x, window.y
            ),
            window.title
        );
    }
    Ok(())
}

fn cmd_inspect_active() -> Result<()> {
    let window = active_window()?;
    println!("window_id     : {}", window.id_hex);
    println!("desktop       : {}", window.desktop);
    println!(
        "pid           : {}",
        window
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!("wm_class      : {}", window.wm_class);
    println!(
        "geometry      : {}x{}+{}+{}",
        window.width, window.height, window.x, window.y
    );
    println!("title         : {}", window.title);
    Ok(())
}

fn cmd_list_slots() -> Result<()> {
    let (config, state) = load_store()?;
    if config.slots.is_empty() {
        println!("登録済み slot はありません");
        return Ok(());
    }

    for slot in SLOT_MIN..=SLOT_MAX {
        if let Some(rule) = config.slots.get(&slot) {
            println!("slot {}", slot);
            println!("  label          : {}", empty_dash(&rule.label));
            println!("  wm_class       : {}", empty_dash(&rule.wm_class));
            println!("  title_contains : {}", empty_dash(&rule.title_contains));
            println!(
                "  desktop_hint   : {}",
                state
                    .slots
                    .get(&slot)
                    .and_then(|slot_state| slot_state.desktop_hint)
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            println!(
                "  last_window_id : {}",
                state
                    .slots
                    .get(&slot)
                    .and_then(|slot_state| slot_state.last_window_id.clone())
                    .clone()
                    .unwrap_or_else(|| "-".to_string())
            );
            println!(
                "  last_seen_title: {}",
                state
                    .slots
                    .get(&slot)
                    .and_then(|slot_state| slot_state.last_seen_title.clone())
                    .clone()
                    .unwrap_or_else(|| "-".to_string())
            );
            println!("  notes          : {}", empty_dash(&rule.notes));
        }
    }
    Ok(())
}

fn cmd_shortcut_specs() -> Result<()> {
    let exe = shortcut_executable_hint();
    println!("GNOME Custom Shortcuts に以下を設定してください。\n");
    for slot in SLOT_MIN..=SLOT_MAX {
        println!(
            "Ctrl+Alt+{} -> {} activate {}",
            slot,
            shell_quote(&exe),
            slot
        );
    }
    Ok(())
}

fn cmd_check_fonts() -> Result<()> {
    let font_check = ui::detect_system_cjk_font();
    println!("{}", font_check.status_line());
    if let Some(path) = font_check.font_path {
        println!("font_path: {}", path.display());
    } else {
        println!("font_path: -");
    }
    Ok(())
}

fn version_one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default = "version_one")]
    version: u32,
    #[serde(default)]
    slots: BTreeMap<u8, SlotRule>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            slots: BTreeMap::new(),
        }
    }
}

impl AppConfig {
    fn set_slot(&mut self, slot: u8, rule: SlotRule) {
        if rule.is_effectively_empty() {
            self.slots.remove(&slot);
        } else {
            self.slots.insert(slot, rule);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SlotRule {
    #[serde(default)]
    label: String,
    #[serde(default)]
    wm_class: String,
    #[serde(default)]
    title_contains: String,
    #[serde(default)]
    notes: String,
}

impl SlotRule {
    fn is_effectively_empty(&self) -> bool {
        self.label.trim().is_empty()
            && self.wm_class.trim().is_empty()
            && self.title_contains.trim().is_empty()
            && self.notes.trim().is_empty()
    }

    fn is_usable(&self) -> bool {
        !self.wm_class.trim().is_empty()
    }

    fn summary(&self) -> String {
        if self.is_effectively_empty() {
            return "<empty>".to_string();
        }

        let mut parts = Vec::new();
        if !self.label.trim().is_empty() {
            parts.push(format!("label={}", self.label.trim()));
        }
        if !self.wm_class.trim().is_empty() {
            parts.push(format!("class={}", self.wm_class.trim()));
        }
        if !self.title_contains.trim().is_empty() {
            parts.push(format!("title~={}", self.title_contains.trim()));
        }
        parts.join(" | ")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppState {
    #[serde(default = "version_one")]
    version: u32,
    #[serde(default)]
    slots: BTreeMap<u8, SlotState>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: 1,
            slots: BTreeMap::new(),
        }
    }
}

impl AppState {
    fn set_slot_state(&mut self, slot: u8, state: SlotState) {
        if state.is_empty() {
            self.slots.remove(&slot);
        } else {
            self.slots.insert(slot, state);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SlotState {
    #[serde(default)]
    desktop_hint: Option<i32>,
    #[serde(default)]
    last_window_id: Option<String>,
    #[serde(default)]
    last_seen_title: Option<String>,
}

impl SlotState {
    fn is_empty(&self) -> bool {
        self.desktop_hint.is_none()
            && self.last_window_id.is_none()
            && self.last_seen_title.is_none()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacyConfig {
    #[serde(default)]
    slots: BTreeMap<u8, LegacySlotRule>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LegacySlotRule {
    #[serde(default)]
    desktop_hint: Option<i32>,
    #[serde(default)]
    last_window_id: Option<String>,
    #[serde(default)]
    last_seen_title: Option<String>,
}

#[derive(Debug, Clone)]
struct WindowInfo {
    id: u64,
    id_hex: String,
    desktop: i32,
    pid: Option<u32>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    wm_class: String,
    host: String,
    title: String,
}

impl WindowInfo {
    fn short_title(&self, max: usize) -> String {
        truncate(&self.title, max)
    }

    fn geometry(&self) -> String {
        format!("{}x{}+{}+{}", self.width, self.height, self.x, self.y)
    }
}

struct CaptureCandidate {
    slot: u8,
    window: WindowInfo,
    label: String,
    title_contains: String,
    notes: String,
}

struct WindowJumpApp {
    config: AppConfig,
    state: AppState,
    read_only: bool,
    status: String,
    ui_font_status: String,
    windows: Vec<WindowInfo>,
    active_window_id: Option<u64>,
    selected_slot: u8,
    selected_live_window_id: Option<u64>,
    last_refresh: Instant,
    capture_receiver: Option<Receiver<std::result::Result<WindowInfo, String>>>,
    pending_capture_slot: Option<u8>,
    capture_candidate: Option<CaptureCandidate>,
    confirm_reset_all: bool,
    executable_hint: String,
    draft_slot: u8,
    draft_rule: SlotRule,
    draft_dirty: bool,
    live_filter: String,
    show_window_catalog: bool,
}

impl WindowJumpApp {
    fn new(
        config: AppConfig,
        state: AppState,
        read_only: bool,
        startup_status: String,
        ui_font_status: String,
    ) -> Self {
        let draft_rule = config.slots.get(&SLOT_MIN).cloned().unwrap_or_default();
        let mut app = Self {
            config,
            state,
            read_only,
            status: startup_status,
            ui_font_status,
            windows: Vec::new(),
            active_window_id: None,
            selected_slot: SLOT_MIN,
            selected_live_window_id: None,
            last_refresh: Instant::now() - REFRESH_INTERVAL,
            capture_receiver: None,
            pending_capture_slot: None,
            capture_candidate: None,
            confirm_reset_all: false,
            executable_hint: shortcut_executable_hint(),
            draft_slot: SLOT_MIN,
            draft_rule,
            draft_dirty: false,
            live_filter: String::new(),
            show_window_catalog: false,
        };

        if let Err(err) = app.refresh_now() {
            app.status = format!("初回更新に失敗しました: {err}");
        }
        app
    }

    fn refresh_if_due(&mut self) {
        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            if let Err(err) = self.refresh_now() {
                self.status = format!("ウィンドウ一覧の更新に失敗しました: {err}");
            }
        }
    }

    fn refresh_now(&mut self) -> Result<()> {
        self.windows = list_windows()?;
        self.active_window_id = active_window_id().ok();
        self.last_refresh = Instant::now();

        if let Some(selected_id) = self.selected_live_window_id {
            if !self.windows.iter().any(|w| w.id == selected_id) {
                self.selected_live_window_id = None;
            }
        }

        Ok(())
    }

    fn select_slot(&mut self, slot: u8) {
        if self.draft_dirty {
            self.status =
                "未保存の編集があります。Save または Revert してから slot を切り替えてください"
                    .to_string();
            return;
        }

        self.selected_slot = slot;
        self.draft_slot = slot;
        self.draft_rule = self.slot_rule(slot);
        self.draft_dirty = false;
    }

    fn mark_read_only_status(&mut self) -> bool {
        if self.read_only {
            self.status =
                "read-only モードです。設定読み込みエラーを直すまで保存できません".to_string();
            true
        } else {
            false
        }
    }

    fn save_draft_rule(&mut self) {
        if self.mark_read_only_status() {
            return;
        }

        self.config
            .set_slot(self.selected_slot, self.draft_rule.clone());
        if self.draft_rule.is_effectively_empty() {
            self.state.slots.remove(&self.selected_slot);
        }
        self.draft_dirty = false;
        match save_store(&self.config, &self.state) {
            Ok((config_path, state_path)) => {
                self.status = format!(
                    "設定を保存しました: {} / {}",
                    config_path.display(),
                    state_path.display()
                );
            }
            Err(err) => {
                self.status = format!("設定の保存に失敗しました: {err}");
            }
        }
    }

    fn revert_draft_rule(&mut self) {
        self.draft_rule = self.slot_rule(self.selected_slot);
        self.draft_slot = self.selected_slot;
        self.draft_dirty = false;
        self.status = "未保存の編集を破棄しました".to_string();
    }

    fn capture_active_into_slot(&mut self, slot: u8) {
        match active_window() {
            Ok(window) => {
                self.prepare_capture_candidate(slot, window);
            }
            Err(err) => {
                self.status = format!("アクティブウィンドウの取得に失敗しました: {err}");
            }
        }
    }

    fn capture_selected_live_into_slot(&mut self, slot: u8) {
        let Some(window) = self
            .selected_live_window_id
            .and_then(|id| self.windows.iter().find(|w| w.id == id).cloned())
        else {
            self.status = "右側の live windows から対象を選択してください".to_string();
            return;
        };

        self.prepare_capture_candidate(slot, window);
    }

    fn prepare_capture_candidate(&mut self, slot: u8, window: WindowInfo) {
        let notes = self
            .config
            .slots
            .get(&slot)
            .map(|rule| rule.notes.clone())
            .unwrap_or_default();

        self.selected_slot = slot;
        self.capture_candidate = Some(CaptureCandidate {
            slot,
            label: default_label(&window),
            title_contains: suggest_title_contains(&window, &self.windows),
            notes,
            window,
        });
        self.status = format!(
            "slot {} の登録候補を認識しました。内容を確認して OK で登録してください",
            slot
        );
    }

    fn register_capture_candidate(&mut self) {
        if self.mark_read_only_status() {
            return;
        }

        let Some(candidate) = self.capture_candidate.take() else {
            self.status = "登録候補がありません".to_string();
            return;
        };

        let mut rule = self
            .config
            .slots
            .get(&candidate.slot)
            .cloned()
            .unwrap_or_default();
        rule.label = candidate.label;
        rule.wm_class = candidate.window.wm_class.clone();
        rule.title_contains = candidate.title_contains;
        rule.notes = candidate.notes;
        update_slot_state(&mut self.state, candidate.slot, &candidate.window);

        self.config.set_slot(candidate.slot, rule.clone());
        self.draft_slot = candidate.slot;
        self.draft_rule = self.slot_rule(candidate.slot);
        self.draft_dirty = false;
        match save_store(&self.config, &self.state) {
            Ok((config_path, state_path)) => {
                self.status = format!(
                    "slot {} を更新しました: {} [{}] {} (saved: {} / {})",
                    candidate.slot,
                    candidate.window.id_hex,
                    rule.wm_class,
                    candidate.window.title,
                    config_path.display(),
                    state_path.display()
                );
            }
            Err(err) => {
                self.status = format!(
                    "slot {} は更新しましたが設定保存に失敗しました: {err}",
                    candidate.slot
                );
            }
        }
    }

    fn cancel_capture_candidate(&mut self) {
        self.capture_candidate = None;
        self.status = "登録候補を破棄しました".to_string();
    }

    fn begin_click_capture(&mut self, slot: u8) {
        if self.capture_receiver.is_some() {
            self.status =
                "すでにクリック取得待ちです。まず現在の取得を完了してください".to_string();
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.capture_candidate = None;
        thread::spawn(move || {
            let result = (|| -> Result<WindowInfo> {
                let id = choose_window_by_click()?;
                window_by_id(id)
            })()
            .map_err(|err| err.to_string());

            let _ = tx.send(result);
        });

        self.capture_receiver = Some(rx);
        self.pending_capture_slot = Some(slot);
        self.status = format!(
            "slot {} 用の対象ウィンドウをクリックしてください。GUI 以外の目的ウィンドウをクリックします",
            slot
        );
    }

    fn poll_capture_result(&mut self) {
        let maybe_message = match self.capture_receiver.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Ok(message) => Some(message),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("クリック取得スレッドが終了しました".to_string()))
                }
            },
            None => None,
        };

        let Some(message) = maybe_message else {
            return;
        };

        self.capture_receiver = None;
        let slot = self
            .pending_capture_slot
            .take()
            .unwrap_or(self.selected_slot);

        match message {
            Ok(window) => self.prepare_capture_candidate(slot, window),
            Err(err) => {
                self.status = format!("クリック取得に失敗しました: {err}");
            }
        }
    }

    fn test_activate_selected_slot(&mut self) {
        let slot = self.selected_slot;
        let Some(rule) = self.config.slots.get(&slot).cloned() else {
            self.status = format!("slot {} は未登録です", slot);
            return;
        };

        let slot_state = self.state.slots.get(&slot);
        match resolve_slot(&rule, slot_state, &self.windows).cloned() {
            Ok(window) => match activate_window(&window) {
                Ok(()) => {
                    update_slot_state(&mut self.state, slot, &window);
                    match save_state(&self.state) {
                        Ok(path) => {
                            self.status = format!(
                                "slot {} をアクティブ化しました: {} [{}] {} (saved: {})",
                                slot,
                                window.id_hex,
                                window.wm_class,
                                window.title,
                                path.display()
                            );
                        }
                        Err(err) => {
                            self.status = format!(
                                "slot {} はアクティブ化しましたが設定保存に失敗しました: {err}",
                                slot
                            );
                        }
                    }
                    let _ = self.refresh_now();
                }
                Err(err) => {
                    self.status = format!("slot {} のアクティブ化に失敗しました: {err}", slot);
                }
            },
            Err(err) => {
                self.status = format!("slot {} の解決に失敗しました: {err}", slot);
            }
        }
    }

    fn clear_selected_slot(&mut self) {
        if self.mark_read_only_status() {
            return;
        }

        self.config.slots.remove(&self.selected_slot);
        self.state.slots.remove(&self.selected_slot);
        if self
            .capture_candidate
            .as_ref()
            .is_some_and(|candidate| candidate.slot == self.selected_slot)
        {
            self.capture_candidate = None;
        }
        self.draft_rule = SlotRule::default();
        self.draft_dirty = false;
        match save_store(&self.config, &self.state) {
            Ok((config_path, state_path)) => {
                self.status = format!(
                    "slot {} を削除しました (saved: {} / {})",
                    self.selected_slot,
                    config_path.display(),
                    state_path.display()
                );
            }
            Err(err) => {
                self.status = format!(
                    "slot {} の削除後に設定保存に失敗しました: {err}",
                    self.selected_slot
                );
            }
        }
    }

    fn reset_all_slots(&mut self) {
        if self.mark_read_only_status() {
            return;
        }

        self.config.slots.clear();
        self.state.slots.clear();
        self.capture_candidate = None;
        self.confirm_reset_all = false;
        self.draft_rule = SlotRule::default();
        self.draft_dirty = false;
        match save_store(&self.config, &self.state) {
            Ok((config_path, state_path)) => {
                self.status = format!(
                    "全 slot を削除しました (saved: {} / {})",
                    config_path.display(),
                    state_path.display()
                );
            }
            Err(err) => {
                self.status = format!("全 slot の削除後に設定保存に失敗しました: {err}");
            }
        }
    }

    fn selected_live_window(&self) -> Option<&WindowInfo> {
        self.selected_live_window_id
            .and_then(|id| self.windows.iter().find(|window| window.id == id))
    }

    fn active_window(&self) -> Option<&WindowInfo> {
        self.active_window_id
            .and_then(|id| self.windows.iter().find(|window| window.id == id))
    }

    fn slot_rule(&self, slot: u8) -> SlotRule {
        self.config.slots.get(&slot).cloned().unwrap_or_default()
    }

    fn slot_status(&self, slot: u8) -> String {
        let Some(rule) = self.config.slots.get(&slot) else {
            return "- Empty".to_string();
        };
        if !rule.is_usable() {
            return "- Empty".to_string();
        }

        match resolve_slot(rule, self.state.slots.get(&slot), &self.windows) {
            Ok(_) => "✓ Resolved".to_string(),
            Err(err) => {
                let text = err.to_string();
                if text.contains("複数候補") {
                    "! Ambiguous".to_string()
                } else {
                    "× Not found".to_string()
                }
            }
        }
    }

    fn set_window_catalog_visible(&mut self, ctx: &egui::Context, visible: bool) {
        if self.show_window_catalog == visible {
            return;
        }

        self.show_window_catalog = visible;
        let [width, height] = if visible {
            GUI_EXPANDED_INNER_SIZE
        } else {
            GUI_COMPACT_INNER_SIZE
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));

        if visible {
            match self.refresh_now() {
                Ok(()) => {
                    self.status = "Window catalog を開き、ウィンドウ一覧を更新しました".to_string()
                }
                Err(err) => {
                    self.status =
                        format!("Window catalog を開きましたが、更新に失敗しました: {err}");
                }
            }
        } else {
            self.status = "Window catalog を閉じました".to_string();
        }
    }

    fn render_top_bar(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::top("top_bar").show_inside(root_ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Window Jump");
                ui.separator();
                ui.label("X11 slot-based window selector");
                ui.separator();
                ui.label(format!(
                    "X11: {}",
                    env::var("XDG_SESSION_TYPE").unwrap_or_default()
                ));
                ui.label(if command_exists("wmctrl") {
                    "[wmctrl OK]"
                } else {
                    "[wmctrl missing]"
                });
                ui.label(if command_exists("xdotool") {
                    "[xdotool OK]"
                } else {
                    "[xdotool missing]"
                });
                if self.read_only {
                    ui.label(RichText::new("[read-only]").strong());
                }
                ui.separator();
                if ui.button("Refresh now").clicked() {
                    match self.refresh_now() {
                        Ok(()) => self.status = "ウィンドウ一覧を更新しました".to_string(),
                        Err(err) => {
                            self.status = format!("更新に失敗しました: {err}");
                        }
                    }
                }
                let catalog_label = if self.show_window_catalog {
                    "Window catalog を閉じる"
                } else {
                    "Window catalog を開く"
                };
                if ui.button(catalog_label).clicked() {
                    let ctx = ui.ctx().clone();
                    self.set_window_catalog_visible(&ctx, !self.show_window_catalog);
                }
                if ui.button("Print shortcut specs to stdout").clicked() {
                    for slot in SLOT_MIN..=SLOT_MAX {
                        println!(
                            "Ctrl+Alt+{} -> {} activate {}",
                            slot,
                            shell_quote(&self.executable_hint),
                            slot
                        );
                    }
                    self.status = "標準出力へ shortcut specs を出力しました".to_string();
                }
            });

            ui.separator();

            let active_summary = match self.active_window() {
                Some(window) => format!(
                    "ACTIVE: {} [{}] {}",
                    window.id_hex,
                    window.wm_class,
                    window.short_title(90)
                ),
                None => "ACTIVE: 不明".to_string(),
            };
            ui.label(active_summary);

            if let Some(window) = self.selected_live_window() {
                ui.label(format!(
                    "LIVE SELECTED: {} [{}] {}",
                    window.id_hex,
                    window.wm_class,
                    window.short_title(90)
                ));
            }

            ui.label(format!("FONT: {}", self.ui_font_status));
            ui.label(format!("STATUS: {}", self.status));
        });
    }

    fn render_slot_list(&mut self, root_ui: &mut egui::Ui) {
        egui::Panel::left("slot_list")
            .default_size(285.0)
            .resizable(true)
            .show_inside(root_ui, |ui| {
                ui.heading("Slots 1..9");
                ui.label("左で slot を選び、中央でルールを編集します。");
                ui.separator();

                ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for slot in SLOT_MIN..=SLOT_MAX {
                            let rule = self.slot_rule(slot);
                            let selected = self.selected_slot == slot;
                            let summary_title = if !rule.label.trim().is_empty() {
                                format!("slot {}  {}", slot, truncate(&rule.label, 26))
                            } else if !rule.wm_class.trim().is_empty() {
                                format!("slot {}  {}", slot, truncate(&rule.wm_class, 26))
                            } else {
                                format!("slot {}  <empty>", slot)
                            };

                            ui.group(|ui| {
                                if ui.selectable_label(selected, summary_title).clicked() {
                                    self.select_slot(slot);
                                }
                                ui.small(self.slot_status(slot));
                                ui.small(rule.summary());
                                if let Some(last_title) = self
                                    .state
                                    .slots
                                    .get(&slot)
                                    .and_then(|slot_state| slot_state.last_seen_title.as_ref())
                                {
                                    ui.small(format!("last: {}", truncate(last_title, 34)));
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
            });
    }

    fn render_selected_slot_panel(&mut self, ui: &mut egui::Ui) {
        let slot = self.selected_slot;
        ui.heading(format!("Slot {} editor", slot));
        ui.label("登録候補を認識し、内容を確認してから slot に保存します。タイトルは完全一致ではなく部分一致です。");
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            if ui.button("候補: 現在のウィンドウ").clicked() {
                self.capture_active_into_slot(slot);
            }
            if ui.button("登録モード: クリックで選択").clicked() {
                self.begin_click_capture(slot);
            }
            if self.show_window_catalog {
                if ui
                    .add_enabled(
                        self.selected_live_window_id.is_some(),
                        egui::Button::new("候補: catalog 選択"),
                    )
                    .clicked()
                {
                    self.capture_selected_live_into_slot(slot);
                }
            } else if ui.button("Window catalog を開く").clicked() {
                let ctx = ui.ctx().clone();
                self.set_window_catalog_visible(&ctx, true);
            }
            if ui.button("Test activate").clicked() {
                self.test_activate_selected_slot();
            }
        });

        ui.add_space(8.0);

        self.render_capture_candidate(ui, slot);

        let shortcut_command = format!("{} activate {}", shell_quote(&self.executable_hint), slot);
        ui.label("GNOME Custom Shortcut command");
        ui.monospace(shortcut_command);
        ui.small(format!("推奨 binding: Ctrl+Alt+{}", slot));

        ui.separator();

        if self.draft_slot != slot && !self.draft_dirty {
            self.draft_slot = slot;
            self.draft_rule = self.slot_rule(slot);
        }

        ui.label("Label");
        self.draft_dirty |= ui
            .text_edit_singleline(&mut self.draft_rule.label)
            .changed();

        ui.label("WM_CLASS (exact match)");
        self.draft_dirty |= ui
            .text_edit_singleline(&mut self.draft_rule.wm_class)
            .changed();

        ui.label("Title contains (case-insensitive substring)");
        self.draft_dirty |= ui
            .text_edit_singleline(&mut self.draft_rule.title_contains)
            .changed();
        if let Some(warning) = title_contains_warning(&self.draft_rule.title_contains) {
            ui.small(RichText::new(warning).color(egui::Color32::YELLOW));
        }

        ui.label("Notes");
        self.draft_dirty |= ui
            .add(
                TextEdit::multiline(&mut self.draft_rule.notes)
                    .desired_rows(3)
                    .hint_text("運用メモや識別の根拠を書けます"),
            )
            .changed();

        ui.horizontal_wrapped(|ui| {
            let save_label = if self.draft_dirty {
                "Save"
            } else {
                "Save (変更なし)"
            };
            if ui
                .add_enabled(
                    !self.read_only && self.draft_dirty,
                    egui::Button::new(save_label),
                )
                .clicked()
            {
                self.save_draft_rule();
            }
            if ui
                .add_enabled(self.draft_dirty, egui::Button::new("Revert"))
                .clicked()
            {
                self.revert_draft_rule();
            }
            if self.read_only {
                ui.small("read-only: 保存不可");
            } else if self.draft_dirty {
                ui.small("未保存の編集があります");
            }
        });

        ui.add_space(8.0);
        ui.label("Metadata");
        let slot_state = self.state.slots.get(&slot);
        ui.monospace(format!(
            "desktop_hint   : {}",
            slot_state
                .and_then(|state| state.desktop_hint)
                .map(|d| d.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        ui.monospace(format!(
            "last_window_id : {}",
            slot_state
                .and_then(|state| state.last_window_id.clone())
                .unwrap_or_else(|| "-".to_string())
        ));
        ui.monospace(format!(
            "last_seen_title: {}",
            slot_state
                .and_then(|state| state.last_seen_title.clone())
                .unwrap_or_else(|| "-".to_string())
        ));

        ui.separator();
        ui.label(RichText::new("Current resolution preview").strong());

        let preview_rule = self.slot_rule(slot);
        if !preview_rule.is_usable() {
            ui.label("まだ WM_CLASS が未設定です。登録モードでウィンドウを選択し、OK で登録してください。");
            return;
        }

        let matches: Vec<WindowInfo> = self
            .windows
            .iter()
            .filter(|window| rule_matches(&preview_rule, window))
            .cloned()
            .collect();

        if matches.is_empty() {
            ui.label("一致候補は 0 件です。");
        } else {
            ui.label(format!("一致候補: {} 件", matches.len()));
        }

        ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for window in matches {
                let active_marker = if self.active_window_id == Some(window.id) {
                    "ACTIVE "
                } else {
                    "       "
                };
                ui.monospace(format!(
                    "{}{}  D{}  {:<28}  {}",
                    active_marker,
                    window.id_hex,
                    window.desktop,
                    truncate(&window.wm_class, 28),
                    window.short_title(70)
                ));
                ui.small(format!("geom={} pid={:?}", window.geometry(), window.pid));
                ui.separator();
            }
        });

        ui.separator();
        ui.collapsing("Danger zone", |ui| {
            if ui
                .add_enabled(!self.read_only, egui::Button::new("Clear selected slot"))
                .clicked()
            {
                self.clear_selected_slot();
            }
            if ui
                .add_enabled(!self.read_only, egui::Button::new("Reset all slots"))
                .clicked()
            {
                self.confirm_reset_all = true;
                self.status =
                    "全 slot を削除するには Confirm reset all slots を押してください".to_string();
            }

            if self.confirm_reset_all {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("全 slot を削除します。").strong());
                    if ui.button("Confirm reset all slots").clicked() {
                        self.reset_all_slots();
                    }
                    if ui.button("Cancel reset").clicked() {
                        self.confirm_reset_all = false;
                        self.status = "全リセットをキャンセルしました".to_string();
                    }
                });
            }
        });
    }

    fn render_capture_candidate(&mut self, ui: &mut egui::Ui, slot: u8) {
        let Some(candidate_slot) = self
            .capture_candidate
            .as_ref()
            .map(|candidate| candidate.slot)
        else {
            return;
        };

        if candidate_slot != slot {
            ui.label(format!(
                "slot {} に未登録の候補があります。左の slot {} を選んで確認してください。",
                candidate_slot, candidate_slot
            ));
            ui.separator();
            return;
        }

        let mut register = false;
        let mut cancel = false;

        if let Some(candidate) = &mut self.capture_candidate {
            ui.group(|ui| {
                ui.label(RichText::new("Registration candidate").strong());
                ui.monospace(format!(
                    "window_id : {}\ndesktop   : {}\npid       : {}\nwm_class  : {}\ngeometry  : {}\ntitle     : {}",
                    candidate.window.id_hex,
                    candidate.window.desktop,
                    candidate
                        .window
                        .pid
                        .map(|pid| pid.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    candidate.window.wm_class,
                    candidate.window.geometry(),
                    candidate.window.title
                ));

                ui.add_space(4.0);
                ui.label("Label");
                ui.text_edit_singleline(&mut candidate.label);

                ui.label("Title contains");
                ui.text_edit_singleline(&mut candidate.title_contains);
                if let Some(warning) = title_contains_warning(&candidate.title_contains) {
                    ui.small(RichText::new(warning).color(egui::Color32::YELLOW));
                }

                ui.label("Notes");
                ui.add(TextEdit::multiline(&mut candidate.notes).desired_rows(2));

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.read_only,
                            egui::Button::new("OK: register to this slot"),
                        )
                        .clicked()
                    {
                        register = true;
                    }
                    if ui.button("Cancel candidate").clicked() {
                        cancel = true;
                    }
                });
            });
            ui.separator();
        }

        if register {
            self.register_capture_candidate();
        }
        if cancel {
            self.cancel_capture_candidate();
        }
    }

    fn render_live_windows_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Window catalog");
        ui.label("現在の X11 管理対象ウィンドウです。行を選択して slot に取り込めます。");
        ui.horizontal_wrapped(|ui| {
            ui.label("Filter");
            ui.text_edit_singleline(&mut self.live_filter);
        });
        ui.separator();

        let rule = self.slot_rule(self.selected_slot);
        let filter = normalize(&self.live_filter);
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .max_height(520.0)
            .show(ui, |ui| {
                ui.monospace("State     ID          Desk App                          Title");
                ui.separator();
                for window in &self.windows {
                    if !filter.is_empty()
                        && !normalize(&format!("{} {}", window.wm_class, window.title))
                            .contains(&filter)
                    {
                        continue;
                    }

                    let mut prefix = String::new();
                    if self.active_window_id == Some(window.id) {
                        prefix.push_str("ACTIVE ");
                    }
                    if rule_matches(&rule, window) {
                        prefix.push_str("MATCH ");
                    }
                    if prefix.is_empty() {
                        prefix.push('-');
                    }
                    let text = format!(
                        "{:<9} {:<11} D{:<3} {:<28} {}",
                        prefix,
                        window.id_hex,
                        window.desktop,
                        truncate(&window.wm_class, 28),
                        window.short_title(80)
                    );

                    if ui
                        .selectable_label(self.selected_live_window_id == Some(window.id), text)
                        .clicked()
                    {
                        self.selected_live_window_id = Some(window.id);
                    }
                }
            });

        ui.separator();
        ui.label(RichText::new("Selected window detail").strong());
        if let Some(window) = self.selected_live_window().cloned() {
            ui.monospace(format!(
                "id       : {}\ndesktop  : {}\npid      : {}\napp      : {}\nhost     : {}\ngeometry : {}\ntitle    : {}",
                window.id_hex,
                window.desktop,
                window
                    .pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                window.wm_class,
                window.host,
                window.geometry(),
                window.title
            ));
            if ui
                .add_enabled(
                    !self.read_only,
                    egui::Button::new(format!("Use for slot {}", self.selected_slot)),
                )
                .clicked()
            {
                self.capture_selected_live_into_slot(self.selected_slot);
            }
        } else {
            ui.label("未選択です。");
        }
    }

    fn handle_quit_shortcuts(&self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }

        let quit_requested = ctx.input(|input| {
            input.modifiers.is_none()
                && (input.key_pressed(egui::Key::Escape) || input.key_pressed(egui::Key::Q))
        });

        if quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for WindowJumpApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_quit_shortcuts(ui.ctx());
        self.refresh_if_due();
        self.poll_capture_result();
        self.render_top_bar(ui);
        self.render_slot_list(ui);

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if self.show_window_catalog {
                ui.columns(2, |columns| {
                    self.render_selected_slot_panel(&mut columns[0]);
                    self.render_live_windows_panel(&mut columns[1]);
                });
            } else {
                self.render_selected_slot_panel(ui);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Window catalog は閉じています。検出ウィンドウ一覧から選ぶ場合だけ開きます。");
                    if ui.button("Window catalog を開く").clicked() {
                        let ctx = ui.ctx().clone();
                        self.set_window_catalog_visible(&ctx, true);
                    }
                });
            }
        });

        ui.ctx().request_repaint_after(REPAINT_INTERVAL);
    }
}

fn apply_capture(
    config: &mut AppConfig,
    state: &mut AppState,
    slot: u8,
    window: &WindowInfo,
    windows: &[WindowInfo],
    label: Option<String>,
    title_contains: Option<String>,
) -> SlotRule {
    let mut rule = config.slots.get(&slot).cloned().unwrap_or_default();

    if let Some(label) = label {
        rule.label = label;
    } else if rule.label.trim().is_empty() {
        rule.label = default_label(window);
    }

    rule.wm_class = window.wm_class.clone();
    rule.title_contains = title_contains.unwrap_or_else(|| suggest_title_contains(window, windows));
    update_slot_state(state, slot, window);

    config.set_slot(slot, rule.clone());
    rule
}

fn update_slot_state(state: &mut AppState, slot: u8, window: &WindowInfo) {
    state.set_slot_state(
        slot,
        SlotState {
            desktop_hint: Some(window.desktop),
            last_window_id: Some(window.id_hex.clone()),
            last_seen_title: Some(window.title.clone()),
        },
    );
}

fn default_label(window: &WindowInfo) -> String {
    let source = if !window.title.trim().is_empty() {
        window.title.trim()
    } else {
        window.wm_class.trim()
    };
    truncate(source, 48)
}

fn suggest_title_contains(window: &WindowInfo, windows: &[WindowInfo]) -> String {
    let same_class_others: Vec<&WindowInfo> = windows
        .iter()
        .filter(|candidate| candidate.id != window.id && candidate.wm_class == window.wm_class)
        .collect();

    let candidates = title_key_candidates(&window.title);
    if candidates.is_empty() {
        return window.title.clone();
    }

    if same_class_others.is_empty() {
        return candidates[0].clone();
    }

    for candidate in &candidates {
        let needle = normalize(candidate);
        if same_class_others
            .iter()
            .all(|other| !normalize(&other.title).contains(&needle))
        {
            return candidate.clone();
        }
    }

    candidates
        .last()
        .cloned()
        .unwrap_or_else(|| window.title.clone())
}

fn title_key_candidates(title: &str) -> Vec<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    for part in trimmed
        .split(['|', '—', '-', '·', '•', ':', '/', '\\'])
        .map(str::trim)
        .filter(|part| part.chars().count() >= 3)
    {
        for candidate in title_part_variants(part) {
            push_unique_candidate(&mut candidates, &candidate);
        }
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    for len in 1..=words.len().min(6) {
        let candidate = words[..len].join(" ");
        if candidate.chars().count() >= 3 {
            push_unique_candidate(&mut candidates, &candidate);
        }
    }

    push_unique_candidate(&mut candidates, trimmed);
    candidates
}

fn title_part_variants(part: &str) -> Vec<String> {
    let mut variants = Vec::new();
    if let Some(stable_prefix) = strip_common_date_suffix(part) {
        variants.push(stable_prefix);
    }
    variants.push(part.trim().to_string());
    variants
}

fn strip_common_date_suffix(input: &str) -> Option<String> {
    let markers = [
        "_Jan", "_Feb", "_Mar", "_Apr", "_May", "_Jun", "_Jul", "_Aug", "_Sep", "_Oct", "_Nov",
        "_Dec", "-Jan", "-Feb", "-Mar", "-Apr", "-May", "-Jun", "-Jul", "-Aug", "-Sep", "-Oct",
        "-Nov", "-Dec",
    ];
    for marker in markers {
        if let Some(index) = input.find(marker) {
            let prefix = input[..index].trim_matches(['_', '-', ' ']).trim();
            if prefix.chars().count() >= 3 {
                return Some(prefix.to_string());
            }
        }
    }
    None
}

fn push_unique_candidate(candidates: &mut Vec<String>, candidate: &str) {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return;
    }
    if !candidates
        .iter()
        .any(|existing| normalize(existing) == normalize(candidate))
    {
        candidates.push(candidate.to_string());
    }
}

fn title_contains_warning(title_contains: &str) -> Option<String> {
    let len = title_contains.chars().count();
    if len >= 70 {
        Some("title_contains が長めです。変化しやすいフルタイトルではなく、短く安定した語を推奨します。".to_string())
    } else {
        None
    }
}

fn rule_matches(rule: &SlotRule, window: &WindowInfo) -> bool {
    if rule.wm_class.trim().is_empty() {
        return false;
    }
    if rule.wm_class != window.wm_class {
        return false;
    }
    if rule.title_contains.trim().is_empty() {
        return true;
    }

    let needle = normalize(&rule.title_contains);
    let haystack = normalize(&window.title);
    haystack.contains(&needle)
}

fn resolve_slot<'a>(
    rule: &SlotRule,
    state: Option<&SlotState>,
    windows: &'a [WindowInfo],
) -> Result<&'a WindowInfo> {
    if !rule.is_usable() {
        bail!("WM_CLASS が未設定です");
    }

    let candidates: Vec<&WindowInfo> = windows
        .iter()
        .filter(|window| rule_matches(rule, window))
        .collect();

    if candidates.is_empty() {
        bail!(
            "一致候補が見つかりません。class={} title_contains={}",
            empty_dash(&rule.wm_class),
            empty_dash(&rule.title_contains)
        );
    }

    if candidates.len() == 1 {
        return Ok(candidates[0]);
    }

    if let Some(last_window_id) = state.and_then(|state| state.last_window_id.as_ref()) {
        let same_id: Vec<&WindowInfo> = candidates
            .iter()
            .copied()
            .filter(|window| window.id_hex.eq_ignore_ascii_case(last_window_id))
            .collect();
        if same_id.len() == 1 {
            return Ok(same_id[0]);
        }
    }

    if let Some(desktop_hint) = state.and_then(|state| state.desktop_hint) {
        let same_desktop: Vec<&WindowInfo> = candidates
            .iter()
            .copied()
            .filter(|window| window.desktop == desktop_hint)
            .collect();
        if same_desktop.len() == 1 {
            return Ok(same_desktop[0]);
        }
    }

    let mut message =
        String::from("複数候補が存在します。title_contains をもう少し具体化してください:\n");
    for candidate in candidates {
        let _ = std::fmt::Write::write_fmt(
            &mut message,
            format_args!(
                "- {} D{} [{}] {}\n",
                candidate.id_hex, candidate.desktop, candidate.wm_class, candidate.title
            ),
        );
    }
    bail!(message)
}

fn store_dir() -> Result<PathBuf> {
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join(APP_DIR));
    }

    let home = env::var_os("HOME").ok_or_else(|| anyhow!("HOME が設定されていません"))?;
    Ok(PathBuf::from(home).join(".config").join(APP_DIR))
}

fn config_path() -> Result<PathBuf> {
    Ok(store_dir()?.join(CONFIG_FILE))
}

fn state_path() -> Result<PathBuf> {
    Ok(store_dir()?.join(STATE_FILE))
}

fn load_store() -> Result<(AppConfig, AppState)> {
    let (config, legacy_state) = load_config_with_legacy_state()?;
    let mut state = load_state()?;
    for (slot, slot_state) in legacy_state.slots {
        state.slots.entry(slot).or_insert(slot_state);
    }
    Ok((config, state))
}

fn load_config_with_legacy_state() -> Result<(AppConfig, AppState)> {
    let path = config_path()?;
    if !path.exists() {
        return Ok((AppConfig::default(), AppState::default()));
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("設定ファイルを読めませんでした: {}", path.display()))?;
    let mut config: AppConfig = serde_json::from_str(&raw)
        .with_context(|| format!("JSON の解析に失敗しました: {}", path.display()))?;
    if config.version == 0 {
        config.version = 1;
    }

    let legacy_state = legacy_state_from_config_json(&raw).unwrap_or_default();
    Ok((config, legacy_state))
}

fn legacy_state_from_config_json(raw: &str) -> Result<AppState> {
    let legacy: LegacyConfig = serde_json::from_str(raw)?;
    let mut state = AppState::default();
    for (slot, legacy_rule) in legacy.slots {
        let slot_state = SlotState {
            desktop_hint: legacy_rule.desktop_hint,
            last_window_id: legacy_rule.last_window_id,
            last_seen_title: legacy_rule.last_seen_title,
        };
        state.set_slot_state(slot, slot_state);
    }
    Ok(state)
}

fn load_state() -> Result<AppState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(AppState::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("状態ファイルを読めませんでした: {}", path.display()))?;
    let mut state: AppState = serde_json::from_str(&raw)
        .with_context(|| format!("JSON の解析に失敗しました: {}", path.display()))?;
    if state.version == 0 {
        state.version = 1;
    }
    Ok(state)
}

fn save_state(state: &AppState) -> Result<PathBuf> {
    let path = state_path()?;
    let dir = store_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("設定ディレクトリを作成できませんでした: {}", dir.display()))?;
    let _lock = StoreLock::acquire(&dir)?;

    let json = serde_json::to_string_pretty(state).context("状態 JSON の生成に失敗しました")?;
    write_atomic(&path, json.as_bytes())?;
    Ok(path)
}

fn save_store(config: &AppConfig, state: &AppState) -> Result<(PathBuf, PathBuf)> {
    let dir = store_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("設定ディレクトリを作成できませんでした: {}", dir.display()))?;
    let _lock = StoreLock::acquire(&dir)?;

    let config_path = config_path()?;
    let state_path = state_path()?;
    let config_json =
        serde_json::to_string_pretty(config).context("設定 JSON の生成に失敗しました")?;
    let state_json =
        serde_json::to_string_pretty(state).context("状態 JSON の生成に失敗しました")?;
    write_atomic(&config_path, config_json.as_bytes())?;
    write_atomic(&state_path, state_json.as_bytes())?;
    Ok((config_path, state_path))
}

struct StoreLock {
    path: PathBuf,
}

impl StoreLock {
    fn acquire(dir: &Path) -> Result<Self> {
        let path = dir.join(LOCK_FILE);
        if fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|elapsed| elapsed > Duration::from_secs(30))
        {
            let _ = fs::remove_file(&path);
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "保存ロックを取得できませんでした: {}。別の window-jump が保存中の可能性があります",
                    path.display()
                )
            })?;
        writeln!(file, "pid={}", std::process::id()).ok();
        file.sync_all().ok();
        Ok(Self { path })
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("保存先ディレクトリを特定できませんでした"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("保存先ファイル名を特定できませんでした: {}", path.display()))?;
    let tmp_path = dir.join(format!(".{}.{}.tmp", file_name, std::process::id()));

    {
        let mut file = File::create(&tmp_path).with_context(|| {
            format!("一時ファイルを作成できませんでした: {}", tmp_path.display())
        })?;
        file.write_all(bytes).with_context(|| {
            format!("一時ファイルへ書き込めませんでした: {}", tmp_path.display())
        })?;
        file.write_all(b"\n").with_context(|| {
            format!("一時ファイルへ書き込めませんでした: {}", tmp_path.display())
        })?;
        file.sync_all().with_context(|| {
            format!("一時ファイルを同期できませんでした: {}", tmp_path.display())
        })?;
    }

    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "一時ファイルを保存先へ置換できませんでした: {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    if let Ok(dir_file) = File::open(dir) {
        let _ = dir_file.sync_all();
    }

    Ok(())
}

fn list_windows() -> Result<Vec<WindowInfo>> {
    let output = run_command_capture("wmctrl", ["-lpxG"])?;
    let mut windows = Vec::new();

    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let (fields, title) = split_prefix_fields(line, 9)
            .ok_or_else(|| anyhow!("wmctrl の出力行を解釈できませんでした: {line}"))?;

        let id = parse_window_id(fields[0])?;
        let desktop = fields[1]
            .parse::<i32>()
            .with_context(|| format!("desktop を解釈できませんでした: {}", fields[1]))?;
        let pid = fields[2].parse::<u32>().ok();
        let x = fields[3]
            .parse::<i32>()
            .with_context(|| format!("x を解釈できませんでした: {}", fields[3]))?;
        let y = fields[4]
            .parse::<i32>()
            .with_context(|| format!("y を解釈できませんでした: {}", fields[4]))?;
        let width = fields[5]
            .parse::<i32>()
            .with_context(|| format!("width を解釈できませんでした: {}", fields[5]))?;
        let height = fields[6]
            .parse::<i32>()
            .with_context(|| format!("height を解釈できませんでした: {}", fields[6]))?;

        windows.push(WindowInfo {
            id,
            id_hex: format_window_id_hex(id),
            desktop,
            pid,
            x,
            y,
            width,
            height,
            wm_class: fields[7].to_string(),
            host: fields[8].to_string(),
            title: title.to_string(),
        });
    }

    windows.sort_by(|left, right| {
        let left_key = (
            left.desktop,
            left.wm_class.to_lowercase(),
            left.title.to_lowercase(),
            left.id,
        );
        let right_key = (
            right.desktop,
            right.wm_class.to_lowercase(),
            right.title.to_lowercase(),
            right.id,
        );
        left_key.cmp(&right_key)
    });

    Ok(windows)
}

fn active_window_id() -> Result<u64> {
    let direct = run_command_capture("xdotool", ["getactivewindow"]);
    if let Ok(output) = direct {
        return parse_window_id(output.trim());
    }

    let focus = run_command_capture("xdotool", ["getwindowfocus"])?;
    parse_window_id(focus.trim())
}

fn active_window() -> Result<WindowInfo> {
    if let Ok(id) = active_window_id() {
        if let Ok(window) = window_by_id(id) {
            return Ok(window);
        }
    }

    let focus = run_command_capture("xdotool", ["getwindowfocus"])?;
    let id = parse_window_id(focus.trim())?;
    window_by_id(id)
}

fn choose_window_by_click() -> Result<u64> {
    let output = run_command_capture("xdotool", ["selectwindow"])?;
    parse_window_id(output.trim())
}

fn window_by_id(id: u64) -> Result<WindowInfo> {
    let windows = list_windows()?;
    windows
        .into_iter()
        .find(|window| window.id == id)
        .ok_or_else(|| {
            anyhow!(
                "window id {} は wmctrl 一覧にありません",
                format_window_id_hex(id)
            )
        })
}

fn activate_window(window: &WindowInfo) -> Result<()> {
    let dec = window.id.to_string();
    let hex = window.id_hex.clone();

    if let Err(first_error) =
        run_command_status("xdotool", ["windowactivate", "--sync", dec.as_str()])
    {
        run_command_status("wmctrl", ["-i", "-a", hex.as_str()]).with_context(|| {
            format!("xdotool と wmctrl の両方でアクティブ化に失敗しました。xdotool={first_error}")
        })?;
    }

    let _ = run_command_status("xdotool", ["windowraise", dec.as_str()]);
    Ok(())
}

fn run_command_capture<I, S>(program: &str, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("コマンド起動に失敗しました: {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("{} が失敗しました: {}", program, empty_dash(&stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_command_status<I, S>(program: &str, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("コマンド起動に失敗しました: {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        bail!(
            "{} が exit status {} で失敗しました。stderr={} stdout={}",
            program,
            output.status,
            empty_dash(&stderr),
            empty_dash(&stdout)
        );
    }
    Ok(())
}

fn parse_window_id(raw: &str) -> Result<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("window id が空です");
    }

    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16)
            .with_context(|| format!("16 進 window id を解釈できませんでした: {trimmed}"));
    }

    trimmed
        .parse::<u64>()
        .with_context(|| format!("10 進 window id を解釈できませんでした: {trimmed}"))
}

fn format_window_id_hex(id: u64) -> String {
    format!("0x{id:x}")
}

fn split_prefix_fields(line: &str, count: usize) -> Option<(Vec<&str>, &str)> {
    let bytes = line.as_bytes();
    let mut cursor = 0usize;
    let mut fields = Vec::with_capacity(count);

    while fields.len() < count {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return None;
        }

        let start = cursor;
        while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        fields.push(&line[start..cursor]);
    }

    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    let rest = if cursor < bytes.len() {
        &line[cursor..]
    } else {
        ""
    };
    Some((fields, rest))
}

fn truncate(input: &str, max: usize) -> String {
    let count = input.chars().count();
    if count <= max {
        return input.to_string();
    }
    let shortened: String = input.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", shortened)
}

fn normalize(input: &str) -> String {
    input.trim().to_lowercase()
}

fn empty_dash(input: &str) -> String {
    if input.trim().is_empty() {
        "-".to_string()
    } else {
        input.to_string()
    }
}

fn shortcut_executable_hint() -> String {
    if let Some(path) = env::var_os("WINDOW_JUMP_CLI") {
        return PathBuf::from(path).display().to_string();
    }

    if let Ok(current) = env::current_exe() {
        if current
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "window-jump-config-gui")
        {
            let sibling = current.with_file_name("window-jump");
            if sibling.exists() {
                return sibling.display().to_string();
            }
        } else {
            return current.display().to_string();
        }
    }

    find_executable_in_path("window-jump")
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "/absolute/path/to/window-jump".to_string())
}

fn command_exists(program: &str) -> bool {
    find_executable_in_path(program).is_some()
}

fn find_executable_in_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn shell_quote(input: &str) -> String {
    if input.is_empty() {
        return "''".to_string();
    }
    if input
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':'))
    {
        return input.to_string();
    }

    let escaped = input.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_window(id: u64, desktop: i32, wm_class: &str, title: &str) -> WindowInfo {
        WindowInfo {
            id,
            id_hex: format_window_id_hex(id),
            desktop,
            pid: Some(42),
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            wm_class: wm_class.to_string(),
            host: "host".to_string(),
            title: title.to_string(),
        }
    }

    #[test]
    fn parse_hex_id() {
        assert_eq!(parse_window_id("0x1a").unwrap(), 0x1a);
    }

    #[test]
    fn parse_decimal_id() {
        assert_eq!(parse_window_id("12345").unwrap(), 12345);
    }

    #[test]
    fn split_fields_and_title() {
        let line =
            "0x04600007  0  12345  100  200  800  600  Navigator.Firefox  host  Mozilla Firefox";
        let (fields, title) = split_prefix_fields(line, 9).unwrap();
        assert_eq!(fields[0], "0x04600007");
        assert_eq!(fields[7], "Navigator.Firefox");
        assert_eq!(fields[8], "host");
        assert_eq!(title, "Mozilla Firefox");
    }

    #[test]
    fn shell_quote_simple() {
        assert_eq!(shell_quote("/tmp/window-jump"), "/tmp/window-jump");
    }

    #[test]
    fn shell_quote_with_space() {
        assert_eq!(shell_quote("/tmp/window jump"), "'/tmp/window jump'");
    }

    #[test]
    fn rule_match_is_case_insensitive_for_title() {
        let rule = SlotRule {
            wm_class: "Navigator.Firefox".to_string(),
            title_contains: "inbox".to_string(),
            ..Default::default()
        };
        let window = test_window(1, 0, "Navigator.Firefox", "Gmail - Inbox - Mozilla Firefox");
        assert!(rule_matches(&rule, &window));
    }

    #[test]
    fn resolve_slot_uses_last_window_id_as_tie_breaker() {
        let rule = SlotRule {
            wm_class: "Terminal.Terminal".to_string(),
            title_contains: "server".to_string(),
            ..Default::default()
        };
        let state = SlotState {
            last_window_id: Some("0x2".to_string()),
            ..Default::default()
        };
        let windows = vec![
            test_window(1, 0, "Terminal.Terminal", "api-server"),
            test_window(2, 1, "Terminal.Terminal", "api-server"),
        ];

        let resolved = resolve_slot(&rule, Some(&state), &windows).unwrap();
        assert_eq!(resolved.id, 2);
    }

    #[test]
    fn resolve_slot_uses_desktop_hint_when_id_does_not_match() {
        let rule = SlotRule {
            wm_class: "Terminal.Terminal".to_string(),
            title_contains: "server".to_string(),
            ..Default::default()
        };
        let state = SlotState {
            desktop_hint: Some(1),
            last_window_id: Some("0xdeadbeef".to_string()),
            ..Default::default()
        };
        let windows = vec![
            test_window(1, 0, "Terminal.Terminal", "api-server"),
            test_window(2, 1, "Terminal.Terminal", "api-server"),
        ];

        let resolved = resolve_slot(&rule, Some(&state), &windows).unwrap();
        assert_eq!(resolved.id, 2);
    }

    #[test]
    fn resolve_slot_reports_ambiguity_without_unique_state_hint() {
        let rule = SlotRule {
            wm_class: "Terminal.Terminal".to_string(),
            title_contains: "server".to_string(),
            ..Default::default()
        };
        let windows = vec![
            test_window(1, 0, "Terminal.Terminal", "api-server"),
            test_window(2, 1, "Terminal.Terminal", "api-server"),
        ];

        let error = resolve_slot(&rule, None, &windows).unwrap_err().to_string();
        assert!(error.contains("複数候補"));
    }

    #[test]
    fn suggested_title_prefers_left_stable_segment() {
        let target = test_window(
            1,
            0,
            "google-chrome.Google-chrome",
            "stem_split_postprocess_theory_spec_mapping_Apr16-2026.md - Markdown Preview Bridge - Google Chrome",
        );
        let other = test_window(
            2,
            0,
            "google-chrome.Google-chrome",
            "inbox - Gmail - Google Chrome",
        );

        assert_eq!(
            suggest_title_contains(&target, &[target.clone(), other]),
            "stem_split_postprocess_theory_spec_mapping"
        );
    }

    #[test]
    fn legacy_config_state_is_migrated_to_state() {
        let raw = r#"{
          "version": 1,
          "slots": {
            "1": {
              "label": "Mail",
              "wm_class": "Navigator.Firefox",
              "title_contains": "Inbox",
              "desktop_hint": 2,
              "last_window_id": "0xabc",
              "last_seen_title": "Inbox - Firefox"
            }
          }
        }"#;

        let state = legacy_state_from_config_json(raw).unwrap();
        let slot_state = state.slots.get(&1).unwrap();
        assert_eq!(slot_state.desktop_hint, Some(2));
        assert_eq!(slot_state.last_window_id.as_deref(), Some("0xabc"));
        assert_eq!(
            slot_state.last_seen_title.as_deref(),
            Some("Inbox - Firefox")
        );
    }
}
