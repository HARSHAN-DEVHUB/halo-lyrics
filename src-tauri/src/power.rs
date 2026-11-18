//! Keeps the display awake while lyrics mode is open and music is playing.
//!
//! * Windows: `SetThreadExecutionState` on a dedicated thread (the request is
//!   tied to the calling thread, so that thread has to stay alive).
//! * macOS: a `caffeinate -d -i -w <pid>` child, which holds the
//!   `PreventUserIdleDisplaySleep` IOPMAssertion for us and dies with the app.

use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

pub struct KeepAwake {
    tx: Mutex<Sender<bool>>,
    current: Mutex<bool>,
}

impl KeepAwake {
    pub fn new() -> Self {
        let (tx, rx) = channel::<bool>();
        std::thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                let mut guard = imp::Guard::default();
                while let Ok(on) = rx.recv() {
                    guard.set(on);
                }
                guard.set(false);
            })
            .expect("spawn keep-awake thread");
        Self { tx: Mutex::new(tx), current: Mutex::new(false) }
    }

    /// Cheap to call often: only state changes reach the OS.
    pub fn set(&self, on: bool) {
        let mut cur = self.current.lock().unwrap();
        if *cur == on {
            return;
        }
        *cur = on;
        log::info!("keep display awake: {on}");
        let _ = self.tx.lock().unwrap().send(on);
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};

    #[derive(Default)]
    pub struct Guard;

    impl Guard {
        pub fn set(&mut self, on: bool) {
            // SAFETY: plain Win32 call; flags are valid EXECUTION_STATE values.
            unsafe {
                if on {
                    SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
                } else {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::process::{Child, Command, Stdio};

    #[derive(Default)]
    pub struct Guard {
        child: Option<Child>,
    }

    impl Guard {
        pub fn set(&mut self, on: bool) {
            if on && self.child.is_none() {
                self.child = Command::new("/usr/bin/caffeinate")
                    .args(["-d", "-i", "-w", &std::process::id().to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| log::warn!("caffeinate failed: {e}"))
                    .ok();
            } else if !on {
                if let Some(mut c) = self.child.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            self.set(false);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    #[derive(Default)]
    pub struct Guard;
    impl Guard {
        pub fn set(&mut self, _on: bool) {}
    }
}
