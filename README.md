# Halo Lyrics

**Word-by-word synced lyrics for whatever is playing on your computer, full screen or in a window.**

Halo Lyrics is a small desktop app for Windows and macOS. It notices what's playing (Spotify, YouTube in the browser, Apple Music, VLC, and more), fetches synced lyrics, and shows them over the album art, either covering the whole screen or in a normal window you can move, resize and close.

- [Install](#install-windows) · [Uninstall](#uninstall-windows) · [Using it](#using-it) · [Running from source](#running-from-source)

Built with [Tauri 2](https://tauri.app) (Rust backend) and plain TypeScript + Vite (no frontend framework).

---

## Features

- **Works with any player.** Reads the operating system's media controls, so there's nothing to connect or log in to.
  - Windows: System Media Transport Controls (SMTC)
  - macOS: MediaRemote, via [mediaremote-adapter](https://github.com/ungive/mediaremote-adapter)
- **Word-by-word highlighting.** Uses enhanced-LRC word timing when available and synthesizes it otherwise.
- **Free lyrics source.** Uses [LRCLIB](https://lrclib.net), so no API key is needed. Results are cached on disk.
- **Your own lyrics win.** Drop `.lrc` or `.txt` files into a folder to override the downloaded lyrics.
- **Smart title cleanup.** Handles YouTube titles like `Artist - Song (Official Video)`, VEVO and "- Topic" channels, `feat.` credits, and "Remastered 2011".
- **Ambient look.** The album art is blurred and slowly drifts, and the text takes a tint from the artwork's colours. A clock appears when nothing is playing.
- **Styles you can pick.** Four lyrics styles (Cinematic, Karaoke, Minimal, Party) and four backgrounds (Album art, Colour gradient, Plain dark, Art on the side), chosen separately in Settings or cycled with `S` and `G`.
- **Sharp artwork.** Browsers only hand the OS a small thumbnail, so when the cover is small the app fetches the same song's cover in high resolution (up to 2000 px) from iTunes or Deezer. It only swaps when the title and artist match, and it can be turned off.
- **Full screen or a window.** Switch any time with `F` / `F11` or the button at the top right. In a window you get the usual title bar with minimise, maximise and close.
- **Closes like a normal app.** The ✕ button quits Halo Lyrics. If you prefer, Settings can make ✕ hide it to the tray instead, so it can open by itself when music starts.
- **One copy at a time.** Starting the app again just brings up the copy that's already running.
- **Multi-monitor.** Pick a display, or mirror the lyrics on every display.
- **Auto-open and auto-close.** Opens when music starts and closes after music stops (both optional).
- **Keeps the screen awake** while lyrics are showing and music is playing.
- **Safe full-screen.** Esc always gets you out, even if the app hangs (see [Safety](#safety-escape-hatches)).

---

## Install (Windows)

You need the installer file, `Halo Lyrics_0.1.0_x64-setup.exe`. To make it yourself, see [Build the installer](#build-the-installer); it ends up in `src-tauri\target\release\bundle\nsis\`.

1. Double-click **`Halo Lyrics_0.1.0_x64-setup.exe`**.
2. Windows may show **"Windows protected your PC"**. This appears for every app that isn't signed with a paid code-signing certificate; it doesn't mean anything was found. Click **More info → Run anyway**.
3. Follow the installer. It installs for your user account only, so it doesn't need administrator rights. If the WebView2 runtime is missing (rare on Windows 10/11), the installer downloads it.
4. Start **Halo Lyrics** from the Start menu.

There's also an `.msi` installer in `src-tauri\target\release\bundle\msi\` if you prefer that format; it installs for all users and asks for administrator rights. Use one installer or the other, not both.

To update, run the newer installer over the old one. Your settings and lyrics cache are kept.

## Uninstall (Windows)

1. Quit Halo Lyrics: close its window with ✕, or right-click the tray icon → **Quit Halo Lyrics**.
2. If you turned on **Launch at login**, turn it off first in Settings (this removes the start-up entry).
3. Open **Settings → Apps → Installed apps**, find **Halo Lyrics**, click **⋯ → Uninstall**. (Or Start menu → Halo Lyrics folder → **Uninstall**, or **Control Panel → Programs and Features**.)
4. In the uninstaller, tick **Delete the application data** to also remove your settings, lyrics cache and logs.

If you didn't tick that box, or used the `.msi`, you can delete these folders by hand afterwards:

| Folder | Contains |
|---|---|
| `%APPDATA%\app.halolyrics.desktop` | Settings, lyrics cache |
| `%LOCALAPPDATA%\app.halolyrics.desktop` | Logs, web view data |
| `%USERPROFILE%\Music\Lyrics` | Your own `.lrc` files (only if you made them; keep them if you want) |

Paste a path into the File Explorer address bar to open it.

---

## Using it

### Start

1. Launch Halo Lyrics. It opens full screen (or in a window, if you chose that) and shows a clock if nothing is playing.
2. Play a song in Spotify, YouTube (Chrome or Edge), or any media player.
3. Lyrics appear within a second or two.

**Window controls:** move the mouse and two buttons appear at the top right: **⤡** switches between full screen and a window, **✕** closes. In a window, the normal title bar has minimise, maximise and close.

**Closing:** ✕ quits the app (you can change this in Settings → Display so that it hides to the tray instead). Esc closes the full-screen view.

**Tray icon:** Halo Lyrics also has a tray icon. On Windows 11 it's usually hidden under the `^` arrow next to the clock; drag it onto the taskbar to keep it visible. Left-click it to show or hide the lyrics. Right-click it for **Settings**, **Pause detection** and **Quit**.

### Keyboard shortcuts

| Key | Action |
|---|---|
| `Ctrl` + `Shift` + `L` (`⌘` + `Shift` + `L` on macOS) | Open or close the lyrics screen (works from anywhere) |
| `F` or `F11` | Switch between full screen and a window |
| `Esc` (tap, full screen) | Close the full-screen lyrics. The app stays in the tray. |
| `Esc` (hold 2 s, full screen) | **Force-quit the app** (emergency exit, Windows) |
| `Space` | Play / pause |
| `←` / `→` | Previous / next track |
| `[` / `]` | Shift the lyrics timing by −/+ 0.1 s |
| `B` | Cycle the background blur |
| `H` | Hide or show the player bar |
| `S` | Next lyrics style |
| `G` | Next background style |
| `,` | Open Settings (also the sliders button on the player bar) |

### What you'll see

| Screen | Meaning |
|---|---|
| Lines moving with the song, active word highlighted | Synced lyrics found |
| Slowly scrolling text marked *unsynced* | Only plain (untimed) lyrics exist |
| Album art + title, or *Instrumental* | No lyrics found, or the track is instrumental |
| Big clock | Nothing has been playing for 10 seconds |

### Settings

Open them with the sliders button on the player bar, the `,` key, or the tray menu → **Settings…** (on Windows 11 the tray icon may be hidden under the `^` arrow next to the clock).

- **Style:** lyrics style and background style, each picked from preview cards (see below)
- **Look:** word size, background blur, lyrics offset, hide the player bar, high-resolution artwork
- **Display:** full screen or a window, whether ✕ quits or hides to the tray, which monitor to use, mirror on all displays (full screen only)
- **Behaviour:** auto-open when music starts, auto-close after music stops, launch at login
- **Lyrics:** clear the lyrics cache

### Styles

The lyrics style and the background are separate settings, so any lyrics style works with any background.

| Lyrics style | What it looks like |
|---|---|
| **Cinematic** (default) | Previous, current and next line; the word being sung grows into a large italic serif |
| **Karaoke** | Current and next line low on the screen; sung words fill in with the artwork's colour |
| **Minimal** | Only the current line, calm and medium-sized |
| **Party** | One huge, bold, uppercase line, readable from across a room |

| Background | What it looks like |
|---|---|
| **Album art** (default) | The cover, blurred and slowly drifting |
| **Colour gradient** | Soft, drifting blobs in the cover's colours |
| **Plain dark** | No picture, just a faint glow in the cover's colour |
| **Art on the side** | The sharp cover on the left, lyrics on the right |

The **Word size** slider scales every lyrics style.

### Using your own lyrics files

Put a file named `Artist - Title.lrc` in your Music folder's `Lyrics` subfolder:

- Windows: `C:\Users\<you>\Music\Lyrics\`
- macOS: `~/Music/Lyrics/`

Files with `[mm:ss.xx]` timestamps are shown synced; files without them are shown as plain text. Enhanced LRC word tags (`<mm:ss.xx>word`) are supported. Characters that aren't allowed in file names (`: / \ ? * " < > |`) can be left out or replaced with `_`. Files larger than 1 MB are ignored.

### Tips for best results

- **Spotify** and other music apps give the most accurate matches because they report the exact title and artist.
- On **YouTube**, official music videos and "Artist - Topic" uploads match best. Covers, remixes, "lyrics" edits and oddly named videos often don't.
- If lyrics are a little early or late, press `[` or `]`, or set the offset in Settings.
- LRCLIB is community-run. Most popular songs are there, including many non-English songs, but not every song is. You can [contribute missing lyrics](https://lrclib.net).

---

## Running from source

### Prerequisites

| Tool | Version | Notes |
|---|---|---|
| [Node.js](https://nodejs.org) | 20.19+ or 22+ | Required by Vite 7 |
| [Rust](https://rustup.rs) | 1.82+ | Install with `rustup` |
| **Windows:** [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) | — | Select the "Desktop development with C++" workload |
| **Windows:** WebView2 | — | Preinstalled on Windows 10 (recent) and 11 |
| **macOS:** Xcode Command Line Tools | — | `xcode-select --install` |

See Tauri's [prerequisites guide](https://v2.tauri.app/start/prerequisites/) if anything is missing.

### Install and run (development)

```bash
git clone https://github.com/<your-name>/halo-lyrics.git
cd halo-lyrics
npm install
npm run tauri dev
```

The first run compiles the Rust side and can take a few minutes; later runs are fast. `npm run tauri dev` starts the Vite dev server on port **1420** and opens the app. Stop it by closing the window with ✕, with **Quit** in the tray menu, or with `Ctrl+C` in the terminal.

`npm run tauri dev` is only for working on the code. For everyday use, [build the installer](#build-the-installer) and [install](#install-windows) the app.

> **Low-spec laptop?** Limit the compile so it doesn't eat every core:
> - PowerShell: `$env:CARGO_BUILD_JOBS=4; npm run tauri dev`
> - bash/zsh: `CARGO_BUILD_JOBS=4 npm run tauri dev`

### Preview the UI in a browser (no Rust needed)

```bash
npm run dev
```

Open <http://localhost:1420>. Outside Tauri, the page runs in **demo mode** with a built-in fake song, which is handy for working on the look. You can force demo mode inside the app with `?demo`.

### Build the installer

```powershell
npm install
$env:CARGO_BUILD_JOBS=4; npm run tauri build
```

Release builds are fully optimised and slow, so expect 10–20 minutes on a laptop (the first time, Tauri also downloads the NSIS and WiX installer tools). Output goes to:

- Windows: `src-tauri\target\release\bundle\nsis\Halo Lyrics_<version>_x64-setup.exe` (recommended) and `src-tauri\target\release\bundle\msi\Halo Lyrics_<version>_x64_en-US.msi`
- macOS: `src-tauri/target/release/bundle/dmg/` and `.../bundle/macos/`

Then follow [Install](#install-windows). To release a new version, raise `version` in `src-tauri/tauri.conf.json` (and `package.json`, `src-tauri/Cargo.toml`) before building.

### macOS: media adapter

Since macOS 15.4, apps can't read "Now Playing" directly. Halo Lyrics bundles [ungive/mediaremote-adapter](https://github.com/ungive/mediaremote-adapter), which is **not committed** to this repo. Before building on macOS, build or download it and place these files:

```
src-tauri/resources/macos/mediaremote-adapter.pl
src-tauri/resources/macos/MediaRemoteAdapter.framework/
```

If the adapter is missing, the app falls back to polling Spotify and Apple Music through AppleScript. Windows builds don't need this.

### Tests

```bash
npm test              # frontend: timing / sync logic (node --test)
npm run test:rust     # backend: LRC parser, title cleanup, matching, cache, artwork
npx tsc --noEmit      # type-check
```

---

## Safety: escape hatches

In full-screen mode the lyrics cover the whole screen and stay on top. To make sure that can never lock you out of your computer:

1. **Esc kill switch (Windows, full screen only).** In window mode it's switched off, because the window can't trap you and Esc belongs to whatever app you're using. A dedicated background thread watches the physical Esc key with `GetAsyncKeyState`. It doesn't depend on the window, the web page or the app's main thread, so it still works if those hang. It doesn't block Esc for other apps.
   - **Tap Esc:** the lyrics screen closes. If it hasn't closed within 2 s, the app terminates itself.
   - **Hold Esc for 2 s:** the app terminates immediately.
2. **Load watchdog.** If the lyrics page hasn't started within 12 s of being shown (for example, the dev server isn't running), the screen hides itself instead of leaving a black window.
3. **The page's own Esc handler**, registered before anything else on the page runs.
4. **The ✕ button** at the top right (move the mouse to show it), **Tray → Quit**, and `Ctrl+Shift+L` to toggle.

### Security

- The web pages can only load the app's own files: a strict Content-Security-Policy blocks remote scripts, styles and connections, and the Tauri JavaScript API isn't exposed globally.
- The pages may only call the app's own commands plus closing/hiding a window and opening a link. Every settings change is validated and clamped in Rust.
- Network requests use HTTPS only, have timeouts and size limits, and only go to the services listed under [Privacy](#privacy). Cover images are only downloaded from Apple's and Deezer's image servers.
- Images from players and the internet are decoded with size and memory limits, so a broken or malicious image can't exhaust memory.
- Crashes and page errors are written to the log file, which stays on your computer.
- The installer isn't code-signed (that needs a paid certificate), which is why Windows shows the "Windows protected your PC" notice on first install.

The logic lives in [`src-tauri/src/safety.rs`](src-tauri/src/safety.rs).

---

## Troubleshooting

| Problem | Fix |
|---|---|
| Screen is stuck or black | Tap **Esc**. If that doesn't help, **hold Esc for 2 seconds**. As a last resort, use `Ctrl+Shift+Esc` → Task Manager → end `halo-lyrics.exe`. |
| Can't find the window or the tray icon | Press `Ctrl+Shift+L`. The tray icon is often hidden under the `^` arrow next to the clock. |
| "Windows protected your PC" when installing | Expected for unsigned apps: **More info → Run anyway**. |
| `Port 1420 is already in use` | An old dev server is still running. Close the old terminal, or end the `node.exe` process running Vite. |
| No song detected | Check that the player shows up in the Windows volume / media flyout. Some players don't publish media info. Make sure **Pause detection** in the tray menu is unchecked. |
| Song detected but "no lyrics" | The song may not be on LRCLIB, or the title is unusual (common with YouTube). Add your own `.lrc` file (see above). |
| Wrong lyrics | Clear the lyrics cache in Settings, or add an override `.lrc` file. |
| The app closed by itself | Crashes and page errors are written to the log file (see below). Search it for `crash` or `page]`. |
| `Ctrl+Shift+L` does nothing | Another app has taken that shortcut. Use the tray icon instead. |
| Build fails on Windows with linker errors | Install the C++ Build Tools ("Desktop development with C++"). |

### Where files are stored (Windows)

| What | Location |
|---|---|
| Settings | `%APPDATA%\app.halolyrics.desktop\settings.json` |
| Lyrics cache | `%APPDATA%\app.halolyrics.desktop\lyrics-cache\` |
| Logs | `%LOCALAPPDATA%\app.halolyrics.desktop\logs\halo-lyrics.log` |
| Your lyrics overrides | `%USERPROFILE%\Music\Lyrics\` |

On macOS these are under `~/Library/Application Support/app.halolyrics.desktop/` and `~/Library/Logs/app.halolyrics.desktop/`.

---

## How it works

```
 OS media controls ──► media/ (SMTC on Windows, adapter on macOS)
                          │  raw snapshots (title, artist, position, artwork)
                          ▼
                       hub.rs ──► artwork.rs (resize, JPEG, colour palette)
                          │   ──► lyrics/ (override folder → cache → LRCLIB → LRC parser)
                          │   ──► auto-open / auto-close / keep-awake
                          ▼
            "now-playing" + "lyrics" events
                          ▼
     Web UI (src/): playback clock, word timing, rendering
```

### Project layout

```
index.html, settings.html     The two windows
src/
  main.ts                     Lyrics window: state, frame loop, keyboard
  settings.ts                 Settings window
  sync.ts                     Playback clock + line/word timing (unit-tested)
  color.ts                    OKLCH tint and gradient colours from the artwork palette
  report.ts                   Sends page errors to the backend log
  demo.ts                     Fake song for browser/demo mode
  render/                     Background, lyrics stage, idle clock
src-tauri/
  src/main.rs                 Commands, tray, global shortcut, startup
  src/ui.rs                   Full-screen window placement, mirrors, settings window
  src/safety.rs               Esc kill switch + load watchdog
  src/hub.rs                  Turns media snapshots into UI events
  src/media/                  Windows SMTC / macOS adapter
  src/lyrics/                 LRCLIB client, LRC parser, title/artist normalisation, cache
  src/artwork.rs              Artwork processing + palette
  src/artwork_hd.rs           High-res cover lookup (iTunes, Deezer)
  src/power.rs                Keep the display awake
  src/settings.rs             Settings file (validated + clamped)
```

### Privacy

Halo Lyrics only makes these network requests:

- **Lyrics:** `lrclib.net`, with the song title, artist, album and duration.
- **High-resolution artwork** (on by default, can be turned off in Settings → Look): when the player only provides a small cover, `itunes.apple.com` and `api.deezer.com` are searched with the song title and artist, and the matching cover image is downloaded.

Nothing else leaves your computer. There's no analytics and no account.

---

## Credits

- Lyrics: [LRCLIB](https://lrclib.net), a free, open lyrics database
- macOS now-playing: [ungive/mediaremote-adapter](https://github.com/ungive/mediaremote-adapter)
- Fonts: [Geist](https://vercel.com/font) and [Instrument Serif](https://github.com/Instrument/instrument-serif), both under the SIL Open Font License (see `public/fonts/OFL-*.txt`)
- Built with [Tauri](https://tauri.app) and [Vite](https://vite.dev)

## License

[MIT](LICENSE) © 2026 HARSHAN-DEVHUB
