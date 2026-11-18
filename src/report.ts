// Sends uncaught page errors to the backend log file (see `frontend_log` in
// src-tauri/src/main.rs). Import this first so it is listening before anything
// else on the page runs.

import { invoke } from "@tauri-apps/api/core";

if ("__TAURI_INTERNALS__" in window) {
  const send = (message: string) => invoke("frontend_log", { message }).catch(() => {});

  window.addEventListener("error", (e) => {
    const where = e.filename ? ` (${e.filename.split("/").pop()}:${e.lineno}:${e.colno})` : "";
    send(`${e.message}${where}${e.error?.stack ? `\n${e.error.stack}` : ""}`);
  });

  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason;
    send(`unhandled promise rejection: ${r instanceof Error ? `${r.message}\n${r.stack ?? ""}` : String(r)}`);
  });
}
