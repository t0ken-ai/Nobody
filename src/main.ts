import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./style.css";

type Settings = {
  engine: "system" | "llm";
  targetLanguage: string;
  autoSelection: boolean;
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
  };
  return `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name] ?? paths.translate}</svg>`;
};

const popup = new URLSearchParams(location.search).get("view") === "result";
let status: Status | undefined;
let latest: Result | undefined;
let busy = false;

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
}
function label(code: string) {
  return languages.find(([value]) => value === code)?.[1] ?? code;
}
function shortcutLabel(value: string) {
  return value
    .replace(
      "CommandOrControl",
      navigator.platform.includes("Mac") ? "⌘" : "Ctrl",
    )
    .replace("Super", "⌘")
    .replace("Shift", "⇧")
    .replace("Alt", "⌥")
    .replaceAll("+", " ");
}

if (popup) {
  document.body.classList.add("popup");
  $("app").innerHTML =
    `<header class="popup-header"><div class="mini-brand">${icon("translate")} <strong>TranslateMe</strong></div><span id="popup-language">译文</span><button id="hide" class="icon-button" aria-label="关闭">×</button></header>
    <div id="notice" class="notice" hidden></div><main class="popup-body"><div id="popup-origin" class="eyebrow">选中文字，阅读译文</div><div id="popup-text" class="popup-text">按阅读快捷键，或在其他应用中选中文字。</div></main>
    <footer class="popup-footer"><span id="popup-message">原文保持不变</span><button id="popup-copy" class="button small" disabled>${icon("copy")}复制</button></footer>`;
  $("hide").onclick = () => {
    void getCurrentWindow().hide();
  };
  $("popup-copy").onclick = () => {
    if (latest) void copy(latest.text);
  };
} else {
  $("app").innerHTML = `
    <aside class="sidebar">
      <div class="brand"><span class="brand-symbol">${icon("translate")}</span><span>TranslateMe<small>为思考保留母语</small></span></div>
      <div class="nav-label">工作空间</div>
      <nav><button id="nav-workbench" class="nav-item active">${icon("translate")}翻译工作台</button><button id="nav-settings" class="nav-item">${icon("settings")}偏好设置</button></nav>
      <div class="sidebar-bottom"><span class="status-dot"></span>在你的工作流里<small>选词 · 翻译 · 继续思考</small><div class="version">TranslateMe / 0.1.0</div></div>
    </aside>
    <main class="workspace">
      <header class="topbar"><span id="breadcrumb">工作空间 <b>/</b> 翻译工作台</span><span class="local-badge">● 桌面助手</span></header>
      <div class="page-content">
        <section id="workbench">
          <div class="heading-row"><div><div class="eyebrow">LESS FRICTION. MORE FLOW.</div><h1>写英文，读母语。</h1><p class="subtitle">用熟悉的语言表达，把翻译留给一个快捷键。</p></div><span class="hero-symbol">A<span>文</span></span></div>
          <div class="shortcut-grid">
            <div class="shortcut-card"><span class="shortcut-icon">↗</span><div><strong>写入英文</strong><p>翻译选区或当前输入框</p></div><kbd id="write-shortcut">⌘ ⇧ E</kbd></div>
            <div class="shortcut-card"><span class="shortcut-icon">↙</span><div><strong>划词阅读</strong><p>选中文字，自动显示译文</p></div><kbd id="read-shortcut">⌘ ⇧ D</kbd></div>
          </div>
          <div id="permission-banner" class="permission-banner"><div><strong id="permission-title">正在检查系统连接…</strong><p id="permission-description">翻译工作台无需辅助功能权限，跨应用取词需要开启。</p></div><button id="permission" class="button small">开启辅助功能</button></div>
          <div class="editor-toolbar"><div><span class="section-title">试译一段</span><button id="sample" class="text-button">使用当前聊天示例 ↗</button></div><span id="engine-badge" class="engine-badge">Apple 系统翻译</span></div>
          <div class="translation-grid">
            <section class="editor-pane"><header><span>自动识别语言</span><span class="muted">原文</span></header><textarea id="source" aria-label="原文" spellcheck="false" placeholder="在这里输入，或直接在其他应用中使用快捷键…"></textarea><footer><span id="count">0 / 16,000</span><button id="clear" class="text-button">清空</button></footer></section>
            <section class="editor-pane output-pane"><header><select id="target" aria-label="翻译目标语言">${languageOptions}</select><span class="muted">译文</span></header><textarea id="output" aria-label="译文" readonly placeholder="让语言退到身后，让想法向前。"></textarea><footer><span id="result-meta">代码片段原样保留</span><button id="copy" class="text-button" disabled>${icon("copy")}复制译文</button></footer></section>
          </div>
          <div class="action-row"><span class="privacy-note" id="engine-note">系统翻译在本机处理；首次使用可能需要下载语言包。</span><button id="translate" class="button primary">翻译成英文 ${icon("arrow")}</button></div>
          <div class="reading-demo"><span class="eyebrow">READING MODE</span><p>You can think in your own language and keep your coding workflow in English.</p><span>在其他应用中选中类似的英文句子，即可查看母语译文。</span></div>
        </section>
        <section id="settings" hidden>
          <div class="eyebrow">MAKE IT YOURS</div><h1>偏好设置</h1><p class="subtitle">选一个翻译引擎，剩下的交给快捷键。</p>
          <form id="settings-form">
            <div class="setting-card"><h2>翻译引擎</h2><div class="engine-choices"><label><input type="radio" name="engine" value="system" checked /><span><strong>系统翻译</strong><small>macOS · 本机处理 · 无需 API Key</small></span></label><label><input type="radio" name="engine" value="llm" /><span><strong>自定义 LLM</strong><small>macOS / Windows · 兼容 Chat Completions 接口</small></span></label></div>
            <p id="system-help" class="field-help">系统会在需要时提示下载语言包。</p>
            <div id="llm-fields" hidden><label class="field">API 地址<input id="endpoint" type="url" placeholder="https://your-provider.com/v1" autocomplete="off" /></label><div class="field-row"><label class="field">模型名称<input id="model" placeholder="服务商提供的模型 ID" autocomplete="off" /></label><label class="field">API Key<input id="api-key" type="password" placeholder="留空保留已保存的密钥" autocomplete="new-password" /></label></div><label class="checkbox-row"><input id="delete-key" type="checkbox" />删除当前服务已保存的密钥</label><p class="field-help">密钥保存在系统凭据库。文字只发送到你配置的服务；本机 LLM 可使用 localhost 地址。</p></div></div>
            <div class="setting-card"><h2>阅读与快捷键</h2><div class="field-row"><label class="field">阅读目标语言<select id="reading-language">${languageOptions}</select></label><label class="field">划词自动翻译<span class="switch-row"><input id="auto-selection" type="checkbox" /><span>选区稳定后显示译文</span></span></label></div><div class="field-row"><label class="field">写入英文快捷键<input id="write-key" aria-label="写入英文快捷键" /></label><label class="field">阅读翻译快捷键<input id="read-key" aria-label="阅读翻译快捷键" /></label></div><p class="field-help">可直接按组合键录入。翻译只负责回填，不会替你按发送；输入变化时保留译文供复制。</p></div>
            <div class="action-row"><span class="privacy-note">不保存翻译历史</span><button class="button primary" type="submit" id="save">保存设置 ${icon("check")}</button></div>
          </form>
        </section>
        <div id="notice" class="notice" role="status" hidden></div>
      </div>
    </main>`;
  $<HTMLSelectElement>("target").value = "en";
  function navigate(settings: boolean) {
    $("workbench").hidden = settings;
    $("settings").hidden = !settings;
    $("nav-settings").classList.toggle("active", settings);
    $("nav-workbench").classList.toggle("active", !settings);
    $("breadcrumb").textContent =
      `工作空间 / ${settings ? "偏好设置" : "翻译工作台"}`;
    notice("");
  }
  $("nav-workbench").onclick = () => navigate(false);
  $("nav-settings").onclick = () => navigate(true);
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
      ? "回到聊天或编辑器，选中文字或按快捷键即可。"
      : "开启辅助功能后，才能读取选区并将英文回填到原输入框。";
    $("permission").hidden = status.accessibility;
    $("permission-banner").classList.toggle("ready", status.accessibility);
    $("write-shortcut").textContent = shortcutLabel(s.writeShortcut);
    $("read-shortcut").textContent = shortcutLabel(s.readShortcut);
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
      $("popup-text").textContent = "";
      $<HTMLButtonElement>("popup-copy").disabled = true;
    }
  });
  await listen<{ message: string }>("translation-progress", (event) => {
    notice(event.payload.message);
    // A previous selection's translation must not remain actionable while a
    // new selection is being translated in the floating window.
    if (popup) {
      $("popup-text").textContent = "";
      $<HTMLButtonElement>("popup-copy").disabled = true;
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
}
