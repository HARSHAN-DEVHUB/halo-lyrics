import "./report.ts";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { gradientFromPalette, tintFromPalette, withAlpha } from "./color.ts";
import { createDemo } from "./demo.ts";
import { Background } from "./render/background.ts";
import { IdleScreen } from "./render/idle.ts";
import { LyricsStage, PlainLyrics, updateCardDots } from "./render/lyrics.ts";
import { formatTime, PlaybackClock } from "./sync.ts";
import type {
  AppSnapshot,
  Artwork,
  LyricsEvent,
  MediaCommand,
  NowPlaying,
  Settings,
} from "./types.ts";
import { BACKGROUND_STYLES, LYRICS_STYLES, STYLE_NAMES } from "./types.ts";

const IDLE_AFTER_MS = 10_000;
const CHROME_HIDE_MS = 3_000;
const BLUR_STEPS = [0, 14, 28, 44, 60];
/** The loading screen stays at least this long (from page start), so it never just flashes. */
const LOADER_MIN_MS = 700;

const isTauri = "__TAURI_INTERNALS__" in window;
const demoMode = !isTauri || new URLSearchParams(location.search).has("demo");

// Esc must work even if anything below fails, so it is wired up first.
// (On Windows the backend also watches Esc natively; see src-tauri/src/safety.rs.)
window.addEventListener(
  "keydown",
  (e) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    if (!demoMode) invoke("close_lyrics").catch(() => {});
  },
  true,
);

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

const app = $("app");
const background = new Background($("bg"));
const stage = new LyricsStage($("stage"));
const plain = new PlainLyrics($("plain"), $("plain-inner"));
const idle = new IdleScreen($("idle-clock"), $("idle-date"));
const card = $("card");
const cardArt = $<HTMLImageElement>("card-art");
const cardTitle = $("card-title");
const cardArtist = $("card-artist");
const cardNote = $("card-note");
const cardDots = [...$("card-dots").children] as HTMLElement[];
const sideArt = $<HTMLImageElement>("side-art");
const barThumb = $<HTMLImageElement>("bar-thumb");
const barTitle = $("bar-title");
const barArtist = $("bar-artist");
const barElapsed = $("bar-elapsed");
const barTotal = $("bar-total");
const barFill = $("bar-fill");
const labelText = $("label-text");
const toastEl = $("toast");

const clock = new PlaybackClock();

let settings: Settings = {
  wordSize: 136,
  blur: 14,
  offsetMs: 0,
  monitor: null,
  mirrorAll: false,
  autoOpen: true,
  autoCloseMinutes: 0,
  launchAtLogin: false,
  hidePlayerBar: false,
  lyricsStyle: "cinematic",
  backgroundStyle: "artwork",
};
let np: NowPlaying | null = null;
let artCache: { key: string; art: Artwork } | null = null;
let lyrics: LyricsEvent | null = null;
let renderedLyricsKey = "";
let lastPlayingAt = 0;
let mode = "loading";
let chromeVisible = true;

// ------------------------------------------------------------------ backend glue

const demo = demoMode ? createDemo() : null;

function send(cmd: MediaCommand) {
  if (demo) {
    if (cmd === "togglePlayPause") demo.toggle();
    else demo.skip(cmd === "next" ? 1 : -1);
    onNowPlaying(demo.nowPlaying());
    return;
  }
  invoke("media_command", { cmd }).catch((e) => console.warn("media_command", e));
}

async function updateSettings(patch: Partial<Settings>) {
  if (demo) {
    applySettings({ ...settings, ...patch });
    return;
  }
  try {
    applySettings(await invoke<Settings>("update_settings", { patch }));
  } catch (e) {
    console.warn("update_settings", e);
  }
}

// ------------------------------------------------------------------ state updates

function onNowPlaying(next: NowPlaying | null) {
  const prevKey = np?.trackKey;
  if (next) {
    if (next.artwork) artCache = { key: next.artworkKey ?? next.trackKey, art: next.artwork };
    else if (artCache && next.artworkKey && artCache.key === next.artworkKey) next.artwork = artCache.art;
    else if (!next.artworkKey) artCache = null;
  }
  np = next;

  if (!next) {
    clock.update({ positionMs: clock.now(), positionAt: Date.now(), playing: false });
    refreshMode();
    return;
  }

  if (next.trackKey !== prevKey) clock.reset();
  clock.update({ positionMs: next.positionMs, positionAt: next.positionAt, playing: next.playing });
  if (next.playing) lastPlayingAt = Date.now();
  app.classList.toggle("playing", next.playing);

  const art = next.artwork;
  background.set(art?.dataUrl);
  const tint = tintFromPalette(art?.palette);
  app.style.setProperty("--tint", tint);
  app.style.setProperty("--tint-glow", withAlpha(tint, 0.4));
  app.style.setProperty("--tint-soft", withAlpha(tint, 0.1));
  gradientFromPalette(art?.palette).forEach((c, i) => app.style.setProperty(`--g${i}`, c));
  app.classList.toggle("no-art", !art);

  setImg(barThumb, art?.dataUrl);
  setImg(cardArt, art?.dataUrl);
  setImg(sideArt, art?.dataUrl);
  setText(barTitle, next.title);
  setText(barArtist, next.artist);
  setText(cardTitle, next.title || "Unknown title");
  setText(cardArtist, next.artist);
  setText(barTotal, next.durationMs > 0 ? formatTime(next.durationMs) : "–:––");
  setText(labelText, next.title ? `LYRICS · ${next.title}` : "LYRICS");

  refreshMode();
}

