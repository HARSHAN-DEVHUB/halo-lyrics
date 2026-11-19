//! Automatic updates.
//!
//! Releases are built by `.github/workflows/release.yml` and published to the
//! public `glowline-releases` repo together with a signed `latest.json`. The
//! app checks that file shortly after start and then every few hours; a newer
//! version is downloaded in the background and installed when the user
//! chooses "Restart to update" (lyrics window, Settings or the tray menu).

use crate::{ui, AppState};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::MenuItem;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST_CHECK_AFTER: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
pub const TRAY_ITEM: &str = "update";
const TRAY_CHECK_TEXT: &str = "Check for updates…";

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateStatus {
    Idle,
    /// Development builds never update themselves.
    Disabled,
    Checking,
    UpToDate,
    Downloading { version: String },
    Ready { version: String, notes: Option<String> },
    Installing { version: String },
    Error { message: String },
}

pub struct Updates {
    status: Mutex<UpdateStatus>,
    ready: Mutex<Option<(Update, Vec<u8>)>>,
    busy: AtomicBool,
    tray_item: Mutex<Option<MenuItem<Wry>>>,
}

impl Updates {
    pub fn new() -> Self {
        Self {
            status: Mutex::new(if cfg!(debug_assertions) {
                UpdateStatus::Disabled
            } else {
                UpdateStatus::Idle
            }),
            ready: Mutex::new(None),
            busy: AtomicBool::new(false),
            tray_item: Mutex::new(None),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn set_tray_item(&self, item: MenuItem<Wry>) {
        *self.tray_item.lock().unwrap() = Some(item);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.lock().unwrap().is_some()
    }
}

/// Stores the new status, tells every window and keeps the tray item in step.
fn set(app: &AppHandle, status: UpdateStatus) -> UpdateStatus {
    let updates = app.state::<Updates>();
    *updates.status.lock().unwrap() = status.clone();
    let tray_text = match &status {
        UpdateStatus::Ready { version, .. } => format!("Restart to update to {version}"),
        UpdateStatus::Installing { .. } => "Updating…".to_string(),
        _ => TRAY_CHECK_TEXT.to_string(),
    };
    let item = updates.tray_item.lock().unwrap().clone();
    if let Some(item) = item {
        let _ = item.set_text(tray_text);
    }
    let _ = app.emit("update-status", &status);
    status
}

/// Starts the background checks (release builds only).
pub fn start(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK_AFTER);
        loop {
            tauri::async_runtime::block_on(check(&app, false));
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// Looks for a newer version and downloads it. `manual` checks report
/// failures; background ones fail quietly (the user may simply be offline).
pub async fn check(app: &AppHandle, manual: bool) -> UpdateStatus {
    let updates = app.state::<Updates>();
    if cfg!(debug_assertions) {
        return set(app, UpdateStatus::Disabled);
    }
    if updates.is_ready() || updates.busy.swap(true, Ordering::SeqCst) {
        return updates.status();
    }
    let result = find_and_download(app).await;
    updates.busy.store(false, Ordering::SeqCst);
    match result {
        Ok(status) => status,
        Err(e) => {
            log::warn!("update check failed: {e}");
            if manual {
                set(app, UpdateStatus::Error { message: e.to_string() })
            } else {
                set(app, UpdateStatus::Idle)
            }
        }
    }
}

async fn find_and_download(app: &AppHandle) -> Result<UpdateStatus, tauri_plugin_updater::Error> {
    set(app, UpdateStatus::Checking);
    let Some(update) = app.updater()?.check().await? else {
        log::info!("no update available");
        return Ok(set(app, UpdateStatus::UpToDate));
    };
    let version = update.version.clone();
    log::info!("update {version} available; downloading");
    set(app, UpdateStatus::Downloading { version: version.clone() });
    let bytes = update.download(|_, _| {}, || {}).await?;
    let notes = update.body.clone().filter(|b| !b.trim().is_empty());
    *app.state::<Updates>().ready.lock().unwrap() = Some((update, bytes));
    log::info!("update {version} downloaded and ready");
    Ok(set(app, UpdateStatus::Ready { version, notes }))
}

/// Installs the downloaded update and restarts. On Windows the installer takes
/// over and the app exits; it is started again once the install finishes.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let Some((update, bytes)) = updates.ready.lock().unwrap().take() else {
        return Err("No update has been downloaded yet".into());
    };
    let version = update.version.clone();
    set(app, UpdateStatus::Installing { version: version.clone() });
    log::info!("installing update {version}");
    log::logger().flush();
    app.state::<AppState>().power.set(false);
    if let Err(e) = update.install(&bytes) {
        log::error!("update {version} failed to install: {e}");
        let notes = update.body.clone().filter(|b| !b.trim().is_empty());
        *updates.ready.lock().unwrap() = Some((update, bytes));
        set(app, UpdateStatus::Ready { version, notes });
        return Err(e.to_string());
    }
    app.restart();
}

/// The tray's update item: restart into a ready update, or check now.
pub fn on_tray_click(app: &AppHandle) {
    if app.state::<Updates>().is_ready() {
        ui::spawn(app, |app| {
            let _ = install(app);
        });
    } else {
        ui::spawn(app, ui::open_settings);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            check(&app, true).await;
        });
    }
}
