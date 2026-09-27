import { desktopPlatform, renderShortcut, type DesktopPlatform } from "./shortcuts";

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
// Fixed SVG geometry avoids the font baseline/emoji fallback that displaced the
// old Unicode mark. Claude mark: Simple Icons (CC0), simpleicons.org/?q=claude.
const claude = `<svg aria-hidden="true" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z"/></svg>`;
const source = "请在请求失败时显示错误，并允许重试。";
const translated = "Show an error when the request fails and allow retrying.";
const duration = 10400;

/** The scenes share playback controls. Writing uses keys; reading shows the
 * pointer-based automatic-selection flow, which must never imply a required chord.
 * Animated content is decorative; a stable accessible description avoids a
 * screen reader announcing each typed character on every loop. */
export function demoMarkup(): string {
  return ["write", "read"].map((mode, index) => `
    <article class="demo-card ${mode}-demo" id="demo-${mode}" data-platform="${desktopPlatform()}" aria-labelledby="${mode}-title">
      <header class="demo-heading"><div><span class="demo-number">0${index + 1}</span><h2 id="${mode}-title">${mode === "write" ? "用母语写，让英文接棒" : "划过英文，读懂它"}</h2></div><span class="demo-direction">${mode === "write" ? "中 → EN" : "EN → 中"}</span></header>
      <p class="sr-only">${mode === "write" ? "演示：输入中文，按写入快捷键后，输入框中的文字变成英文，不自动发送。" : "演示：开启自动划词后，拖选英文并松开，选区稳定后自动在上方显示中文译文，无需快捷键。"}</p>
      <div class="demo-scene" aria-hidden="true">
        <div class="scene-chrome"><span class="scene-dots"><i></i><i></i><i></i></span><span>${mode === "write" ? "YOUR NEXT PROMPT" : "A LITTLE MORE CLARITY"}</span><span class="scene-mark">↗</span></div>
        ${mode === "write" ? `
          <div class="write-context"><span class="context-orb">${claude}</span><div><i></i><i></i></div></div>
          <div class="mock-input"><div class="input-text"><span class="typed-text"></span><span class="typing-caret"></span></div><div class="input-bottom"><span class="input-plus">＋</span><span class="input-model">Ask anything</span><span class="send-key">${arrow}</span></div></div>
          <div class="scene-caption"><span class="success-check">${check}</span><span class="scene-feedback">从一个想法开始</span></div>
        ` : `
          <div class="mock-document"><div class="document-heading"><span class="context-orb">${claude}</span><span>Implementation notes</span></div><p>Keep the current data visible<br>while loading new results.</p><p class="selection-line"><span class="selection-text">Allow users to retry.</span><span class="selection-pointer">${pointer}</span></p><div class="document-lines"><i></i><i></i></div></div>
          <div class="mini-translation"><div><span>译文 · 简体中文</span><span>⠿</span></div><p>允许用户重试。</p><footer><span>TranslateMe</span><span>⧉</span></footer></div>
          <div class="scene-caption"><span class="success-check">${check}</span><span class="scene-feedback">遇到一句想读懂的话</span></div>
        `}
      </div>
      <div class="keyboard-area" aria-hidden="true"><div class="keyboard-label"><span class="keyboard-action">${mode === "write" ? "输入你的想法" : "按住并拖选文字"}</span><span class="shortcut-label" ${mode === "write" ? 'id="write-shortcut"' : ""}>${mode === "read" ? "无需按键" : ""}</span></div>
        ${mode === "write" ? `<div class="keyboard-deck"><div class="key-row ghost-keys">${"QWERTYUIOP".split("").map(key => `<span>${key}</span>`).join("")}</div><div class="shortcut-keys"></div></div>` : `<div class="mouse-deck"><div class="mouse-motion"><div class="demo-trackpad"><span class="trackpad-trail"></span><span class="trackpad-contact"></span><span class="trackpad-finger"></span></div><div class="demo-mouse"><span class="mouse-left"></span><span class="mouse-wheel"></span></div></div><div class="automatic-path"><span class="path-track"><i></i></span><span class="auto-spark">✦</span><span class="path-track"><i></i></span></div><div class="auto-result-icon"><span></span><span></span><span></span><i>${check}</i></div><div class="mouse-captions"><span class="device-caption">拖选 · 松开</span><span>自动翻译</span></div></div>`}
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
  let platform: DesktopPlatform = desktopPlatform();
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
  // illustrative second after pointer release: no keyboard event is involved.
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
      scene.root.style.setProperty("--finger-shift", `${selection * 34}px`);
      scene.root.style.setProperty("--settled", `${Math.min(1, Math.max(0, (t - 2400) / 1000)) * 100}%`);
    }
    if (scene.stage !== stage) {
      scene.stage = stage;
      scene.root.dataset.stage = stage;
      scene.feedback.textContent = result
        ? scene.mode === "write" ? "英文已回填，随时发送" : "译文就在原文上方"
        : scene.mode === "write" ? "从一个想法开始" : "遇到一句想读懂的话";
      scene.action.textContent = stage === "prepare"
        ? scene.mode === "write" ? "输入你的想法" : platform === "macOS" ? "按住触控板，拖选文字" : "按住鼠标，拖选文字"
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
    /** Settings and tutorials share modifier semantics. Native platform updates
     * also switch the pointer illustration; this never changes input capture. */
    setWriteShortcut(write: string, nextPlatform = desktopPlatform()) {
      platform = nextPlatform;
      scenes.forEach(scene => {
        scene.root.dataset.platform = platform;
        if (scene.mode === "write") {
          renderShortcut(scene.root.querySelector(".shortcut-label")!, write, platform);
          renderShortcut(scene.root.querySelector(".shortcut-keys")!, write, platform, true);
        } else {
          scene.root.querySelector(".device-caption")!.textContent = platform === "macOS" ? "触控板 · 拖选" : "鼠标 · 拖选";
          scene.root.querySelector(".sr-only")!.textContent = `演示：使用${platform === "macOS" ? "触控板" : "鼠标"}拖选英文并松开，选区稳定后自动在上方显示译文，无需快捷键。`;
        }
        // Refresh cached captions when native status arrives after first paint.
        scene.stage = "";
        render(scene);
      });
    },
  };
}
