/** Shared visual vocabulary for settings and tutorials. Native platform status
 * is authoritative; navigator is only a fallback for the standalone preview. */
export type DesktopPlatform = "macOS" | "Windows";
export function desktopPlatform(native?: string): DesktopPlatform {
  if (native === "macOS" || native === "Windows") return native;
  return navigator.platform.includes("Mac") ? "macOS" : "Windows";
}

// Local paths avoid emoji/font fallback and never depend on user-supplied HTML.
const symbols: Record<string, string> = {
  command: '<path d="M8 8V5a3 3 0 1 0-3 3h14a3 3 0 1 0-3-3v14a3 3 0 1 0 3-3H5a3 3 0 1 0 3 3V8Z"/>',
  shift: '<path d="m12 3 8 8h-5v9H9v-9H4z"/>',
  option: '<path d="M3 5h6l6 14h6M15 5h6"/>',
  control: '<path d="m5 15 7-7 7 7"/>',
  windows: '<path class="key-solid" d="M3 3h8v8H3zm10 0h8v8h-8zM3 13h8v8H3zm10 0h8v8h-8z"/>',
  return: '<path d="M20 5v9H4m5-5-5 5 5 5"/>',
  tab: '<path d="M4 12h15m-5-5 5 5-5 5M20 5v14"/>',
  backspace: '<path d="M9 5h12v14H9l-7-7zM12 9l5 6m0-6-5 6"/>',
  up: '<path d="M12 20V4m-6 6 6-6 6 6"/>',
  down: '<path d="M12 4v16m-6-6 6 6 6-6"/>',
  left: '<path d="M20 12H4m6-6-6 6 6 6"/>',
  right: '<path d="M4 12h16m-6-6 6 6-6 6"/>',
};
type KeyFace = { label: string; text?: string; symbol?: string };

/** Map actual modifier semantics, not just spelling: CmdOrCtrl is Ctrl on
 * Windows, whereas Super is the Windows key. Never depict Ctrl as a Win key. */
function face(key: string, platform: DesktopPlatform): KeyFace {
  const mac = platform === "macOS";
  const name = key.toLowerCase();
  if (["commandorcontrol", "cmdorctrl"].includes(name)) return mac
    ? { label: "Command", symbol: "command" } : { label: "Ctrl", text: "Ctrl" };
  if (["super", "command", "cmd", "meta"].includes(name)) return mac
    ? { label: "Command", symbol: "command" } : { label: "Windows", symbol: "windows" };
  if (["control", "ctrl"].includes(name)) return mac
    ? { label: "Control", symbol: "control" } : { label: "Ctrl", text: "Ctrl" };
  if (["alt", "option"].includes(name)) return mac
    ? { label: "Option", symbol: "option" } : { label: "Alt", text: "Alt" };
  if (name === "shift") return { label: "Shift", symbol: "shift" };
  const special: Record<string, string> = { enter: "return", return: "return", tab: "tab", backspace: "backspace", arrowup: "up", arrowdown: "down", arrowleft: "left", arrowright: "right" };
  if (special[name]) return { label: key, symbol: special[name] };
  const text = ({ space: "Space", escape: "Esc", delete: "Del" } as Record<string, string>)[name] ?? key;
  return { label: text, text };
}

/** Render safe DOM keycaps at either settings/inline or animated-deck size.
 * Preserve the original chord elsewhere for native registration and persistence. */
export function renderShortcut(container: HTMLElement, chord: string, platform: DesktopPlatform, large = false): string {
  const keys = chord.split("+").map(key => key.trim()).filter(Boolean).map(key => face(key, platform));
  const label = keys.map(key => key.label).join(" + ");
  container.replaceChildren(...keys.map((key) => {
    const cap = document.createElement("kbd");
    cap.className = large ? "shortcut-cap demo-key" : "shortcut-cap";
    cap.title = key.label;
    cap.setAttribute("aria-hidden", "true");
    if (key.symbol) {
      cap.dataset.symbol = key.symbol;
      cap.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true">${symbols[key.symbol]}</svg>`;
    } else { cap.textContent = key.text!; }
    return cap;
  }));
  container.setAttribute("aria-label", label);
  container.dataset.platform = platform;
  return label;
}

/** A focused recorder captures a chord without exposing raw config syntax.
 * Blur/Escape cancel an unfinished recording. Only a complete modified key
 * updates the hidden value; settings still require the explicit Save action. */
export function mountShortcutRecorder(button: HTMLButtonElement, input: HTMLInputElement, platform: () => DesktopPlatform, name: string) {
  let recording = false;
  const paint = () => {
    const label = renderShortcut(button, input.value, platform());
    button.setAttribute("aria-label", `${name}：${label}。点击后按新的组合键`);
    button.title = `${name}：${label} · 点击修改`;
    button.classList.remove("is-recording");
  };
  button.onclick = () => {
    // WKWebView on macOS does not always focus a button on pointer/AX activation.
    // Own keyboard focus before arming, or the chord/Escape goes to the page.
    button.focus({ preventScroll: true });
    recording = true;
    button.classList.add("is-recording");
    button.textContent = "按下组合键…";
    button.setAttribute("aria-label", `${name}：正在录入，Esc 取消`);
  };
  button.onblur = () => { recording = false; paint(); };
  button.onkeydown = event => {
    if (!recording || event.isComposing) return;
    if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey) {
      event.preventDefault(); recording = false; paint(); return;
    }
    // Leave Tab navigation available; modifier-only and unmodified typing must
    // not turn into shortcuts that intercept normal typing in other apps.
    if (!(event.metaKey || event.ctrlKey || event.altKey) || ["Meta", "Control", "Alt", "Shift"].includes(event.key)) return;
    event.preventDefault();
    if (event.repeat) return;
    const key = event.code.replace(/^Key|^Digit/, "");
    if (!key) return;
    input.value = [event.metaKey ? "Super" : "", event.ctrlKey ? "Control" : "", event.altKey ? "Alt" : "", event.shiftKey ? "Shift" : "", key].filter(Boolean).join("+");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    recording = false;
    paint();
  };
  paint();
  return { refresh: paint };
}
