import "./report.ts";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { MonitorInfo, Settings } from "./types.ts";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

type NumKey = "wordSize" | "blur" | "offsetMs";
type BoolKey = "mirrorAll" | "autoOpen" | "launchAtLogin" | "hidePlayerBar";

const ranges: Record<NumKey, (v: number) => string> = {
  wordSize: (v) => `${v}px`,
  blur: (v) => `${v}`,
  offsetMs: (v) => `${v > 0 ? "+" : ""}${v} ms`,
};
const checks: BoolKey[] = ["mirrorAll", "autoOpen", "launchAtLogin", "hidePlayerBar"];
/** Radio groups whose `name` is the settings key. */
const radios = ["lyricsStyle", "backgroundStyle"] as const;

const radioInputs = (name: (typeof radios)[number]) =>
  [...document.querySelectorAll<HTMLInputElement>(`input[type="radio"][name="${name}"]`)];

let current: Settings | null = null;

async function save(patch: Partial<Settings>) {
  try {
    render(await invoke<Settings>("update_settings", { patch }));
  } catch (e) {
    console.warn(e);
  }
}

function render(s: Settings) {
  current = s;
  for (const key of Object.keys(ranges) as NumKey[]) {
    const input = $<HTMLInputElement>(key);
    if (document.activeElement !== input) input.value = String(s[key]);
    $(`${key}-out`).textContent = ranges[key](s[key]);
  }
  for (const key of checks) $<HTMLInputElement>(key).checked = s[key];
  for (const name of radios) for (const input of radioInputs(name)) input.checked = input.value === s[name];
  $<HTMLSelectElement>("autoCloseMinutes").value = String(s.autoCloseMinutes);
  $<HTMLSelectElement>("monitor").value = s.monitor ?? "";
}

async function loadMonitors() {
  const sel = $<HTMLSelectElement>("monitor");
  sel.replaceChildren(new Option("Primary display", ""));
  try {
    const list = await invoke<MonitorInfo[]>("list_monitors");
    for (const m of list) sel.add(new Option(m.label + (m.primary ? " · primary" : ""), m.id));
  } catch (e) {
    console.warn(e);
  }
  if (current) sel.value = current.monitor ?? "";
}

async function refreshCacheInfo() {
  try {
    const n = await invoke<number>("lyrics_cache_count");
    $("cache-info").textContent = `· ${n} ${n === 1 ? "song" : "songs"}`;
  } catch {
    /* ignore */
  }
}

function wire() {
  for (const key of Object.keys(ranges) as NumKey[]) {
    const input = $<HTMLInputElement>(key);
    input.addEventListener("input", () => {
      const v = Number(input.value);
      $(`${key}-out`).textContent = ranges[key](v);
      save({ [key]: v } as Partial<Settings>);
    });
  }
  for (const key of checks) {
    const input = $<HTMLInputElement>(key);
    input.addEventListener("change", () => save({ [key]: input.checked } as Partial<Settings>));
  }
  for (const name of radios) {
    for (const input of radioInputs(name)) {
      input.addEventListener("change", () => {
        if (input.checked) save({ [name]: input.value } as Partial<Settings>);
      });
    }
  }
  const autoClose = $<HTMLSelectElement>("autoCloseMinutes");
  autoClose.addEventListener("change", () => save({ autoCloseMinutes: Number(autoClose.value) }));
  const monitor = $<HTMLSelectElement>("monitor");
  monitor.addEventListener("change", () => save({ monitor: monitor.value || null }));

  $("clear-cache").addEventListener("click", async () => {
    const btn = $<HTMLButtonElement>("clear-cache");
    btn.disabled = true;
    try {
      const n = await invoke<number>("clear_lyrics_cache");
      btn.textContent = `Cleared ${n}`;
      setTimeout(() => (btn.textContent = "Clear lyrics cache"), 2000);
    } finally {
      btn.disabled = false;
      refreshCacheInfo();
    }
  });

  $("lrclib").addEventListener("click", (e) => {
    e.preventDefault();
    openUrl("https://lrclib.net").catch(console.warn);
  });

  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape") getCurrentWindow().close();
  });
}

async function boot() {
  wire();
  await listen<Settings>("settings", (e) => render(e.payload));
  render(await invoke<Settings>("get_settings"));
  await loadMonitors();
  refreshCacheInfo();
  try {
    $("override-dir").textContent = await invoke<string>("lyrics_override_dir");
  } catch {
    /* ignore */
  }
  getVersion().then((v) => ($("version").textContent = `v${v}`)).catch(() => {});
}

boot();
