/** Update UI subscribes only once in the main window. Native code owns timers,
 * trusted package URLs, version decisions and installation; this view displays
 * sanitized release notes and explicit user choices without background polling. */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { renderTranslation } from "./markdown";
import "./updates.css";

export type UpdateSnapshot = {
  revision: number; currentVersion: string; version: string | null; notes: string;
  date: string; phase: string; message: string; downloaded: number; total: number | null;
  prompt: boolean; automatic: boolean;
};

/** Static controls contain no remote markup. Release text is rendered below
 * through the same passive Markdown boundary as translated documents. */
export function updateDialogMarkup() {
  return `<dialog class="update-dialog" aria-labelledby="update-title">
    <div class="update-heading"><span class="update-brand nobody-symbol" aria-hidden="true"></span><div><span class="eyebrow">NOBODY · UPDATES</span><h2 id="update-title">检查更新</h2></div><button type="button" class="update-close" aria-label="关闭更新提醒">×</button></div>
    <p class="update-version"></p><div class="update-notes" tabindex="0" aria-label="版本更新摘要"></div>
    <div class="update-progress" hidden><progress aria-label="下载进度"></progress><span></span></div>
    <p class="update-status" role="status" aria-live="polite"></p>
    <footer><button type="button" class="text-button update-skip">跳过此版本</button><div><button type="button" class="button update-later">稍后</button><button type="button" class="button primary update-action">立即更新</button></div></footer>
  </dialog>`;
}

/** Exported separately for deterministic visual fixtures. Text never becomes
 * an attribute, URL or handler; only known phases affect action availability. */
export function renderUpdateDialog(dialog: HTMLDialogElement, state: UpdateSnapshot) {
  const find = <T extends HTMLElement>(selector: string) => dialog.querySelector<T>(selector)!;
  const active = ["checking", "downloading", "installing"].includes(state.phase);
  const applying = ["downloading", "installing"].includes(state.phase);
  const available = Boolean(state.version) && ["available", "error"].includes(state.phase);
  find("#update-title").textContent = applying ? "正在更新 Nobody" : state.version ? `Nobody ${state.version}` : state.phase === "current" ? "已是最新版本" : "检查更新";
  find(".update-version").textContent = `当前版本 ${state.currentVersion}${state.date ? ` · 发布于 ${state.date}` : ""}`;
  const notes = find(".update-notes");
  // Progress events do not reparse Markdown or reset a reader's scroll position.
  if (notes.dataset.version !== (state.version ?? "") || notes.dataset.notes !== state.notes) {
    renderTranslation(notes, state.notes, true);
    notes.dataset.version = state.version ?? ""; notes.dataset.notes = state.notes;
  }
  notes.hidden = !state.notes;
  find(".update-status").textContent = state.message;
  find(".update-status").classList.toggle("error", state.phase === "error");
  find<HTMLButtonElement>(".update-skip").hidden = !available;
  find<HTMLButtonElement>(".update-skip").disabled = active;
  find<HTMLButtonElement>(".update-later").disabled = applying;
  find<HTMLButtonElement>(".update-later").textContent = state.phase === "current" ? "完成" : "稍后";
  find<HTMLButtonElement>(".update-close").disabled = applying;
  const action = find<HTMLButtonElement>(".update-action");
  action.hidden = state.phase === "current";
  action.disabled = active;
  action.textContent = state.phase === "downloading" ? "正在下载…" : state.phase === "installing" ? "正在安装…" : available ? "更新并重启" : "重新检查";
  find(".update-progress").hidden = !applying;
  const progress = find<HTMLProgressElement>("progress");
  if (state.total && state.total > 0) {
    progress.max = state.total; progress.value = Math.min(state.downloaded, state.total);
  } else { progress.removeAttribute("value"); }
  const mib = (n: number) => `${(n / 1024 / 1024).toFixed(1)} MB`;
  find(".update-progress span").textContent = state.phase === "installing" ? "安装后自动重启" : `${mib(state.downloaded)}${state.total ? ` / ${mib(state.total)}` : ""}`;
}

