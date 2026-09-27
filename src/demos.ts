/** Self-contained, fixed-example UI demonstrations. They never read selections,
 * invoke translation, or dispatch real shortcuts. Only the saved writing shortcut
 * enters this module; all animation state belongs to the main window. */
const glyph = (path: string) => `<svg viewBox="0 0 24 24" aria-hidden="true">${path}</svg>`;
const play = glyph('<path d="m9 5 10 7-10 7z"/>');
const pause = glyph('<path d="M9 5v14M15 5v14"/>');
const replay = glyph('<path d="M4 10a8 8 0 1 1 1 8M4 4v6h6"/>');
const pointer = glyph('<path d="m5 3 14 10-7 1-3 7z"/>');
const arrow = glyph('<path d="M12 18V6m-5 5 5-5 5 5"/>');
const check = glyph('<path d="m5 12 4 4L19 6"/>');
const source = "请在请求失败时显示错误，并允许重试。";
const translated = "Show an error when the request fails and allow retrying.";
const duration = 10400;

/** The scenes share playback controls. Writing uses keys; reading shows the
 * mouse-only automatic-selection flow, which must never imply a required chord.
 * Animated content is decorative; a stable accessible description avoids a
 * screen reader announcing each typed character on every loop. */
export function demoMarkup(): string {
  return ["write", "read"].map((mode, index) => `
    <article class="demo-card ${mode}-demo" id="demo-${mode}" aria-labelledby="${mode}-title">
      <header class="demo-heading"><div><span class="demo-number">0${index + 1}</span><h2 id="${mode}-title">${mode === "write" ? "用母语写，让英文接棒" : "划过英文，读懂它"}</h2></div><span class="demo-direction">${mode === "write" ? "中 → EN" : "EN → 中"}</span></header>
      <p class="sr-only">${mode === "write" ? "演示：输入中文，按写入快捷键后，输入框中的文字变成英文，不自动发送。" : "演示：开启自动划词后，拖选英文并松开鼠标，选区稳定后自动在上方显示中文译文，无需快捷键。"}</p>
      <div class="demo-scene" aria-hidden="true">
        <div class="scene-chrome"><span class="scene-dots"><i></i><i></i><i></i></span><span>${mode === "write" ? "YOUR NEXT PROMPT" : "A LITTLE MORE CLARITY"}</span><span class="scene-mark">↗</span></div>
        ${mode === "write" ? `
          <div class="write-context"><span class="context-orb">✳</span><div><i></i><i></i></div></div>
          <div class="mock-input"><div class="input-text"><span class="typed-text"></span><span class="typing-caret"></span></div><div class="input-bottom"><span class="input-plus">＋</span><span class="input-model">Ask anything</span><span class="send-key">${arrow}</span></div></div>
          <div class="scene-caption"><span class="success-check">${check}</span><span class="scene-feedback">从一个想法开始</span></div>
        ` : `
          <div class="mock-document"><div class="document-heading"><span class="context-orb">✳</span><span>Implementation notes</span></div><p>Keep the current data visible<br>while loading new results.</p><p class="selection-line"><span class="selection-text">Allow users to retry.</span><span class="selection-pointer">${pointer}</span></p><div class="document-lines"><i></i><i></i></div></div>
          <div class="mini-translation"><div><span>译文 · 简体中文</span><span>⠿</span></div><p>允许用户重试。</p><footer><span>TranslateMe</span><span>⧉</span></footer></div>
          <div class="scene-caption"><span class="success-check">${check}</span><span class="scene-feedback">遇到一句想读懂的话</span></div>
        `}
      </div>
      <div class="keyboard-area" aria-hidden="true"><div class="keyboard-label"><span class="keyboard-action">${mode === "write" ? "输入你的想法" : "按住鼠标，划选文字"}</span><span class="shortcut-label" ${mode === "write" ? 'id="write-shortcut"' : ""}>${mode === "read" ? "无需按键" : ""}</span></div>
        ${mode === "write" ? `<div class="keyboard-deck"><div class="key-row ghost-keys">${"QWERTYUIOP".split("").map(key => `<span>${key}</span>`).join("")}</div><div class="shortcut-keys"></div></div>` : `<div class="mouse-deck"><div class="mouse-motion"><div class="demo-mouse"><span class="mouse-left"></span><span class="mouse-wheel"></span></div></div><div class="automatic-path"><span class="path-track"><i></i></span><span class="auto-spark">✦</span><span class="path-track"><i></i></span></div><div class="auto-result-icon"><span></span><span></span><span></span><i>${check}</i></div><div class="mouse-captions"><span>拖选 · 松开</span><span>自动翻译</span></div></div>`}
      </div>
      <footer class="demo-controls"><div class="demo-steps"><span>01 ${mode === "write" ? "输入" : "划选"}</span><i></i><span>02 ${mode === "write" ? "快捷键" : "松开"}</span><i></i><span>03 ${mode === "write" ? "回填" : "译文"}</span></div><div><button class="demo-toggle" aria-label="暂停${mode === "write" ? "写入" : "划词"}演示" title="暂停">${pause}</button><button class="demo-replay" aria-label="重播${mode === "write" ? "写入" : "划词"}演示" title="重播">${replay}</button></div></footer>
      <div class="demo-progress" aria-hidden="true"><span></span></div>
    </article>`).join("");
}

