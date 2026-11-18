// Shared types — mirror the Rust structs in src-tauri/src/media/mod.rs, lyrics/mod.rs and settings.rs.

export type Artwork = {
  dataUrl: string;
  /** 5 hex colours, most vivid first */
  palette: string[];
};

export type NowPlaying = {
  sourceApp: string;
  title: string;
  artist: string;
  album?: string;
  durationMs: number;
  /** position at `positionAt` */
  positionMs: number;
  /** epoch ms when position was sampled */
  positionAt: number;
  playing: boolean;
  /** Heartbeat events omit `artwork` to keep IPC light; `artworkKey` says which artwork is current. */
  artwork?: Artwork;
  artworkKey?: string;
  trackKey: string;
};

export type Word = { startMs: number; endMs: number; text: string };
export type Line = { startMs: number; endMs: number; text: string; words?: Word[] };

export type Lyrics =
  | { kind: "synced"; lines: Line[] }
  | { kind: "plain"; text: string }
  | { kind: "instrumental" }
  | { kind: "none" };

export type LyricsEvent = { trackKey: string; lyrics: Lyrics };

export type Settings = {
  wordSize: number;
  blur: number;
  offsetMs: number;
  monitor: string | null;
  mirrorAll: boolean;
  autoOpen: boolean;
  autoCloseMinutes: number;
  launchAtLogin: boolean;
  hidePlayerBar: boolean;
  lyricsStyle: LyricsStyle;
  backgroundStyle: BackgroundStyle;
  hdArtwork: boolean;
  windowMode: "fullscreen" | "window";
  /** The window's ✕ quits the app (otherwise it hides to the tray). */
  closeQuits: boolean;
};

// Keep in step with LYRICS_STYLES / BACKGROUND_STYLES in src-tauri/src/settings.rs.
export const LYRICS_STYLES = ["cinematic", "karaoke", "minimal", "party"] as const;
export const BACKGROUND_STYLES = ["artwork", "gradient", "dark", "split"] as const;
export type LyricsStyle = (typeof LYRICS_STYLES)[number];
export type BackgroundStyle = (typeof BACKGROUND_STYLES)[number];

export const STYLE_NAMES: Record<LyricsStyle | BackgroundStyle, string> = {
  cinematic: "Cinematic",
  karaoke: "Karaoke",
  minimal: "Minimal",
  party: "Party",
  artwork: "Album art",
  gradient: "Colour gradient",
  dark: "Plain dark",
  split: "Art on the side",
};

export type MonitorInfo = {
  id: string;
  label: string;
  primary: boolean;
};

export type AppSnapshot = {
  nowPlaying: NowPlaying | null;
  lyrics: LyricsEvent | null;
  settings: Settings;
  detectionPaused: boolean;
};

export type MediaCommand = "togglePlayPause" | "next" | "previous";
