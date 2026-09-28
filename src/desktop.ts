/** About and login-item controls mount only in the main window. OS state is
 * refreshed on focus; no background polling and no duplicate persisted flag. */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./desktop.css";

type DesktopStatus = { version: string; launchOnLogin: boolean; startupAvailable: boolean; startupMessage: string; mainVisible: boolean };
const logo = new URL("./assets/nobody-icon.png", import.meta.url).href;

/** Reuse the approved local icon and existing dialog style. External links
 * are static; native navigation goes through a bounded opener command. */
export function mountDesktop(container: HTMLElement, aboutButton: HTMLButtonElement, onVisibility: (visible: boolean) => void) {
  container.innerHTML = `<div class="desktop-setting-row"><div><h2>登录时启动</h2><p class="field-help startup-status">正在读取系统设置…</p></div><label class="desktop-switch"><input id="launch-on-login" type="checkbox" aria-label="登录系统时在后台启动 Nobody" disabled /><span aria-hidden="true"></span></label></div>`;
  const dialog = document.createElement("dialog");
  dialog.className = "update-dialog about-dialog";
  dialog.setAttribute("aria-labelledby", "about-title");
  dialog.innerHTML = `<div class="update-heading"><img class="about-logo" src="${logo}" width="64" height="64" alt="Nobody logo" /><div><span class="eyebrow">语言不同，价值不减</span><h2 id="about-title">Nobody</h2><p class="desktop-version">正在读取版本…</p></div><button class="update-close" type="button" aria-label="关闭关于">×</button></div><p class="about-description">面向开发者的翻译与局域网互传小工具。</p><div class="about-links"><a href="https://github.com/t0ken-ai/Nobody" target="_blank" rel="noreferrer" data-destination="github"><span>开源项目</span><strong>GitHub ↗</strong></a><a href="mailto:spridu@gmail.com" data-destination="email"><span>联系作者</span><strong>spridu@gmail.com ↗</strong></a></div><p class="field-help about-license">MIT License · 感谢每一位试用和反馈的人。</p><p class="field-help about-error" role="status"></p>`;
  document.body.append(dialog);
  const open = () => { if (!dialog.open) dialog.showModal(); };
  aboutButton.onclick = open;
  dialog.querySelector<HTMLButtonElement>(".update-close")!.onclick = () => dialog.close();
  for (const link of Array.from(dialog.querySelectorAll<HTMLAnchorElement>(".about-links a"))) {
    link.onclick = event => {
      if (!isTauri()) return;
      event.preventDefault();
      void invoke("open_about_link", { destination: link.dataset.destination }).catch(error => {
        dialog.querySelector(".about-error")!.textContent = String(error);
      });
    };
  }
  const checkbox = container.querySelector<HTMLInputElement>("#launch-on-login")!;
  const message = container.querySelector<HTMLElement>(".startup-status")!;
  let busy = false;
  let latest: DesktopStatus | undefined;
  const render = (state: DesktopStatus) => {
    latest = state;
    checkbox.checked = state.launchOnLogin;
    checkbox.disabled = busy || (!state.startupAvailable && !state.launchOnLogin);
    message.textContent = state.startupMessage;
    message.classList.remove("error");
    dialog.querySelector(".desktop-version")!.textContent = `版本 ${state.version}`;
    aboutButton.textContent = `Nobody ${state.version} · 关于`;
    onVisibility(state.mainVisible);
  };
  const refresh = async () => {
    if (!isTauri() || busy) return;
    try { render(await invoke<DesktopStatus>("get_desktop_status")); }
    catch (error) { message.textContent = String(error); message.classList.add("error"); }
  };
  checkbox.onchange = async () => {
    const enabled = checkbox.checked;
    busy = true; checkbox.disabled = true;
    try { render(await invoke<DesktopStatus>("set_launch_on_login", { enabled })); }
    catch (error) {
      checkbox.checked = latest?.launchOnLogin ?? false;
      message.textContent = String(error); message.classList.add("error");
    } finally {
      busy = false;
      checkbox.disabled = !latest || (!latest.startupAvailable && !latest.launchOnLogin);
    }
  };
  if (isTauri()) {
    // Subscribe before consuming a pending menu request. Repeated events are
    // harmless and a click before webview initialization opens exactly once.
    const consume = async () => { if (await invoke<boolean>("take_about_menu_request")) open(); };
    void (async () => { await listen("open-about", () => { void consume(); }); await consume(); await refresh(); })()
      .catch(error => { message.textContent = String(error); });
    window.addEventListener("focus", () => { void refresh(); });
  } else {
    message.textContent = "桌面应用中可开启，默认关闭。";
    dialog.querySelector(".desktop-version")!.textContent = "桌面版显示实际安装版本";
  }
}