/** Mount on the main settings surface only. Subscribe before fetching state,
 * then reject older revisions so fast menu clicks/startup checks cannot vanish. */
export function mountUpdates(container: HTMLElement) {
  container.innerHTML = `<div class="update-settings-heading"><div><h2>Nobody 更新</h2><p class="field-help update-current">正在读取版本…</p></div><button type="button" class="button small update-check">检查更新</button></div><label class="checkbox-row"><input class="update-automatic" type="checkbox" checked />自动检查更新</label><p class="field-help update-settings-status">启动后检查，每 6 小时检查一次。</p>`;
  const wrapper = document.createElement("div"); wrapper.innerHTML = updateDialogMarkup();
  const dialog = wrapper.firstElementChild as HTMLDialogElement;
  document.body.append(dialog);
  let latest: UpdateSnapshot | undefined;
  let pending = false;
  const automatic = container.querySelector<HTMLInputElement>(".update-automatic")!;
  const checkButton = container.querySelector<HTMLButtonElement>(".update-check")!;
  const errorMessage = (error: unknown) => {
    const node = dialog.open ? dialog.querySelector<HTMLElement>(".update-status")! : container.querySelector<HTMLElement>(".update-settings-status")!;
    node.textContent = String(error); node.classList.add("error");
  };
  const receive = (state: UpdateSnapshot) => {
    if (latest && state.revision < latest.revision) return;
    latest = state;
    container.querySelector(".update-current")!.textContent = `当前版本 ${state.currentVersion}`;
    document.querySelectorAll(".app-version").forEach(node => { node.textContent = `Nobody ${state.currentVersion}`; });
    automatic.checked = state.automatic;
    const active = ["checking", "downloading", "installing"].includes(state.phase);
    checkButton.disabled = active;
    const settingsStatus = container.querySelector<HTMLElement>(".update-settings-status")!;
    settingsStatus.textContent = state.message;
    settingsStatus.classList.toggle("error", state.phase === "error");
    renderUpdateDialog(dialog, state);
    if (state.prompt && !dialog.open) dialog.showModal();
    else if (!state.prompt && dialog.open) dialog.close();
  };
  const run = async (command: string, args?: Record<string, unknown>) => {
    if (pending) return;
    pending = true;
    try { await invoke(command, args); } catch (error) { errorMessage(error); }
    finally { pending = false; }
  };
  checkButton.onclick = () => { void run("check_for_updates"); };
  // A check can take up to twenty seconds. Dismiss remains available while its
  // promise is pending; the native result respects that dismissal when it lands.
  const dismiss = async (skip = false) => {
    try { await invoke("dismiss_update", { version: latest?.version ?? null, skip }); }
    catch (error) { errorMessage(error); }
  };
  dialog.querySelector<HTMLButtonElement>(".update-close")!.onclick = () => { void dismiss(); };
  dialog.querySelector<HTMLButtonElement>(".update-later")!.onclick = () => { void dismiss(); };
  dialog.querySelector<HTMLButtonElement>(".update-skip")!.onclick = () => { void dismiss(true); };
  dialog.addEventListener("cancel", event => { event.preventDefault(); void dismiss(); });
  dialog.querySelector<HTMLButtonElement>(".update-action")!.onclick = () => {
    if (latest?.version) void run("install_update", { version: latest.version });
    else void run("check_for_updates");
  };
  automatic.onchange = () => {
    const enabled = automatic.checked;
    automatic.disabled = true;
    void invoke("set_automatic_updates", { enabled }).catch(error => {
      automatic.checked = latest?.automatic ?? true; errorMessage(error);
    }).finally(() => { automatic.disabled = false; });
  };
  if (isTauri()) {
    void (async () => {
      await listen<UpdateSnapshot>("update-state", event => receive(event.payload));
      receive(await invoke<UpdateSnapshot>("get_update_state"));
    })().catch(errorMessage);
  } else {
    checkButton.disabled = true; automatic.disabled = true;
    container.querySelector(".update-current")!.textContent = "桌面应用支持自动更新";
    container.querySelector(".update-settings-status")!.textContent = "浏览器预览不执行下载或安装。";
  }
}
