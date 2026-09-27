import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./style.css";
import { demoMarkup, mountDemos } from "./demos";

type Settings = {
  engine: "system" | "llm";
  targetLanguage: string;
  autoSelection: boolean;
  automaticApps: string[];
  writeShortcut: string;
  readShortcut: string;
  endpoint: string;
  model: string;
};
type Status = {
  accessibility: boolean;
  systemTranslation: boolean;
  platform: string;
  settings: Settings;
};
type Result = {
  source: string;
  text: string;
  target: string;
  engine: string;
  origin: string;
  message: string;
  replaced: boolean;
};
const languages = [
  ["zh-Hans", "简体中文"],
  ["zh-Hant", "繁體中文"],
  ["en", "English"],
  ["ja", "日本語"],
  ["ko", "한국어"],
  ["fr", "Français"],
  ["de", "Deutsch"],
  ["es", "Español"],
  ["pt", "Português"],
  ["ru", "Русский"],
  ["it", "Italiano"],
  ["ar", "العربية"],
  ["hi", "हिन्दी"],
  ["vi", "Tiếng Việt"],
  ["th", "ไทย"],
  ["id", "Indonesia"],
  ["tr", "Türkçe"],
  ["uk", "Українська"],
];
const languageOptions = languages
  .map(([code, name]) => `<option value="${code}">${name}</option>`)
  .join("");
