// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod artwork;
mod hub;
mod lyrics;
mod media;
mod power;
mod safety;
mod settings;
mod ui;

use hub::NowPlaying;
use lyrics::{LyricsEvent, LyricsService};
use media::{MediaCommand, NowPlayingSource};
use serde::Serialize;
use settings::Settings;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

const TOGGLE_SHORTCUT: &str = "CommandOrControl+Shift+L";
const AUTOSTART_FLAG: &str = "--autostart";

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub settings_path: PathBuf,
    pub now_playing: Mutex<Option<NowPlaying>>,
    pub lyrics: Mutex<Option<LyricsEvent>>,
    pub detection_paused: AtomicBool,
    pub source: Box<dyn NowPlayingSource>,
    pub lyrics_service: Arc<LyricsService>,
    pub power: power::KeepAwake,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    now_playing: Option<NowPlaying>,
    lyrics: Option<LyricsEvent>,
    settings: Settings,
    detection_paused: bool,
}

// ------------------------------------------------------------------ commands

#[tauri::command]
fn get_state(state: State<'_, AppState>) -> AppSnapshot {
    // One lock at a time: this runs on the main thread and must never wait on
    // a second lock while holding the first.
    let now_playing = state.now_playing.lock().unwrap().clone();
    let lyrics = state.lyrics.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    AppSnapshot { now_playing, lyrics, settings, detection_paused: state.detection_paused.load(Ordering::Relaxed) }
}

/// The lyrics page calls this as soon as its script runs (see `safety.rs`).
#[tauri::command]
fn frontend_ready(window: tauri::WebviewWindow) {
    if window.label() == ui::MAIN {
        safety::mark_frontend_ready();
    }
}

#[tauri::command]
fn media_command(state: State<'_, AppState>, cmd: MediaCommand) {
    state.source.command(cmd);
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
async fn update_settings(app: AppHandle, state: State<'_, AppState>, patch: serde_json::Value) -> Result<Settings, String> {
    let (old, new) = {
        let mut s = state.settings.lock().unwrap();
        let old = s.clone();
        let new = old.merged(&patch)?;
        *s = new.clone();
        (old, new)
    };
    if old == new {
        return Ok(new);
    }
    new.save(&state.settings_path);

    if old.launch_at_login != new.launch_at_login {
        let al = app.autolaunch();
        let res = if new.launch_at_login { al.enable() } else { al.disable() };
        if let Err(e) = res {
            log::warn!("autostart change failed: {e}");
        }
    }
    if (old.monitor != new.monitor || old.mirror_all != new.mirror_all) && ui::lyrics_visible(&app) {
        ui::show_lyrics(&app);
    }
    let _ = app.emit("settings", &new);
    Ok(new)
}

#[tauri::command]
async fn list_monitors(app: AppHandle) -> Vec<ui::MonitorInfo> {
    ui::list_monitors(&app)
}

// Filesystem work: async so it runs off the main (UI) thread.
#[tauri::command]
async fn clear_lyrics_cache(state: State<'_, AppState>) -> Result<usize, String> {
    Ok(state.lyrics_service.clear_cache())
}

#[tauri::command]
async fn lyrics_cache_count(state: State<'_, AppState>) -> Result<usize, String> {
    Ok(state.lyrics_service.cache_count())
}

#[tauri::command]
fn lyrics_override_dir(state: State<'_, AppState>) -> String {
    state
        .lyrics_service
        .override_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "Music/Lyrics".into())
}

#[tauri::command]
async fn close_lyrics(app: AppHandle) {
    ui::hide_lyrics(&app);
}

#[tauri::command]
async fn open_settings(app: AppHandle) {
    ui::open_settings(&app);
}

