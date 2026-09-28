// In-page confirm/prompt dialogs.
//
// Why not window.confirm()/window.prompt(): on macOS the webview is WKWebView,
// and WebKit only runs the native JS panels when the host app's WKUIDelegate
// implements the matching `webView:runJavaScript…Panel:…` selectors. wry
// (Tauri's webview layer) installs a delegate without them, so WebKit silently
// applies its defaults — confirm() → false, prompt() → null, alert() → no-op —
// which made every confirm-guarded delete and both group-name prompts a no-op
// in the macOS dmg, while Linux (WebKitGTK, which does implement script
// dialogs) behaved fine. Rendering the dialogs inside the page works the same
// on every backend and needs no extra Tauri plugin (the official dialog plugin
// has no text-input prompt anyway).
//
// Both helpers resolve after the user acts; the native calls were synchronous,
// so call sites must await them.
import "./dialog.css";
import { locale } from "./i18n";

const LABELS = {
  zh: { ok: "确定", cancel: "取消" },
  en: { ok: "OK", cancel: "Cancel" },
} as const;

/** Resolves true when confirmed; false on cancel, Escape or backdrop click. */
export function confirmDialog(message: string): Promise<boolean> {
  return openDialog(message, null).then((v) => v !== null);
}

/**
 * Resolves the entered text, or null when cancelled. The initial value comes
 * up preselected so renaming can be typed over directly.
 */
export function promptDialog(message: string, initial = ""): Promise<string | null> {
  return openDialog(message, initial);
}

/** Shared implementation: `initial === null` renders a confirm box. */
function openDialog(message: string, initial: string | null): Promise<string | null> {
  const t = LABELS[locale.value] ?? LABELS.zh;
  // Plain executor rather than Promise.withResolvers(): the project targets
  // es2022 (tsconfig + vite build.target) and this code has to run on older
  // WKWebView/WebKitGTK, where the ES2024 API is missing.
  return new Promise((resolve) => {
    const prevFocus = document.activeElement as HTMLElement | null;

    const mask = document.createElement("div");
    mask.className = "nekos-dialog-mask";

    const box = document.createElement("div");
    box.className = "nekos-dialog";
    box.setAttribute("role", "dialog");
    box.setAttribute("aria-modal", "true");

    const msg = document.createElement("div");
    msg.className = "nekos-dialog-msg";
    msg.textContent = message;
    box.appendChild(msg);

    let input: HTMLInputElement | null = null;
    if (initial !== null) {
      input = document.createElement("input");
      input.className = "nekos-dialog-input";
      input.value = initial;
      box.appendChild(input);
    }

    const btns = document.createElement("div");
    btns.className = "nekos-dialog-btns";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.className = "nekos-dialog-btn";
    cancel.textContent = t.cancel;
    const ok = document.createElement("button");
    ok.type = "button";
    ok.className = "nekos-dialog-btn nekos-dialog-ok";
    ok.textContent = t.ok;
    btns.append(cancel, ok);
    box.appendChild(btns);
    mask.appendChild(box);

    const finish = (value: string | null) => {
      document.removeEventListener("keydown", onKey, true);
      mask.remove();
      prevFocus?.focus?.();
      resolve(value);
    };

    function onKey(e: KeyboardEvent) {
      if (e.isComposing) return; // IME candidate Enter/Escape must not close
      if (e.key === "Escape") {
        e.preventDefault();
        finish(null);
        return;
      }
      if (e.key !== "Enter") return;
      // Enter confirms from the field or the default button; on Cancel it is
      // left to the button's own activation.
      if (e.target !== ok && (input === null || e.target !== input)) return;
      e.preventDefault();
      finish(input ? input.value : "");
    }

    if (input) {
      const field = input;
      // An empty name has nothing to submit, and closing silently would look
      // like the broken no-op this module replaces.
      const syncOk = () => {
        ok.disabled = !field.value.trim();
      };
      field.addEventListener("input", syncOk);
      syncOk();
    }
    cancel.addEventListener("click", () => finish(null));
    ok.addEventListener("click", () => finish(input ? input.value : ""));
    mask.addEventListener("click", (e) => {
      if (e.target === mask) finish(null);
    });
    document.addEventListener("keydown", onKey, true);

    document.body.appendChild(mask);
    (input ?? ok).focus();
    if (input) input.select();
  });
}
