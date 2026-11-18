//! Windows: the full-screen lyrics window (+ optional mirrors) and settings.
//!
//! These functions block on the event loop, so never call them *on* the main
//! thread (tray/shortcut handlers use [`spawn`]); creating a WebView2 window
//! from the main thread while it waits would deadlock on Windows.

use crate::{safety, AppState};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::window::Color;
use tauri::{
    AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, Position, Size, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

pub const MAIN: &str = "main";
pub const SETTINGS: &str = "settings";
const MIRROR_PREFIX: &str = "mirror-";
const BG: Color = Color(9, 9, 12, 255);

/// Serialises show/hide so rapid shortcut/tray presses can't interleave.
static UI_LOCK: Mutex<()> = Mutex::new(());
/// The lyrics window has a size/position the user may have changed in window
/// mode; keep it when the window is shown again. Cleared by full screen.
static WINDOW_PLACED: AtomicBool = AtomicBool::new(false);

fn ui_lock() -> MutexGuard<'static, ()> {
    UI_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub id: String,
    pub label: String,
    pub primary: bool,
}

/// Run `f` off the main thread.
pub fn spawn(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    std::thread::spawn(move || f(&app));
}

pub fn is_lyrics_window(label: &str) -> bool {
    label == MAIN || label.starts_with(MIRROR_PREFIX)
}

pub fn lyrics_visible(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

fn monitor_id(m: &Monitor, index: usize) -> String {
    m.name()
        .cloned()
        .unwrap_or_else(|| format!("monitor-{index}"))
}

pub fn list_monitors(app: &AppHandle) -> Vec<MonitorInfo> {
    let Some(w) = app.get_webview_window(MAIN) else {
        return Vec::new();
    };
    let monitors = w.available_monitors().unwrap_or_default();
    let primary = w.primary_monitor().ok().flatten();
    monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let size = m.size();
            MonitorInfo {
                id: monitor_id(m, i),
                label: format!("Display {} — {} × {}", i + 1, size.width, size.height),
                primary: primary
                    .as_ref()
                    .is_some_and(|p| p.name() == m.name() && p.position() == m.position()),
            }
        })
        .collect()
}

fn place(w: &WebviewWindow, m: &Monitor) {
    WINDOW_PLACED.store(false, Ordering::SeqCst);
    let _ = w.set_decorations(false);
    let pos = *m.position();
    let size = *m.size();
    // Leave full screen first so the window can move to another display.
    if w.is_fullscreen().unwrap_or(false) {
        if let Ok(Some(cur)) = w.current_monitor() {
            if cur.position() == m.position() && cur.size() == m.size() {
                let _ = w.set_always_on_top(true);
                return;
            }
        }
        let _ = w.set_fullscreen(false);
    }
    let _ = w.set_position(Position::Physical(PhysicalPosition::new(pos.x, pos.y)));
    let _ = w.set_size(Size::Physical(PhysicalSize::new(size.width, size.height)));
    let _ = w.set_fullscreen(true);
    let _ = w.set_always_on_top(true);
}

/// Window mode: an ordinary window with a title bar (minimise, maximise, close),
/// in the taskbar and not on top. The first time it gets a comfortable size,
/// centred on the chosen display; after that it keeps where the user put it.
fn place_windowed(w: &WebviewWindow, m: Option<&Monitor>) {
    let _ = w.set_always_on_top(false);
    let _ = w.set_fullscreen(false);
    let _ = w.set_decorations(true);
    let _ = w.set_resizable(true);
    let _ = w.set_skip_taskbar(false);
    if WINDOW_PLACED.swap(true, Ordering::SeqCst) {
        return;
    }
    match m {
        Some(m) => {
            let area = *m.size();
            let pos = *m.position();
            let width = (area.width as f64 * 0.7).max(960.0).min(area.width as f64) as u32;
            let height = (area.height as f64 * 0.72).max(600.0).min(area.height as f64) as u32;
            let _ = w.set_size(Size::Physical(PhysicalSize::new(width, height)));
            let _ = w.set_position(Position::Physical(PhysicalPosition::new(
                pos.x + (area.width as i32 - width as i32) / 2,
                pos.y + (area.height as i32 - height as i32) / 2,
            )));
        }
        None => {
            let _ = w.center();
        }
    }
}

