//! Escape hatches for the full-screen, always-on-top lyrics overlay.
//!
//! The overlay covers the whole screen and sits above everything, so if the
//! page or the event loop ever hangs the user can be locked out of their only
//! machine. Two independent safety nets prevent that:
//!
//! 1. **Esc kill switch** (Windows): a dedicated thread polls the physical Esc
//!    key with `GetAsyncKeyState`. It needs no window, no focus, no message
//!    loop and no WebView, so it keeps working when everything else is stuck.
//!    It does not swallow Esc, so other apps are unaffected.
//!    * Tap Esc while the overlay is up → close the overlay. If it has not
//!      closed within [`HIDE_DEADLINE`], the process is terminated.
//!    * Hold Esc for [`HOLD_TO_KILL`] (pressed while the overlay was up) →
//!      terminate the process immediately.
//! 2. **Load watchdog**: if the lyrics page never reports that it booted within
//!    [`READY_TIMEOUT`] of the overlay being shown, the overlay is hidden again
//!    instead of leaving a black full-screen window behind.

use crate::ui;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::AppHandle;

/// Tap Esc: if the overlay is still up after this long, kill the process.
pub const HIDE_DEADLINE: Duration = Duration::from_millis(2000);
/// Holding Esc this long (pressed while the overlay was up) kills the process.
pub const HOLD_TO_KILL: Duration = Duration::from_millis(2000);
/// The lyrics page must call `frontend_ready` within this long of being shown.
pub const READY_TIMEOUT: Duration = Duration::from_secs(12);

static OVERLAY_ACTIVE: AtomicBool = AtomicBool::new(false);
static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
/// Bumped on every show, so a watchdog only acts on the show that started it.
static SHOW_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Set *before* the overlay is shown and cleared only *after* it is fully hidden,
/// so "still true" means "the user may be looking at a stuck overlay".
pub fn set_overlay_active(on: bool) {
    OVERLAY_ACTIVE.store(on, Ordering::SeqCst);
}

pub fn overlay_active() -> bool {
    OVERLAY_ACTIVE.load(Ordering::SeqCst)
}

pub fn mark_frontend_ready() {
    if !FRONTEND_READY.swap(true, Ordering::SeqCst) {
        log::info!("lyrics page ready");
    }
}

/// Call right after showing the overlay. Hides it again if the page never boots.
pub fn watch_frontend_load(app: &AppHandle) {
    let generation = SHOW_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if FRONTEND_READY.load(Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new().name("load-watchdog".into()).spawn(move || {
        std::thread::sleep(READY_TIMEOUT);
        let same_show = SHOW_GENERATION.load(Ordering::SeqCst) == generation;
        if same_show && !FRONTEND_READY.load(Ordering::SeqCst) && overlay_active() {
            log::error!("lyrics page did not load within {READY_TIMEOUT:?}; hiding the overlay");
            ui::hide_lyrics(&app);
        }
    });
}

/// Ends the process right now, without running any shutdown code that could
/// itself be stuck behind a hung thread.
pub fn force_exit(reason: &str) -> ! {
    log::error!("emergency exit: {reason}");
    log::logger().flush();
    imp::terminate()
}

/// Starts the Esc kill switch thread (no-op where unsupported).
pub fn start(app: &AppHandle) {
    imp::start(app.clone());
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::time::Instant;
    use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};

    const POLL: Duration = Duration::from_millis(30);

    fn esc_down() -> bool {
        // SAFETY: plain Win32 call with a valid virtual-key code. The high bit
        // of the result means "currently down".
        let state = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) };
        (state as u16) & 0x8000 != 0
    }

    pub fn terminate() -> ! {
        // SAFETY: terminating our own process; the pseudo-handle is always valid.
        unsafe {
            let _ = TerminateProcess(GetCurrentProcess(), 1);
        }
        // TerminateProcess on ourselves does not return; this is a fallback.
        std::process::exit(1)
    }

    pub fn start(app: AppHandle) {
        let spawned = std::thread::Builder::new().name("esc-kill-switch".into()).spawn(move || {
            // When Esc went down, and whether the overlay was up at that moment.
            let mut press: Option<(Instant, bool)> = None;
            let mut hide_deadline: Option<Instant> = None;
            loop {
                std::thread::sleep(POLL);
                let now = Instant::now();

                if esc_down() {
                    match press {
                        None => {
                            let armed = overlay_active();
                            press = Some((now, armed));
                            if armed {
                                log::info!("Esc: closing lyrics overlay");
                                hide_deadline = Some(now + HIDE_DEADLINE);
                                ui::spawn(&app, ui::hide_lyrics);
                            }
                        }
                        Some((since, true)) if now.duration_since(since) >= HOLD_TO_KILL => {
                            force_exit("Esc held while the overlay was open");
                        }
                        Some(_) => {}
                    }
                } else {
                    press = None;
                }

                if let Some(deadline) = hide_deadline {
                    if !overlay_active() {
                        hide_deadline = None;
                    } else if now >= deadline {
                        force_exit("overlay did not close after Esc");
                    }
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("could not start the Esc kill switch: {e}");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn terminate() -> ! {
        std::process::exit(1)
    }

    /// Other platforms rely on the page's Esc handler, the global shortcut and
    /// the load watchdog.
    pub fn start(_app: AppHandle) {}
}
