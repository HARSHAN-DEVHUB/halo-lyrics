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
