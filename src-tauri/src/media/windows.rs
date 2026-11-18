//! Windows: System Media Transport Controls (SMTC).
//!
//! One dedicated thread owns all WinRT objects. SMTC events only *wake* the
//! thread; it then re-reads the state, so a missed or duplicated event can never
//! leave us out of sync. A 1 s timeout doubles as the heartbeat.

use super::{friendly_app_name, now_ms, MediaCommand, NowPlayingSource, RawNowPlaying, SnapshotSender};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

/// 100 ns ticks between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_TICKS: i64 = 116_444_736_000_000_000;
const MAX_ARTWORK_BYTES: u64 = 20 * 1024 * 1024;
const ARTWORK_RETRIES: u8 = 6;

enum Msg {
    /// Something changed somewhere; re-read everything.
    Refresh,
    /// Media properties of this session changed (title, artwork...).
    Props(String),
    /// Playback status / timeline of this session changed.
    Playback(String),
    Cmd(MediaCommand),
}

pub struct SmtcSource {
    tx: Mutex<Option<Sender<Msg>>>,
}

impl SmtcSource {
    pub fn new() -> Self {
        Self { tx: Mutex::new(None) }
    }
}

impl NowPlayingSource for SmtcSource {
    fn start(&self, out: SnapshotSender) {
        let (tx, rx) = channel::<Msg>();
        *self.tx.lock().unwrap() = Some(tx.clone());
        std::thread::Builder::new()
            .name("smtc".into())
            .spawn(move || {
                // SAFETY: called once at thread start, before any WinRT use.
                unsafe {
                    let _ = RoInitialize(RO_INIT_MULTITHREADED);
                }
                let mut backoff = Duration::from_secs(1);
                loop {
                    match run(&tx, &rx, &out) {
                        Ok(()) => return, // channel closed: app is quitting
                        Err(e) => {
                            log::warn!("SMTC loop failed: {e:?}; retrying in {backoff:?}");
                            let _ = out.send(None);
                            std::thread::sleep(backoff);
                            backoff = (backoff * 2).min(Duration::from_secs(30));
                        }
                    }
                }
            })
            .expect("spawn smtc thread");
    }

    fn command(&self, cmd: MediaCommand) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(Msg::Cmd(cmd));
        }
    }
}

struct Tracked {
    session: Session,
    playing: bool,
    changed_at: Instant,
}

#[derive(Default)]
struct ArtState {
    /// session id + title + artist + album
    sig: String,
    bytes: Option<Arc<Vec<u8>>>,
    attempts: u8,
}

fn run(tx: &Sender<Msg>, rx: &std::sync::mpsc::Receiver<Msg>, out: &SnapshotSender) -> windows::core::Result<()> {
    let manager = Manager::RequestAsync()?.get()?;
    log::info!("SMTC session manager ready");

    {
        let t = tx.clone();
        manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }))?;
        let t = tx.clone();
        manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = t.send(Msg::Refresh);
            Ok(())
        }))?;
    }

    let mut tracked: HashMap<String, Tracked> = HashMap::new();
    let mut art = ArtState::default();
    let mut chosen: Option<String> = None;
    let mut force_props_for: HashSet<String> = HashSet::new();
    let mut timeout = Duration::from_millis(0);

    loop {
        // Wait for an event or the heartbeat, then drain everything queued.
        let mut msgs = Vec::new();
        match rx.recv_timeout(timeout) {
            Ok(m) => msgs.push(m),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        timeout = Duration::from_millis(1000);

        for m in msgs {
            match m {
                Msg::Refresh => {}
                Msg::Props(id) => {
                    if let Some(t) = tracked.get_mut(&id) {
                        t.changed_at = Instant::now();
                    }
                    force_props_for.insert(id);
                }
                Msg::Playback(id) => {
                    if let Some(t) = tracked.get_mut(&id) {
                        t.changed_at = Instant::now();
                    }
                }
                Msg::Cmd(cmd) => {
                    if let Some(t) = chosen.as_ref().and_then(|id| tracked.get(id)) {
                        if let Err(e) = send_command(&t.session, cmd) {
                            log::warn!("SMTC command {cmd:?} failed: {e:?}");
                        }
                    }
                    // Pick up the new state quickly.
                    timeout = Duration::from_millis(150);
                }
            }
        }

        sync_sessions(&manager, &mut tracked, tx)?;
        chosen = choose(&manager, &tracked);

        let snapshot = match chosen.as_ref().and_then(|id| tracked.get(id).map(|t| (id, t))) {
            Some((id, t)) => {
                let force = force_props_for.remove(id);
                match read_session(id, &t.session, &mut art, force) {
                    Ok(s) => s,
                    Err(e) => {
                        log::debug!("reading session {id} failed: {e:?}");
                        None
                    }
                }
            }
            None => None,
        };
        force_props_for.clear();
        if out.send(snapshot).is_err() {
            return Ok(());
        }
    }
}

/// Keeps `tracked` in step with the OS session list and subscribes to new sessions.
fn sync_sessions(manager: &Manager, tracked: &mut HashMap<String, Tracked>, tx: &Sender<Msg>) -> windows::core::Result<()> {
    let sessions = manager.GetSessions()?;
    let mut seen = HashSet::new();
    for i in 0..sessions.Size()? {
        let session = sessions.GetAt(i)?;
        let id = session.SourceAppUserModelId()?.to_string();
        let playing = session
            .GetPlaybackInfo()
            .and_then(|p| p.PlaybackStatus())
            .map(|s| s == Status::Playing)
            .unwrap_or(false);
        seen.insert(id.clone());
        match tracked.get_mut(&id) {
            Some(t) => {
                if t.playing != playing {
                    t.playing = playing;
                    t.changed_at = Instant::now();
                }
                t.session = session;
            }
            None => {
                subscribe(&session, &id, tx);
                tracked.insert(id, Tracked { session, playing, changed_at: Instant::now() });
            }
        }
    }
    tracked.retain(|id, _| seen.contains(id));
    Ok(())
}

