// WebKitGTK's own clipboard path (manual Ctrl+C / context-menu copy on
// selected text inside the webview) writes through GDK and logs a cosmetic
// "Gdk-WARNING: Error writing selection data: ... broken pipe" to stderr on
// Wayland/X11 when the compositor-side transfer pipe closes. Route those
// copies through the Rust clipboard plugin (wl-clipboard-rs) instead; fall
// back to the native path if the plugin write fails so a copy is never lost.
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

let nativeCopy = false;

function selectedText(): string {
  const selection = window.getSelection()?.toString();
  if (selection) return selection;
  const el = document.activeElement;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    const start = el.selectionStart;
    const end = el.selectionEnd;
    if (start !== null && end !== null && end > start) {
      return el.value.slice(start, end);
    }
  }
  return "";
}

export function installNativeCopyInterceptor(): void {
  document.addEventListener("copy", (event) => {
    if (nativeCopy || event.defaultPrevented) return;
    const text = selectedText();
    if (!text) return; // nothing selected: keep the native (no-op) path
    event.preventDefault();
    writeText(text).catch(() => {
      // Plugin unavailable (e.g. compositor without wlr-data-control):
      // retry once through the native path so the copy still happens.
      nativeCopy = true;
      try {
        document.execCommand("copy");
      } catch {
        /* ignore */
      } finally {
        nativeCopy = false;
      }
    });
  });
}