function onLyrics(ev: LyricsEvent) {
  lyrics = ev;
  refreshMode();
}

function setText(el: HTMLElement, text: string) {
  if (el.textContent !== text) el.textContent = text;
}

function setImg(img: HTMLImageElement, url: string | undefined) {
  if (url) {
    if (img.src !== url) img.src = url;
    img.classList.remove("empty");
  } else {
    img.removeAttribute("src");
    img.classList.add("empty");
  }
}

function setMode(m: string) {
  if (m === mode) return;
  mode = m;
  app.dataset.mode = m;
  if (m === "plain") requestAnimationFrame(() => plain.measure());
}

/** Decides which screen to show. Cheap; called on events and once a second. */
function refreshMode() {
  const nothingPlaying = !np || (!np.playing && Date.now() - lastPlayingAt > IDLE_AFTER_MS);
  if (nothingPlaying) {
    setMode("idle");
    idle.update();
    kick();
    return;
  }
  const current = np!;
  const ly = lyrics && lyrics.trackKey === current.trackKey ? lyrics.lyrics : null;
  const key = `${current.trackKey}|${ly ? ly.kind : "pending"}`;
  if (key !== renderedLyricsKey) {
    renderedLyricsKey = key;
    stage.clear();
    card.classList.remove("instrumental");
    card.classList.toggle("pending", !ly);
    setText(cardNote, ly ? "" : "Finding lyrics");
    if (ly?.kind === "synced") stage.setLines(ly.lines, current.durationMs);
    else if (ly?.kind === "plain") plain.set(ly.text);
    else if (ly?.kind === "instrumental") {
      card.classList.add("instrumental");
      setText(cardNote, "Instrumental");
    }
  }
  if (!ly) setMode("card");
  else if (ly.kind === "synced") setMode("synced");
  else if (ly.kind === "plain") setMode("plain");
  else setMode("card");
  kick();
}

// ------------------------------------------------------------------ frame loop

let rafId = 0;
let lastSecond = -1;

function needsFrames(): boolean {
  if (document.visibilityState !== "visible" || mode === "idle") return false;
  // The "Finding lyrics" dots animate even while paused.
  return clock.isPlaying || (mode === "card" && card.classList.contains("pending"));
}

function frame() {
  rafId = 0;
  const t = clock.now();
  if (mode === "synced") stage.update(t + settings.offsetMs);
  else if (mode === "plain" && np) plain.update(t, np.durationMs);
  else if (mode === "card" && np && card.classList.contains("instrumental"))
    updateCardDots(cardDots, np.durationMs ? t / np.durationMs : 0, t);
  else if (mode === "card" && card.classList.contains("pending")) updateCardDots(cardDots, 0, performance.now());

  if (np && chromeVisible && !settings.hidePlayerBar && mode !== "idle") {
    const d = np.durationMs;
    const p = d > 0 ? Math.min(1, t / d) : 0;
    barFill.style.transform = `scaleX(${p.toFixed(4)})`;
    const sec = Math.floor(t / 1000);
    if (sec !== lastSecond) {
      lastSecond = sec;
      setText(barElapsed, formatTime(t));
    }
  }
  if (needsFrames()) rafId = requestAnimationFrame(frame);
}

/** Make sure exactly one rAF loop runs while needed, or render one frame otherwise. */
function kick() {
  if (!rafId) rafId = requestAnimationFrame(frame);
}

// 1 Hz housekeeping: idle clock, idle detection, demo heartbeat.
setInterval(() => {
  if (demo) onNowPlaying(demo.nowPlaying());
  if (mode === "idle") idle.update();
  refreshMode();
}, 1000);

document.addEventListener("visibilitychange", kick);

// ------------------------------------------------------------------ settings

function applySettings(s: Settings) {
  settings = s;
  const w = window.innerWidth;
  const h = window.innerHeight;
  // Scale down on screens narrower than 1280 px (and keep it sane on short ones).
  const scale = Math.min(1, w / 1280, h / 760);
  const active = Math.max(40, Math.round(s.wordSize * scale));
  app.style.setProperty("--active", `${active}px`);
  app.style.setProperty("--sung", `${Math.round(active * 0.34)}px`);
  app.style.setProperty("--blur", `${s.blur}px`);
  app.classList.toggle("no-bar", s.hidePlayerBar);
  app.dataset.layout = s.lyricsStyle;
  app.dataset.bg = s.backgroundStyle;
  plain.measure();
  kick();
}