const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;
const icon = (name: string) => {
  const paths: Record<string, string> = {
    translate:
      '<path d="m4 5 11 0M9 3v2m4 0c-1 6-4 9-9 11m2-8c2 4 4 6 8 8m0 4 4-10 4 10m-6-3h4"/>',
    settings:
      '<path d="M4 7h16M4 17h16"/><circle cx="8" cy="7" r="3"/><circle cx="16" cy="17" r="3"/>',
    arrow: '<path d="M5 12h14m-5-5 5 5-5 5"/>',
    copy: '<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M15 8V3H3v13h5"/>',
    check: '<path d="m5 12 4 4L19 6"/>',
    close: '<path d="m6 6 12 12M18 6 6 18"/>',
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name] ?? paths.translate}</svg>`;
};

const popup = new URLSearchParams(location.search).get("view") === "result";
let status: Status | undefined;
let latest: Result | undefined;
let busy = false;
let demos: ReturnType<typeof mountDemos> | undefined;

/** Measure content, not the current window height, to avoid a resize feedback
 * loop. Native placement limits the final height to the available screen space. */
function resizePopup() {
  if (!popup || !isTauri()) return;
  requestAnimationFrame(() => {
    const header = document.querySelector<HTMLElement>(".popup-header")!;
    const footer = document.querySelector<HTMLElement>(".popup-footer")!;
    const content = $("popup-content");
    const message = $("notice");
    const height = header.offsetHeight + footer.offsetHeight + content.scrollHeight
      + (message.hidden ? 0 : message.offsetHeight + 10) + 26;
    void call("resize_popover", { height }).catch(() => {});
  });
}

// Browser preview is intentionally nonfunctional: a missing desktop bridge must
// never be mistaken for working translation or silently use fabricated results.
async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isTauri())
    throw new Error(
      "当前为界面预览。请运行 TranslateMe 桌面应用以使用翻译和系统取词。",
    );
  return invoke<T>(command, args);
}
function notice(message: string, error = false) {
  const node = $("notice");
  node.textContent = message;
  node.className = `notice ${error ? "error" : ""}`;
  node.hidden = !message;
  if (popup) resizePopup();
}
function label(code: string) {
  return languages.find(([value]) => value === code)?.[1] ?? code;
}

if (popup) {
  document.body.classList.add("popup");
  document.documentElement.classList.add("popup-root");
  $("app").innerHTML =
    `<header class="popup-header" title="拖动顶部，移动译文"><div class="mini-brand">${icon("translate")} <strong id="popup-language">译文</strong></div><span class="drag-grip" aria-hidden="true">⠿</span><button id="hide" class="icon-button" aria-label="关闭译文" title="关闭 · Esc">${icon("close")}</button></header>
    <div id="notice" class="notice" role="status" hidden></div><main class="popup-body"><div id="popup-content"><div id="popup-text" class="popup-text">选中文字，即可在附近阅读译文。</div></div></main>
    <footer class="popup-footer"><span id="popup-origin">TranslateMe</span><span id="popup-message" class="sr-only">原文保持不变</span><button id="popup-copy" class="popup-copy" disabled title="复制译文">${icon("copy")}复制</button></footer>`;
  $("hide").onclick = () => {
    void call("dismiss_popover");
  };
  // Only the header initiates a native drag. Text remains selectable and the
  // close/copy buttons never turn into drag targets or send input to the source.
  document.querySelector<HTMLElement>(".popup-header")!.onpointerdown = (event) => {
    if (event.button !== 0 || (event.target as Element).closest("button")) return;
    event.preventDefault();
    void call("drag_popover");
  };
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") void call("dismiss_popover");
  });
  $("popup-copy").onclick = () => {
    if (latest) void copy(latest.text);
  };
  new ResizeObserver(resizePopup).observe($("popup-content"));
} else {
  $("app").innerHTML = `
    <main class="workspace">
      <header class="topbar"><div class="brand"><span class="brand-symbol">${icon("translate")}</span><span>TranslateMe<span class="brand-divider">/</span><small>语言之间，思路不断</small></span></div><nav aria-label="主导航"><button id="nav-demos" class="nav-item active" aria-current="page">使用演示</button><button id="nav-workbench" class="nav-item">翻译工作台</button><button id="nav-settings" class="nav-item" aria-label="偏好设置">${icon("settings")}</button></nav></header>
      <div class="page-content">
        <section id="demos">
          <div class="heading-row"><div><div class="eyebrow"><span></span> THINK IN YOUR LANGUAGE</div><h1>想法，不必绕远路<span>。</span></h1><p class="subtitle">写下你想说的，读懂你想看的。</p></div><span class="demo-badge"><span></span>交互演示</span></div>
          <div class="demo-grid">${demoMarkup()}</div>
          <div class="demo-footnote"><span>固定样例 · 自动划词默认仅限 Codex / Claude Desktop</span><button id="try-workbench" class="text-button">去翻译一段 ${icon("arrow")}</button></div>
        </section>
        <section id="workbench" hidden>
          <div class="heading-row"><div><div class="eyebrow">A SPACE FOR YOUR WORDS</div><h1>让想法，跨过语言。</h1><p class="subtitle">粘贴一段文字，在这里完成翻译。</p></div></div>
          <div class="editor-toolbar"><div><span class="section-title">试译一段</span><button id="sample" class="text-button">使用当前聊天示例 ↗</button></div><span id="engine-badge" class="engine-badge">Apple 系统翻译</span></div>
          <div class="translation-grid">
            <section class="editor-pane"><header><span>自动识别语言</span><span class="muted">原文</span></header><textarea id="source" aria-label="原文" spellcheck="false" placeholder="在这里输入，或直接在其他应用中使用快捷键…"></textarea><footer><span id="count">0 / 16,000</span><button id="clear" class="text-button">清空</button></footer></section>
            <section class="editor-pane output-pane"><header><select id="target" aria-label="翻译目标语言">${languageOptions}</select><span class="muted">译文</span></header><textarea id="output" aria-label="译文" readonly placeholder="译文会显示在这里。"></textarea><footer><span id="result-meta">代码片段原样保留</span><button id="copy" class="text-button" disabled>${icon("copy")}复制译文</button></footer></section>
          </div>
          <div class="action-row"><span class="privacy-note" id="engine-note">系统翻译在本机处理；首次使用可能需要下载语言包。</span><button id="translate" class="button primary">翻译成英文 ${icon("arrow")}</button></div>
        </section>
        <section id="settings" hidden>
          <div class="eyebrow">MAKE IT YOURS</div><h1>让它，更合你的习惯。</h1><p class="subtitle">翻译引擎、阅读语言和触发方式，都在这里。</p>
          <form id="settings-form">
            <div class="setting-card"><h2>翻译引擎</h2><div class="engine-choices"><label><input type="radio" name="engine" value="system" checked /><span><strong>系统翻译</strong><small>macOS · 本机处理 · 无需 API Key</small></span></label><label><input type="radio" name="engine" value="llm" /><span><strong>自定义 LLM</strong><small>macOS / Windows · 兼容 Chat Completions 接口</small></span></label></div>
            <p id="system-help" class="field-help">系统会在需要时提示下载语言包。</p>
            <div id="llm-fields" hidden><label class="field">API 地址<input id="endpoint" type="url" placeholder="https://your-provider.com/v1" autocomplete="off" /></label><div class="field-row"><label class="field">模型名称<input id="model" placeholder="服务商提供的模型 ID" autocomplete="off" /></label><label class="field">API Key<input id="api-key" type="password" placeholder="留空保留已保存的密钥" autocomplete="new-password" /></label></div><label class="checkbox-row"><input id="delete-key" type="checkbox" />删除当前服务已保存的密钥</label><p class="field-help">密钥保存在系统凭据库。文字只发送到你配置的服务；本机 LLM 可使用 localhost 地址。</p></div></div>
            <div class="setting-card"><h2>阅读与快捷键</h2><div class="field-row"><label class="field">阅读目标语言<select id="reading-language">${languageOptions}</select></label><label class="field">划词自动翻译<span class="switch-row"><input id="auto-selection" type="checkbox" /><span>仅在白名单应用中触发</span></span></label></div><div class="allowlist-field"><span>自动划词白名单</span><div><label><input id="allow-codex" type="checkbox" />Codex</label><label><input id="allow-claude" type="checkbox" />Claude Desktop</label></div><p class="field-help">只在所选应用的阅读正文中自动翻译；输入框和文件对话框忽略。手动快捷键不受白名单限制。</p></div><div class="field-row"><label class="field">写入英文快捷键<input id="write-key" aria-label="写入英文快捷键" /></label><label class="field">阅读翻译快捷键<input id="read-key" aria-label="阅读翻译快捷键" /></label></div><p class="field-help">可直接按组合键录入。翻译只负责回填，不会替你按发送；输入变化时保留译文供复制。</p></div>
            <div class="action-row"><span class="privacy-note">不保存翻译历史</span><button class="button primary" type="submit" id="save">保存设置 ${icon("check")}</button></div>
          </form>
        </section>
        <div id="permission-banner" class="permission-banner"><span class="connection-dot" aria-hidden="true"></span><div><strong id="permission-title">正在检查系统连接…</strong><p id="permission-description">跨应用取词需要辅助功能权限。</p></div><button id="permission" class="button small">开启辅助功能</button><span class="app-version">TranslateMe 0.1</span></div>
        <div id="notice" class="notice" role="status" hidden></div>
      </div>
    </main>`;
  $<HTMLSelectElement>("target").value = "en";
  demos = mountDemos();
  demos.setWriteShortcut("CommandOrControl+Shift+E");
  /** Keep each view mounted so navigation preserves draft text and unsaved
   * settings. Only the visible tutorial may consume animation frames. */
  function navigate(view: "demos" | "workbench" | "settings") {
    for (const name of ["demos", "workbench", "settings"]) {
      $(name).hidden = name !== view;
      $(`nav-${name}`).classList.toggle("active", name === view);
      if (name === view) $(`nav-${name}`).setAttribute("aria-current", "page");
      else $(`nav-${name}`).removeAttribute("aria-current");
    }
    demos?.setVisible(view === "demos");
    notice("");
  }
  $("nav-demos").onclick = () => navigate("demos");
  $("nav-workbench").onclick = () => navigate("workbench");
  $("nav-settings").onclick = () => navigate("settings");
  $("try-workbench").onclick = () => navigate("workbench");
  const source = $<HTMLTextAreaElement>("source");
  const target = $<HTMLSelectElement>("target");
  const count = () => {
    $("count").textContent =
      `${Array.from(source.value).length.toLocaleString()} / 16,000`;
  };
  source.oninput = count;
  $("sample").onclick = () => {
    source.value =
      "我想写一个 macOS 和 Windows 都能用的翻译器。通过快捷键把输入翻译成英文，选中文字时翻译成我设置的语言。请保留代码、路径和变量名。";
    target.value = "en";
    updateTranslateLabel();
    count();
    source.focus();
  };
  $("clear").onclick = () => {
    source.value = "";
    count();
    source.focus();
  };
  function updateTranslateLabel() {
    $("translate").innerHTML =
      `翻译成${target.value === "en" ? "英文" : label(target.value)} ${icon("arrow")}`;
  }
  target.onchange = updateTranslateLabel;
  $("translate").onclick = async () => {
    if (busy) return;
    busy = true;
    $<HTMLButtonElement>("translate").disabled = true;
    $("translate").textContent = "正在翻译…";
    notice("");
    try {
      display(
        await call<Result>("translate_text", {
          text: source.value,
          target: target.value,
        }),
      );
    } catch (error) {
      notice(String(error), true);
    } finally {
      busy = false;
      $<HTMLButtonElement>("translate").disabled = false;
      updateTranslateLabel();
    }
  };
  $("copy").onclick = () => {
    if (latest) void copy(latest.text);
  };
  $("permission").onclick = async () => {
    try {
      await call("request_permission");
      notice("在系统设置中启用 TranslateMe，然后回到这里。");
    } catch (error) {
      notice(String(error), true);
    }
  };
  document
    .querySelectorAll<HTMLInputElement>("input[name=engine]")
    .forEach((input) => {
      input.onchange = () => {
        $("llm-fields").hidden = selectedEngine() !== "llm";
      };
    });
  for (const id of ["write-key", "read-key"]) {
    $(id).onkeydown = (event) => {
      if (
        !(event.metaKey || event.ctrlKey || event.altKey) ||
        ["Meta", "Control", "Alt", "Shift"].includes(event.key)
      )
        return;
      event.preventDefault();
      const modifiers = [
        event.metaKey ? "Super" : "",
        event.ctrlKey ? "Control" : "",
        event.altKey ? "Alt" : "",
        event.shiftKey ? "Shift" : "",
      ].filter(Boolean);
      const key = event.code.replace(/^Key|^Digit/, "");
      $<HTMLInputElement>(id).value = [...modifiers, key].join("+");
    };
  }
  $("settings-form").onsubmit = async (event) => {
    event.preventDefault();
    $<HTMLButtonElement>("save").disabled = true;
    const settings: Settings = {
      engine: selectedEngine(),
      targetLanguage: $<HTMLSelectElement>("reading-language").value,
      autoSelection: $<HTMLInputElement>("auto-selection").checked,
      automaticApps: ["codex", "claude"].filter(app => $<HTMLInputElement>(`allow-${app}`).checked),
      writeShortcut: $<HTMLInputElement>("write-key").value,
      readShortcut: $<HTMLInputElement>("read-key").value,
      endpoint: $<HTMLInputElement>("endpoint").value.trim(),
      model: $<HTMLInputElement>("model").value.trim(),
    };
    const key = $<HTMLInputElement>("api-key").value;
    const apiKey = $<HTMLInputElement>("delete-key").checked ? "" : key || null;
    try {
      await call("save_settings", { settings, apiKey });
      $<HTMLInputElement>("api-key").value = "";
      $<HTMLInputElement>("delete-key").checked = false;
      await refresh();
      notice("设置已保存，快捷键已生效。");
    } catch (error) {
      notice(String(error), true);
    } finally {
      $<HTMLButtonElement>("save").disabled = false;
    }
  };
}

function selectedEngine(): Settings["engine"] {
  return (document.querySelector<HTMLInputElement>("input[name=engine]:checked")
    ?.value ?? "system") as Settings["engine"];
}
async function copy(text: string) {
  try {
    await call("copy_text", { text });
    notice("译文已复制。");
  } catch (error) {
    notice(String(error), true);
  }
}
function display(result: Result) {
  latest = result;
  notice("");
  if (popup) {
    $("popup-text").textContent = result.text;
    $("popup-language").textContent = label(result.target);
    $("popup-origin").textContent =
      `${result.origin} · ${result.engine === "system" ? "系统翻译" : "LLM"}`;
    $("popup-message").textContent = result.message;
    $<HTMLButtonElement>("popup-copy").disabled = false;
    document.body.classList.remove("popup-loading");
    resizePopup();
  } else {
    $<HTMLTextAreaElement>("output").value = result.text;
    $("result-meta").textContent =
      `${label(result.target)} · ${result.engine === "system" ? "系统翻译" : "LLM"}`;
    $<HTMLButtonElement>("copy").disabled = false;
    notice(result.message);
  }
}

/** Status refresh updates permissions without overwriting unsaved form edits. */
async function refresh(initializeForm = true) {
  if (popup) return;
  try {
    status = await call<Status>("get_settings");
    const s = status.settings;
    $("permission-title").textContent = status.accessibility
      ? "跨应用翻译已就绪"
      : "再一步，连接你的工作流";
    $("permission-description").textContent = status.accessibility
      ? (s.autoSelection && s.automaticApps.length
        ? `${s.automaticApps.map(app => app === "codex" ? "Codex" : "Claude Desktop").join(" / ")} 中划选正文，松开鼠标即可。`
        : "自动划词已关闭，仍可使用手动翻译快捷键。")
      : "开启辅助功能后，才能读取选区并将英文回填到原输入框。";
    $("permission").hidden = status.accessibility;
    $("permission-banner").classList.toggle("ready", status.accessibility);
    demos?.setWriteShortcut(s.writeShortcut);
    $("engine-badge").textContent =
      s.engine === "system" ? "Apple 系统翻译" : `LLM · ${s.model}`;
    $("engine-note").textContent =
      s.engine === "system"
        ? "系统翻译在本机处理；首次使用可能需要下载语言包。"
        : "文字将发送到你配置的 LLM 服务。代码片段留在本机。";
    $("system-help").textContent = status.systemTranslation
      ? "Apple 系统翻译可用。首次使用时，系统可能提示下载语言包。"
      : `${status.platform} 当前构建无法使用系统翻译，请选择 LLM。macOS 需使用含 Translation 的 SDK 构建。`;
    if (initializeForm) {
      document.querySelector<HTMLInputElement>(
        `input[name=engine][value=${s.engine}]`,
      )!.checked = true;
      $("llm-fields").hidden = s.engine !== "llm";
      $<HTMLSelectElement>("reading-language").value = s.targetLanguage;
      $<HTMLInputElement>("auto-selection").checked = s.autoSelection;
      for (const app of ["codex", "claude"]) {
        $<HTMLInputElement>(`allow-${app}`).checked = s.automaticApps.includes(app);
      }
      $<HTMLInputElement>("write-key").value = s.writeShortcut;
      $<HTMLInputElement>("read-key").value = s.readShortcut;
      $<HTMLInputElement>("endpoint").value = s.endpoint;
      $<HTMLInputElement>("model").value = s.model;
    }
  } catch (error) {
    notice(String(error), true);
  }
}

if (isTauri()) {
  await listen<Result>("translation-result", (event) => display(event.payload));
  await listen<string>("translation-error", (event) => {
    notice(event.payload, true);
    if (popup) {
      latest = undefined;
      document.body.classList.remove("popup-loading");
      $("popup-text").textContent = "";
      $("popup-origin").textContent = "暂时无法翻译";
      $<HTMLButtonElement>("popup-copy").disabled = true;
      resizePopup();
    }
  });
  await listen<{ message: string; target?: string }>("translation-progress", (event) => {
    notice(event.payload.message);
    // A previous selection's translation must not remain actionable while a
    // new selection is being translated in the floating window.
    if (popup) {
      latest = undefined;
      document.body.classList.add("popup-loading");
      if (event.payload.target) $("popup-language").textContent = label(event.payload.target);
      $("popup-text").textContent = "";
      $("popup-origin").textContent = "TranslateMe";
      $("popup-message").textContent = "正在翻译";
      $<HTMLButtonElement>("popup-copy").disabled = true;
      resizePopup();
    }
  });
  if (popup) {
    const result = await call<Result | null>("get_last_result");
    if (result) display(result);
  } else {
    await refresh();
    window.addEventListener("focus", () => {
      void refresh(false);
    });
  }
} else {
  notice("界面预览模式 · 请启动桌面应用体验系统翻译与快捷键。");
  if (!popup) {
    $("permission-title").textContent = "桌面预览";
    $("permission-description").textContent = "动画可直接体验，真实翻译请打开桌面应用。";
    $("permission").hidden = true;
  }
}