// ------------------------------------------------------------------ setup

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open lyrics mode", true, Some(TOGGLE_SHORTCUT))?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause detection", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Halo Lyrics", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let pause_item = pause.clone();
    let mut tray = TrayIconBuilder::with_id("halo")
        .tooltip("Halo Lyrics")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => ui::spawn(app, ui::show_lyrics),
            "settings" => ui::spawn(app, ui::open_settings),
            "pause" => {
                let paused = pause_item.is_checked().unwrap_or(false);
                app.state::<AppState>().detection_paused.store(paused, Ordering::Relaxed);
                log::info!("detection paused: {paused}");
            }
            "quit" => {
                app.state::<AppState>().power.set(false);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                ui::spawn(tray.app_handle(), ui::toggle_lyrics);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    let launched_at_login = std::env::args().any(|a| a == AUTOSTART_FLAG);

    let log_plugin = tauri_plugin_log::Builder::new()
        .clear_targets()
        .target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
            file_name: Some("halo-lyrics".into()),
        }))
        .max_file_size(1_000_000)
        .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
        .level(log::LevelFilter::Info)
        .level_for("tao", log::LevelFilter::Warn)
        .level_for("wry", log::LevelFilter::Warn)
        .level_for("reqwest", log::LevelFilter::Warn)
        .level_for("hyper_util", log::LevelFilter::Warn);
    #[cfg(debug_assertions)]
    let log_plugin = log_plugin.target(tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout));

    tauri::Builder::default()
        .plugin(log_plugin.build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_FLAG]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        ui::spawn(app, ui::toggle_lyrics);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_state,
            media_command,
            get_settings,
            update_settings,
            list_monitors,
            clear_lyrics_cache,
            lyrics_cache_count,
            lyrics_override_dir,
            close_lyrics,
            open_settings,
            frontend_ready,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Lyrics windows hide instead of closing; the app lives in the tray.
                if ui::is_lyrics_window(window.label()) {
                    api.prevent_close();
                    ui::spawn(window.app_handle(), ui::hide_lyrics);
                }
            }
        })
        .setup(move |app| {
            let paths = app.path();
            let config_dir = paths.app_config_dir()?;
            let data_dir = paths.app_data_dir()?;
            let settings_path = config_dir.join("settings.json");
            let settings = Settings::load(&settings_path);
            let override_dir = paths.audio_dir().ok().map(|d| d.join("Lyrics"));
            let lyrics_service = Arc::new(LyricsService::new(data_dir.join("lyrics-cache"), override_dir));
            let source = media::platform_source(paths.resource_dir().ok());
            log::info!("Halo Lyrics {} starting", app.package_info().version);

            // Keep the OS autostart entry in step with the setting.
            let al = app.autolaunch();
            if al.is_enabled().unwrap_or(false) != settings.launch_at_login {
                let _ = if settings.launch_at_login { al.enable() } else { al.disable() };
            }

            app.manage(AppState {
                settings: Mutex::new(settings),
                settings_path,
                now_playing: Mutex::new(None),
                lyrics: Mutex::new(None),
                detection_paused: AtomicBool::new(false),
                source,
                lyrics_service,
                power: power::KeepAwake::new(),
            });

            // Arm the Esc kill switch before any full-screen window can appear.
            safety::start(app.handle());
            setup_tray(app)?;
            if let Err(e) = app.global_shortcut().register(TOGGLE_SHORTCUT) {
                log::warn!("could not register {TOGGLE_SHORTCUT}: {e}");
            }

            let (tx, rx) = std::sync::mpsc::channel();
            app.state::<AppState>().source.start(tx);
            hub::spawn(app.handle().clone(), rx);

            // Full screen on launch (but stay in the tray when started at login).
            if !launched_at_login {
                ui::spawn(app.handle(), ui::show_lyrics);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Halo Lyrics")
        .run(|app, event| match event {
            // Closing every window must not quit: we live in the tray.
            RunEvent::ExitRequested { api, code, .. } if code.is_none() => api.prevent_exit(),
            RunEvent::Exit => app.state::<AppState>().power.set(false),
            _ => {}
        });
}