window.addEventListener("resize", () => applySettings(settings));

// ------------------------------------------------------------------ chrome auto-hide

let chromeTimer = 0;
function showChrome() {
  if (!chromeVisible) {
    chromeVisible = true;
    app.classList.remove("chrome-off");
    kick();
  }
  clearTimeout(chromeTimer);
  chromeTimer = window.setTimeout(() => {
    chromeVisible = false;
    app.classList.add("chrome-off");
  }, CHROME_HIDE_MS);
}
window.addEventListener("mousemove", showChrome, { passive: true });
window.addEventListener("mousedown", showChrome, { passive: true });
showChrome();

// ------------------------------------------------------------------ toast

let toastTimer = 0;
function toast(text: string) {
  toastEl.textContent = text;
  toastEl.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toastEl.classList.remove("show"), 1400);
}

// ------------------------------------------------------------------ keyboard

window.addEventListener("keydown", (e) => {
  if (e.ctrlKey || e.metaKey || e.altKey) return;
  switch (e.key) {
    case " ":
      e.preventDefault();
      send("togglePlayPause");
      break;
    case "ArrowLeft":
      e.preventDefault();
      send("previous");
      break;
    case "ArrowRight":
      e.preventDefault();
      send("next");
      break;
    case "[":
    case "]": {
      e.preventDefault();
      const delta = e.key === "]" ? 100 : -100;
      const offsetMs = Math.max(-2000, Math.min(2000, settings.offsetMs + delta));
      updateSettings({ offsetMs });
      const sign = offsetMs > 0 ? "+" : offsetMs < 0 ? "−" : "±";
      toast(`Lyrics offset ${sign}${(Math.abs(offsetMs) / 1000).toFixed(1)} s`);
      break;
    }
    case "b":
    case "B": {
      const i = BLUR_STEPS.findIndex((v) => v > settings.blur);
      const blur = i === -1 ? BLUR_STEPS[0] : BLUR_STEPS[i];
      updateSettings({ blur });
      toast(`Blur ${blur}`);
      break;
    }
    case "h":
    case "H": {
      const hidePlayerBar = !settings.hidePlayerBar;
      updateSettings({ hidePlayerBar });
      toast(hidePlayerBar ? "Player bar hidden" : "Player bar shown");
      break;
    }
    case "s":
    case "S": {
      const lyricsStyle = nextOf(LYRICS_STYLES, settings.lyricsStyle);
      updateSettings({ lyricsStyle });
      toast(`Lyrics style · ${STYLE_NAMES[lyricsStyle]}`);
      break;
    }
    case "g":
    case "G": {
      const backgroundStyle = nextOf(BACKGROUND_STYLES, settings.backgroundStyle);
      updateSettings({ backgroundStyle });
      toast(`Background · ${STYLE_NAMES[backgroundStyle]}`);
      break;
    }
  }
});

function nextOf<T>(list: readonly T[], current: T): T {
  return list[(list.indexOf(current) + 1) % list.length];
}

$("btn-play").addEventListener("click", () => send("togglePlayPause"));
$("btn-prev").addEventListener("click", () => send("previous"));
$("btn-next").addEventListener("click", () => send("next"));

// ------------------------------------------------------------------ boot

/** Fades out the loading screen once the first real screen is ready. */
function hideLoader() {
  const loader = document.getElementById("loader");
  if (!loader) return;
  window.setTimeout(() => {
    loader.classList.add("done");
    window.setTimeout(() => loader.remove(), 700);
  }, Math.max(0, LOADER_MIN_MS - performance.now()));
}

async function boot() {
  // Everything above ran without throwing: tell the backend's load watchdog.
  if (!demo) invoke("frontend_ready").catch(() => {});
  applySettings(settings);
  if (demo) {
    // `?demo&style=karaoke&bg=split` previews a style (handy for screenshots).
    const q = new URLSearchParams(location.search);
    const style = LYRICS_STYLES.find((s) => s === q.get("style"));
    const bg = BACKGROUND_STYLES.find((s) => s === q.get("bg"));
    applySettings({ ...settings, ...(style && { lyricsStyle: style }), ...(bg && { backgroundStyle: bg }) });
    onNowPlaying(demo.nowPlaying());
    onLyrics(demo.lyrics);
    return;
  }
  await listen<NowPlaying | null>("now-playing", (e) => onNowPlaying(e.payload));
  await listen<LyricsEvent>("lyrics", (e) => onLyrics(e.payload));
  await listen<Settings>("settings", (e) => applySettings(e.payload));
  try {
    const snap = await invoke<AppSnapshot>("get_state");
    applySettings(snap.settings);
    if (snap.lyrics) lyrics = snap.lyrics;
    onNowPlaying(snap.nowPlaying);
  } catch (e) {
    console.warn("get_state", e);
    refreshMode();
  }
}

// The loading screen goes away whatever happens, so it can never get stuck.
boot().finally(hideLoader);