type Scene = {
  root: HTMLElement;
  mode: "write" | "read";
  elapsed: number;
  paused: boolean;
  text?: HTMLElement;
  feedback: HTMLElement;
  action: HTMLElement;
  toggle: HTMLButtonElement;
  stage: string;
};

/** One clock drives both scenes. Hidden views/documents stop the RAF entirely;
 * resuming resets the clock origin so background time never skips the story.
 * Reduced motion starts at the final frame; explicit play is still available. */
export function mountDemos() {
  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
  let visible = true;
  let frame = 0;
  let lastTime = 0;
  const scenes: Scene[] = (["write", "read"] as const).map(mode => {
    const root = document.getElementById(`demo-${mode}`)!;
    return {
      root, mode, elapsed: reducedMotion.matches ? 7000 : 0,
      paused: reducedMotion.matches,
      text: root.querySelector<HTMLElement>(".typed-text") ?? undefined,
      feedback: root.querySelector<HTMLElement>(".scene-feedback")!,
      action: root.querySelector<HTMLElement>(".keyboard-action")!,
      toggle: root.querySelector<HTMLButtonElement>(".demo-toggle")!,
      stage: "",
    };
  });

  // Writing changes text during the key press. Reading instead waits one
  // illustrative second after mouse release: no keyboard event is involved.
  // This is an explanatory timeline, not a promise of real translation latency.
  function render(scene: Scene) {
    const t = scene.elapsed;
    const writing = scene.mode === "write";
    const result = t >= (writing ? 4550 : 3400);
    const pressing = writing && t >= 3900 && t < 5050;
    const stage = result ? "result" : t >= (writing ? 3400 : 2400) ? "trigger" : "prepare";
    scene.root.classList.toggle("is-pressing", pressing);
    scene.root.classList.toggle("is-result", result);
    scene.root.classList.toggle("is-typing", scene.mode === "write" && t < 3000);
    scene.root.classList.toggle("is-paused", scene.paused);
    scene.root.style.setProperty("--progress", `${Math.min(t / duration, 1) * 100}%`);
    if (scene.text) {
      const next = result ? translated : source.slice(0, Math.max(0, Math.floor((t - 450) / 105)));
      if (scene.text.textContent !== next) scene.text.textContent = next;
    } else {
      const selection = Math.min(1, Math.max(0, (t - 800) / 1600));
      scene.root.style.setProperty("--selection", `${selection * 100}%`);
      scene.root.classList.toggle("is-selecting", t >= 700 && t < 2400);
      scene.root.classList.toggle("is-settling", t >= 2400 && t < 3400);
      scene.root.style.setProperty("--mouse-shift", `${(selection - .5) * 14}px`);
      scene.root.style.setProperty("--settled", `${Math.min(1, Math.max(0, (t - 2400) / 1000)) * 100}%`);
    }
    if (scene.stage !== stage) {
      scene.stage = stage;
      scene.root.dataset.stage = stage;
      scene.feedback.textContent = result
        ? scene.mode === "write" ? "英文已回填，随时发送" : "译文就在原文上方"
        : scene.mode === "write" ? "从一个想法开始" : "遇到一句想读懂的话";
      scene.action.textContent = stage === "prepare"
        ? scene.mode === "write" ? "输入你的想法" : "按住鼠标，划选文字"
        : result ? scene.mode === "write" ? "已回填 · 不自动发送" : "自动显示 · 原文保留" : writing ? "按下组合键" : "已松开，等待选区稳定";
    }
  }

  function updateControl(scene: Scene) {
    scene.toggle.innerHTML = scene.paused ? play : pause;
    scene.toggle.title = scene.paused ? "播放" : "暂停";
    scene.toggle.setAttribute("aria-label", `${scene.paused ? "播放" : "暂停"}${scene.mode === "write" ? "写入" : "划词"}演示`);
  }

  function tick(now: number) {
    const delta = lastTime ? Math.min(now - lastTime, 100) : 0;
    lastTime = now;
    frame = 0;
    for (const scene of scenes) {
      if (!scene.paused) {
        scene.elapsed += delta;
        if (scene.elapsed >= duration) {
          // With reduced motion, user-initiated playback is one pass only.
          if (reducedMotion.matches) {
            scene.elapsed = 7000;
            scene.paused = true;
            updateControl(scene);
          } else scene.elapsed %= duration;
        }
        render(scene);
      }
    }
    schedule();
  }

  function schedule() {
    if (visible && !document.hidden && scenes.some(scene => !scene.paused)) {
      if (!frame) frame = requestAnimationFrame(tick);
    } else {
      cancelAnimationFrame(frame);
      frame = 0;
      lastTime = 0;
    }
  }

  scenes.forEach(scene => {
    scene.toggle.onclick = () => {
      scene.paused = !scene.paused;
      if (!scene.paused && reducedMotion.matches) scene.elapsed = 0;
      updateControl(scene);
      render(scene);
      schedule();
    };
    scene.root.querySelector<HTMLButtonElement>(".demo-replay")!.onclick = () => {
      scene.elapsed = 0;
      scene.paused = false;
      updateControl(scene);
      render(scene);
      schedule();
    };
    updateControl(scene);
    render(scene);
  });
  document.addEventListener("visibilitychange", schedule);
  reducedMotion.addEventListener("change", () => {
    if (!reducedMotion.matches) return;
    scenes.forEach(scene => {
      scene.paused = true;
      scene.elapsed = 7000;
      updateControl(scene);
      render(scene);
    });
    schedule();
  });
  schedule();

  return {
    setVisible(value: boolean) {
      visible = value;
      schedule();
    },
    /** Render saved shortcuts as text nodes, never HTML. Custom key names may
     * come from a user-edited configuration; the demo must not interpret them. */
    setWriteShortcut(write: string) {
      scenes.filter(scene => scene.mode === "write").forEach(scene => {
        const mac = navigator.platform.includes("Mac");
        const keys = write.split("+").filter(Boolean).map(key => {
          const labels: Record<string, string> = {
            CommandOrControl: mac ? "⌘" : "Ctrl", Super: "⌘", Command: "⌘",
            Control: "Ctrl", Shift: "⇧", Alt: mac ? "⌥" : "Alt",
          };
          return labels[key] ?? key;
        });
        scene.root.querySelector(".shortcut-label")!.textContent = keys.join(" + ");
        const deck = scene.root.querySelector(".shortcut-keys")!;
        deck.replaceChildren(...keys.map((key, index) => {
          const cap = document.createElement("kbd");
          cap.className = `demo-key${index === keys.length - 1 ? " letter-key" : ""}`;
          cap.textContent = key;
          return cap;
        }));
      });
    },
  };
}