fn subscribe(session: &Session, id: &str, tx: &Sender<Msg>) {
    let (t, i) = (tx.clone(), id.to_string());
    let _ = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
        let _ = t.send(Msg::Props(i.clone()));
        Ok(())
    }));
    let (t, i) = (tx.clone(), id.to_string());
    let _ = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
        let _ = t.send(Msg::Playback(i.clone()));
        Ok(())
    }));
    let (t, i) = (tx.clone(), id.to_string());
    let _ = session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
        let _ = t.send(Msg::Playback(i.clone()));
        Ok(())
    }));
}

/// Prefer a playing session (the most recently changed one if several play),
/// then the OS "current" session, then anything.
fn choose(manager: &Manager, tracked: &HashMap<String, Tracked>) -> Option<String> {
    if let Some((id, _)) = tracked
        .iter()
        .filter(|(_, t)| t.playing)
        .max_by_key(|(_, t)| t.changed_at)
    {
        return Some(id.clone());
    }
    if let Ok(current) = manager.GetCurrentSession() {
        if let Ok(id) = current.SourceAppUserModelId() {
            let id = id.to_string();
            if tracked.contains_key(&id) {
                return Some(id);
            }
        }
    }
    tracked.iter().max_by_key(|(_, t)| t.changed_at).map(|(id, _)| id.clone())
}

fn read_session(id: &str, session: &Session, art: &mut ArtState, force_art: bool) -> windows::core::Result<Option<RawNowPlaying>> {
    let props = session.TryGetMediaPropertiesAsync()?.get()?;
    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();
    if title.trim().is_empty() {
        return Ok(None);
    }

    let playing = session
        .GetPlaybackInfo()
        .and_then(|p| p.PlaybackStatus())
        .map(|s| s == Status::Playing)
        .unwrap_or(false);

    // Timeline. Some apps (notably Spotify) update it rarely, so always
    // extrapolate from LastUpdatedTime while playing.
    let now = now_ms();
    let (mut position_ms, mut duration_ms) = (0u64, 0u64);
    if let Ok(tl) = session.GetTimelineProperties() {
        let ticks_to_ms = |t: i64| (t / 10_000).max(0) as u64;
        let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
        let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
        let pos = tl.Position().map(|t| t.Duration).unwrap_or(0);
        duration_ms = ticks_to_ms(end - start);
        position_ms = ticks_to_ms(pos - start);
        if playing {
            if let Ok(updated) = tl.LastUpdatedTime() {
                let updated_ms = (updated.UniversalTime - EPOCH_DIFF_TICKS) / 10_000;
                if updated_ms > 0 {
                    let elapsed = now as i64 - updated_ms;
                    if elapsed > 0 {
                        position_ms += elapsed as u64;
                    }
                }
            }
        }
        if duration_ms > 0 {
            position_ms = position_ms.min(duration_ms);
        }
    }

    // Artwork: only re-read when the track changes, when SMTC says the
    // properties changed, or while we're still waiting for a late thumbnail.
    let sig = format!("{id}\u{1f}{title}\u{1f}{artist}\u{1f}{album}");
    let need = if art.sig != sig {
        *art = ArtState { sig, bytes: None, attempts: 0 };
        true
    } else {
        force_art || (art.bytes.is_none() && art.attempts < ARTWORK_RETRIES)
    };
    if need {
        art.attempts = art.attempts.saturating_add(1);
        match read_thumbnail(&props) {
            Ok(Some(bytes)) => {
                let same = art.bytes.as_ref().is_some_and(|b| **b == bytes);
                if !same {
                    art.bytes = Some(Arc::new(bytes));
                }
            }
            Ok(None) => {}
            Err(e) => log::debug!("thumbnail read failed: {e:?}"),
        }
    }

    Ok(Some(RawNowPlaying {
        source_app: friendly_app_name(id),
        source_id: id.to_string(),
        title,
        artist,
        album: if album.trim().is_empty() { None } else { Some(album) },
        duration_ms,
        position_ms,
        position_at: now,
        playing,
        artwork: art.bytes.clone(),
    }))
}

fn read_thumbnail(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> windows::core::Result<Option<Vec<u8>>> {
    let thumb = match props.Thumbnail() {
        Ok(t) => t,
        Err(_) => return Ok(None), // no thumbnail set
    };
    let stream = thumb.OpenReadAsync()?.get()?;
    let size = stream.Size()?;
    if size == 0 || size > MAX_ARTWORK_BYTES {
        return Ok(None);
    }
    let input = stream.GetInputStreamAt(0)?;
    let reader = DataReader::CreateDataReader(&input)?;
    let loaded = reader.LoadAsync(size as u32)?.get()?;
    let mut buf = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut buf)?;
    Ok(Some(buf))
}

fn send_command(session: &Session, cmd: MediaCommand) -> windows::core::Result<()> {
    let ok = match cmd {
        MediaCommand::TogglePlayPause => session.TryTogglePlayPauseAsync()?.get()?,
        MediaCommand::Next => session.TrySkipNextAsync()?.get()?,
        MediaCommand::Previous => session.TrySkipPreviousAsync()?.get()?,
    };
    if !ok {
        log::info!("session refused {cmd:?}");
    }
    Ok(())
}