pub fn show_lyrics(app: &AppHandle) {
    let Some(main) = app.get_webview_window(MAIN) else {
        return;
    };
    let _guard = ui_lock();
    let state = app.state::<AppState>();
    let settings = state
        .settings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let windowed = settings.windowed();
    // Arm the Esc kill switch before anything can cover the screen. A normal
    // window can't trap anyone, and there Esc belongs to whatever app has focus.
    safety::set_overlay_active(!windowed);
    let monitors = main.available_monitors().unwrap_or_default();
    let primary = main.primary_monitor().ok().flatten();

    let target_idx = settings
        .monitor
        .as_ref()
        .and_then(|id| {
            monitors
                .iter()
                .enumerate()
                .position(|(i, m)| &monitor_id(m, i) == id)
        })
        .or_else(|| {
            primary.as_ref().and_then(|p| {
                monitors
                    .iter()
                    .position(|m| m.name() == p.name() && m.position() == p.position())
            })
        })
        .or(if monitors.is_empty() { None } else { Some(0) });

    let target = target_idx.and_then(|i| monitors.get(i));
    if windowed {
        place_windowed(&main, target);
    } else {
        let _ = main.set_skip_taskbar(true);
        match target {
            Some(m) => place(&main, m),
            None => {
                let _ = main.set_decorations(false);
                let _ = main.set_fullscreen(true);
            }
        }
    }
    let _ = main.unminimize();
    let _ = main.show();
    let _ = main.set_focus();
    log::info!("lyrics shown ({})", if windowed { "window" } else { "full screen" });
    safety::watch_frontend_load(app);

    // Mirrors on every other display (full screen only).
    let mut used = 0;
    if settings.mirror_all && !windowed {
        for (i, m) in monitors.iter().enumerate() {
            if Some(i) == target_idx {
                continue;
            }
            let label = format!("{MIRROR_PREFIX}{used}");
            used += 1;
            let w = match app.get_webview_window(&label) {
                Some(w) => w,
                None => match WebviewWindowBuilder::new(
                    app,
                    &label,
                    WebviewUrl::App("index.html".into()),
                )
                .title("Halo Lyrics")
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                .focused(false)
                .background_color(BG)
                .build()
                {
                    Ok(w) => w,
                    Err(e) => {
                        log::warn!("creating mirror window failed: {e}");
                        continue;
                    }
                },
            };
            place(&w, m);
            let _ = w.show();
        }
    }
    for (label, w) in app.webview_windows() {
        if let Some(n) = label
            .strip_prefix(MIRROR_PREFIX)
            .and_then(|n| n.parse::<usize>().ok())
        {
            if n >= used {
                let _ = w.destroy();
            }
        }
    }
    // Focus back on the main window after creating mirrors.
    let _ = main.set_focus();
}

pub fn hide_lyrics(app: &AppHandle) {
    let _guard = ui_lock();
    for (label, w) in app.webview_windows() {
        if is_lyrics_window(&label) {
            let _ = w.set_always_on_top(false);
            let _ = w.set_fullscreen(false);
            let _ = w.hide();
        }
    }
    app.state::<AppState>().power.set(false);
    // Only now is the screen really free again.
    safety::set_overlay_active(false);
    log::info!("lyrics overlay hidden");
}

pub fn toggle_lyrics(app: &AppHandle) {
    if lyrics_visible(app) {
        hide_lyrics(app);
    } else {
        show_lyrics(app);
    }
}

pub fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        log::info!("settings: focusing the existing window");
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    log::info!("settings: creating the window");
    let built = WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("settings.html".into()))
        .title("Halo Lyrics Settings")
        .inner_size(560.0, 780.0)
        .min_inner_size(440.0, 480.0)
        .resizable(true)
        .center()
        // Must sit above the always-on-top lyrics window.
        .always_on_top(true)
        .theme(Some(tauri::Theme::Dark))
        .background_color(BG)
        .build();
    match built {
        Ok(w) => {
            log::info!("settings: window created");
            let _ = w.set_focus();
        }
        Err(e) => log::warn!("opening settings failed: {e}"),
    }
}
