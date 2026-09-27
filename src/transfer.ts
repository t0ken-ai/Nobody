/** LAN page owns only its view/drafts. The Rust service owns peers, trust and
 * jobs even when this page is hidden; no file body passes through the webview. */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import "./transfer.css";

type Peer = { id: string; name: string; platform: string; online: boolean; trusted: boolean };
type Pairing = { id: string; peerId: string; peerName: string; code: string; incoming: boolean; expiresAt: number };
type FileMeta = { name: string; size: number };
type PickedFile = FileMeta & { path: string };
type RecordItem = { id: string; peerId: string; peerName: string; direction: string; phase: string; createdAt: number; bytes: number; total: number; text: string; files: FileMeta[]; paths: string[]; error: string };
type Settings = { enabled: boolean; name: string; receiveDir: string };
type Snapshot = { settings: Settings; deviceId: string; status: string; error: string; peers: Peer[]; pairings: Pairing[]; records: RecordItem[] };
/** Escape untrusted device names, text and filenames at each HTML boundary. */
const esc = (s: string) => s.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
/** Binary units match backend byte limits rather than misleading decimal labels. */
const size = (n: number) => n < 1024 ? `${n} B` : n < 1024 ** 2 ? `${(n / 1024).toFixed(1)} KiB` : n < 1024 ** 3 ? `${(n / 1024 ** 2).toFixed(1)} MiB` : `${(n / 1024 ** 3).toFixed(1)} GiB`;
const phases: Record<string, string> = { connecting: "连接中", pairing: "等待信任确认", transferring: "传输中", verifying: "正在确认接收", completed: "已完成", failed: "未完成", cancelled: "已取消" };
/** Only terminal jobs release the draft/send guard and offer record actions. */
const finished = (r: RecordItem) => ["completed", "failed", "cancelled"].includes(r.phase);
// Only the view is paginated: the service still owns the full bounded history
// and active jobs. Five compact previews keep the desktop page easy to scan.
const recordsPerPage = 5;

/** Static markup uses native form controls; peer-provided values are escaped
 * only at the rendering boundary and never treated as HTML, URLs or commands. */
export function transferMarkup() {
  return `<div class="heading-row lan-heading"><h1>局域网互传</h1><span id="lan-status" class="lan-status">正在连接…</span></div>
  <div id="lan-notice" class="notice" role="status" hidden></div>
  <div id="lan-pairings" aria-live="polite"></div>
  <div class="lan-grid"><aside class="lan-devices"><div class="lan-section-heading"><h2>附近设备</h2><span id="lan-count">0 在线</span></div><div id="lan-peers" class="lan-peer-list"></div></aside>
    <div class="lan-compose"><div class="lan-section-heading"><h2>发送到 <span id="lan-destination">一台电脑</span></h2><span id="lan-trust-label" class="muted">先选择左侧设备</span></div>
      <label class="sr-only" for="lan-text">要发送的文字</label><textarea id="lan-text" spellcheck="false" placeholder="粘贴文字、代码片段或报错日志…"></textarea>
      <button id="lan-add-files" type="button" class="lan-drop"><span class="lan-drop-icon" aria-hidden="true">＋</span><span><strong>添加或拖入文件</strong><small>可多选，不含文件夹</small></span></button>
      <ul id="lan-files" class="lan-file-list" aria-label="待发送文件"></ul>
      <div class="lan-send-row"><span id="lan-send-info" class="field-help">加密直传</span><button id="lan-send" class="button primary" disabled>发送 <span aria-hidden="true">↗</span></button></div>
    </div></div>
  <details class="lan-preferences"><summary>本机与接收设置 <span id="lan-self-name"></span></summary><div class="lan-settings-body"><div class="field-row"><label class="field">这台电脑的名称<input id="lan-name" maxlength="80" autocomplete="off" /></label><label class="field">局域网互传<span class="switch-row"><input id="lan-enabled" type="checkbox" /><span>允许发现并接收已信任设备的内容</span></span></label></div><div class="lan-directory"><div><span class="field-help">文件接收目录</span><p id="lan-directory-path"></p></div><button id="lan-directory" class="button small" type="button">选择目录</button></div><div class="lan-send-row"><span id="lan-fingerprint" class="field-help"></span><div><button id="lan-retry" class="text-button" type="button">重新连接</button><button id="lan-save" class="button small" type="button">保存设置</button></div></div></div></details>
  <div class="lan-inbox-heading"><h2>传输记录 <span id="lan-record-count" class="lan-record-count"></span></h2><div class="lan-record-actions"><div class="lan-tabs" role="group" aria-label="记录方向"><button data-lan-filter="all" class="active" aria-pressed="true">全部</button><button data-lan-filter="received" aria-pressed="false">收到</button><button data-lan-filter="sent" aria-pressed="false">发出</button></div><nav id="lan-pagination" class="lan-pagination" aria-label="传输记录分页" hidden><button id="lan-prev" class="button small" aria-label="上一页" aria-controls="lan-records">‹</button><span id="lan-page-info" role="status"></span><button id="lan-next" class="button small" aria-label="下一页" aria-controls="lan-records">›</button></nav><button id="lan-clear" class="button small" title="清除已结束的记录，不删除文件" aria-label="清除已结束记录">清理记录</button></div></div><div id="lan-records" class="lan-records"></div>
  <dialog id="lan-text-dialog" class="lan-text-dialog"><div class="lan-section-heading"><h2>文字内容</h2><button id="lan-text-close" class="button small">关闭</button></div><pre id="lan-full-text"></pre><button id="lan-text-copy" class="button primary">复制文字</button></dialog>`;
}

/** Mount once to preserve drafts across navigation. Snapshot refreshes coalesce
 * progress events; hidden views continue to receive pairing requests. */
export function mountTransfer(root: HTMLElement, showPage: () => void) {
  const $ = <T extends HTMLElement = HTMLElement>(id: string) => root.querySelector<T>(`#${id}`)!;
  let current: Snapshot | undefined;
  let selected = "";
  let files: PickedFile[] = [];
  let dirty = false;
  let folder = "";
  let sending = false;
  let sentDraft: { id: string; text: string; paths: string[] } | undefined;
  let filter = "all";
  let page = 0;
  let pairingKey = "";
  let timer: ReturnType<typeof setTimeout> | undefined;
  let refreshing = false;
  let refreshAgain = false;
  const input = $<HTMLTextAreaElement>("lan-text");
  /** Keep feature feedback local and use textContent for errors from the OS/peer. */
  const note = (message: string, error = false) => {
    $("lan-notice").textContent = message; $("lan-notice").hidden = !message;
    $("lan-notice").classList.toggle("error", error);
  };
  /** Keep async command errors local; event handlers never leak rejections. */
  const run = async (action: () => Promise<void>) => { try { await action(); } catch (e) { note(String(e), true); } };
  /** Browser preview cannot invoke real discovery or fake successful sends. */
  const command = <T>(name: string, args?: Record<string, unknown>) => {
    if (!isTauri()) return Promise.reject(new Error("请打开桌面应用使用局域网互传。"));
    return invoke<T>(name, args);
  };
  /** Gate duplicate sends while a draft is active, preserving edits for a later send. */
  function composeState() {
    const peer = current?.peers.find(p => p.id === selected);
    $("lan-destination").textContent = peer?.name ?? "一台电脑";
    $("lan-trust-label").textContent = peer ? peer.trusted ? "自动接收" : "需双方确认信任" : "选择左侧设备";
    const pending = sentDraft && current?.records.some(r => r.id === sentDraft!.id && !finished(r));
    $<HTMLButtonElement>("lan-send").disabled = sending || !!pending || current?.status !== "running" || !peer?.online || (!input.value && !files.length);
    $<HTMLButtonElement>("lan-add-files").disabled = sending || !isTauri();
    input.disabled = sending;
    $("lan-send-info").textContent = files.length ? `${files.length} 个文件 · ${size(files.reduce((n, f) => n + f.size, 0))}` : "加密直传";
  }
  /** Render metadata only; Rust opens the selected file at actual send time. */
  function renderFiles() {
    $("lan-files").innerHTML = files.map((f, index) => `<li><span class="lan-file-symbol" aria-hidden="true">↗</span><span class="lan-file-name">${esc(f.name)}</span><small>${size(f.size)}</small><button class="icon-button" data-lan-remove="${index}" aria-label="移除 ${esc(f.name)}">×</button></li>`).join("");
    composeState();
  }
  /** Keep offline trusted peers manageable; selection is keyed by identity, not name. */
  function renderPeers() {
    if (!current) return;
    $("lan-count").textContent = `${current.peers.filter(p => p.online).length} 在线`;
    $("lan-peers").innerHTML = current.peers.length ? current.peers.map(p => `<div class="lan-peer ${selected === p.id ? "selected" : ""} ${p.online ? "" : "offline"}"><button class="lan-peer-select" data-lan-peer="${esc(p.id)}" aria-pressed="${selected === p.id}" title="设备标识 ${esc(p.id.slice(0, 12))}"><span class="lan-device-icon" aria-hidden="true">▱</span><span><strong>${esc(p.name)}</strong><small>${esc(p.platform)} · ${p.online ? "在线" : "离线"} · ${p.trusted ? "已信任" : "待信任"}</small></span><i aria-hidden="true"></i></button>${p.trusted ? `<div class="lan-peer-actions"><button class="lan-forget button small" data-lan-forget="${esc(p.id)}" title="后续连接需重新确认信任">解除信任</button></div>` : ""}</div>`).join("") : `<div class="lan-empty"><span aria-hidden="true">⌁</span><strong>${current.settings.enabled ? "正在寻找附近电脑" : "互传已关闭"}</strong><p>${current.settings.enabled ? "在另一台电脑打开 TranslateMe" : "在下方设置中开启互传"}</p></div>`;
    composeState();
  }
  /** Paginate after filtering, preserving the page during live progress updates.
   * Clamp after removals so the last page cannot become an empty dead end.
   * Pager controls stay mounted to preserve keyboard focus across refreshes;
   * full text and file reveal continue to use the original record identity. */
  function renderRecords() {
    const records = current?.records.filter(r => filter === "all" || r.direction === filter) ?? [];
    const pageCount = Math.max(1, Math.ceil(records.length / recordsPerPage));
    page = Math.min(page, pageCount - 1);
    $("lan-record-count").textContent = `${records.length} 条`;
    $("lan-pagination").hidden = pageCount <= 1;
    $("lan-page-info").textContent = `${page + 1} / ${pageCount}`;
    $("lan-page-info").setAttribute("aria-label", `第 ${page + 1} 页，共 ${pageCount} 页`);
    $<HTMLButtonElement>("lan-prev").disabled = page === 0;
    $<HTMLButtonElement>("lan-next").disabled = page === pageCount - 1;
    $<HTMLButtonElement>("lan-clear").disabled = !current?.records.some(finished);
    const visible = records.slice(page * recordsPerPage, (page + 1) * recordsPerPage);
    $("lan-records").innerHTML = records.length ? visible.map(r => {
      const received = r.direction === "received";
      const percent = r.total ? Math.min(100, r.bytes / r.total * 100) : r.phase === "completed" ? 100 : 0;
      return `<article class="lan-record"><div class="lan-record-heading"><span class="lan-record-direction" aria-hidden="true">${received ? "↙" : "↗"}</span><div><strong>${received ? "来自" : "发给"} ${esc(r.peerName)}</strong><small>${new Date(r.createdAt).toLocaleString()}${r.total ? ` · ${size(r.total)}` : ""}</small></div><span class="lan-phase ${esc(r.phase)}">${phases[r.phase] ?? "处理中"}</span>${!finished(r) ? `<button class="text-button" data-lan-cancel="${r.id}">取消</button>` : ""}</div>
      ${!finished(r) ? `<div class="lan-progress" role="progressbar" aria-label="传输进度" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(percent)}"><span style="width:${percent}%"></span></div>` : ""}
      ${r.text ? `<pre class="lan-preview">${esc(r.text)}</pre>${r.phase === "completed" ? `<div class="lan-text-actions"><button class="text-button" data-lan-read="${r.id}" aria-label="查看全文">全文</button><button class="text-button" data-lan-copy="${r.id}" aria-label="复制文字">复制</button></div>` : ""}` : ""}
      ${r.files.length ? `<ul class="lan-received-files">${r.files.map((f, index) => `<li><span>${esc(f.name)}</span><small>${size(f.size)}</small>${received && r.phase === "completed" ? `<button class="text-button" data-lan-reveal="${r.id}" data-index="${index}" aria-label="在文件夹中显示 ${esc(f.name)}">显示文件</button>` : ""}</li>`).join("")}</ul>` : ""}
      ${r.error ? `<p class="lan-record-error">${esc(r.error)}</p>` : ""}</article>`;
    }).join("") : `<div class="lan-empty-records">${filter === "received" ? "暂无接收记录" : filter === "sent" ? "暂无发送记录" : "暂无传输记录"}</div>`;
  }
  /** Avoid rebuilding unchanged prompts so refreshes do not steal keyboard focus. */
  function renderPairings() {
    const requests = current?.pairings ?? [];
    const key = JSON.stringify(requests);
    if (key === pairingKey) return;
    pairingKey = key;
    $("lan-pairings").innerHTML = requests.map(p => `<section class="lan-pairing" role="region" aria-label="首次信任确认"><div><span class="eyebrow">首次连接</span><h2>信任「${esc(p.peerName)}」？</h2><p>核对两台电脑上的校验码一致，再确认信任。</p></div><div class="lan-pair-code" aria-label="校验码 ${p.code}">${p.code.slice(0, 3)} ${p.code.slice(3)}</div><div class="lan-pair-actions"><button class="button" data-lan-reject="${p.id}">拒绝</button><button class="button primary" data-lan-trust="${p.id}">校验码一致，信任</button></div><p class="lan-pair-help">${p.incoming ? "对方正在向这台电脑发送。" : "也请在对方电脑上确认。"}信任后自动接收，可随时解除。确认前不会传输正文和文件内容。</p></section>`).join("");
  }
  /** Reconcile service state without overwriting unsaved settings or edited drafts. */
  function render() {
    if (!current) return;
    // Clear only a successfully delivered, unchanged draft. Failed/cancelled
    // sends and any edits made while a transfer runs remain available to retry.
    if (sentDraft) {
      const record = current.records.find(r => r.id === sentDraft!.id);
      if (record && finished(record)) {
        if (record.phase === "completed" && input.value === sentDraft.text && JSON.stringify(files.map(f => f.path)) === JSON.stringify(sentDraft.paths)) {
          input.value = ""; files = []; renderFiles();
        }
        sentDraft = undefined;
      }
    }
    const statusText: Record<string, string> = { running: "● 局域网已就绪", starting: "◌ 正在准备互传", stopped: "○ 互传已关闭", error: "● 连接未就绪" };
    $("lan-status").textContent = statusText[current.status] ?? current.status;
    $("lan-status").dataset.state = current.status;
    $("lan-self-name").textContent = current.settings.name;
    if (!dirty) {
      $<HTMLInputElement>("lan-name").value = current.settings.name;
      $<HTMLInputElement>("lan-enabled").checked = current.settings.enabled;
      folder = current.settings.receiveDir;
      $("lan-directory-path").textContent = folder;
    }
    $("lan-fingerprint").textContent = current.deviceId ? `设备标识 ${current.deviceId.slice(0, 16)}` : "正在准备设备身份…";
    if (current.error) note(current.error, true);
    renderPeers(); renderPairings(); renderRecords(); composeState();
  }
  /** Only one snapshot can be in flight, preventing old responses from
   * overwriting newer job state during bursts of progress/discovery events. */
  async function refresh() {
    if (refreshing) { refreshAgain = true; return; }
    refreshing = true;
    try { current = await command<Snapshot>("get_transfer_state"); render(); }
    catch (e) { $("lan-status").textContent = "互传未就绪"; note(String(e), true); }
    finally { refreshing = false; if (refreshAgain) { refreshAgain = false; schedule(); } }
  }
  /** Coalesce discovery/progress bursts; Rust retains all state between refreshes. */
  function schedule() { if (!timer) timer = setTimeout(() => { timer = undefined; void refresh(); }, 180); }
  /** Deduplicate by local path and cap the draft before server-side revalidation. */
  async function addFiles(next: PickedFile[]) {
    for (const file of next) if (!files.some(f => f.path === file.path)) files.push(file);
    if (files.length > 100) { files = files.slice(0, 100); note("每次最多选择 100 个文件。", true); }
    renderFiles();
  }
  input.oninput = composeState;
  $("lan-name").oninput = () => { dirty = true; };
  $("lan-enabled").onchange = () => { dirty = true; };
  $("lan-add-files").onclick = () => void run(async () => addFiles(await command<PickedFile[]>("pick_transfer_files")));
  $("lan-directory").onclick = () => void run(async () => { const result = await command<string | null>("pick_transfer_directory"); if (result) { folder = result; dirty = true; $("lan-directory-path").textContent = folder; } });
  $("lan-save").onclick = () => void run(async () => { await command("save_transfer_settings", { enabled: $<HTMLInputElement>("lan-enabled").checked, name: $<HTMLInputElement>("lan-name").value, receiveDir: folder }); dirty = false; note("互传设置已保存。"); await refresh(); });
  $("lan-retry").onclick = () => void run(async () => { note(""); await command("retry_transfer_service"); await refresh(); });
  $("lan-send").onclick = () => void run(async () => {
    sending = true; composeState();
    try {
      const text = input.value; const paths = files.map(f => f.path);
      const id = await command<string>("send_transfer", { peerId: selected, text, paths });
      sentDraft = { id, text, paths };
      note("已开始发送，可在传输记录中查看进度。"); await refresh();
    }
    finally { sending = false; composeState(); }
  });
  $("lan-clear").onclick = () => void run(async () => { await command("clear_transfer_records"); note("已清除结束的记录，保存的文件保持不变。"); await refresh(); });
  // Reuse the snapshot already in memory; changing pages never starts a network
  // operation or modifies stored history. Filter changes below reset the page.
  $("lan-prev").onclick = () => { page = Math.max(0, page - 1); renderRecords(); };
  $("lan-next").onclick = () => { page += 1; renderRecords(); };
  const dialog = $<HTMLDialogElement>("lan-text-dialog");
  $("lan-text-close").onclick = () => dialog.close();
  $("lan-text-copy").onclick = () => void run(async () => { await command("copy_text", { text: $("lan-full-text").textContent ?? "" }); note("文字已复制。"); });
  /** Delegation keeps file/record buttons functional across snapshot refreshes. */
  root.addEventListener("click", event => {
    const button = (event.target as Element).closest<HTMLButtonElement>("button"); if (!button) return;
    const d = button.dataset;
    if (d.lanPeer) { selected = d.lanPeer; renderPeers(); }
    if (d.lanRemove !== undefined && !sending) { files.splice(Number(d.lanRemove), 1); renderFiles(); }
    if (d.lanFilter) {
      filter = d.lanFilter; page = 0;
      root.querySelectorAll("[data-lan-filter]").forEach(n => {
        const active = (n as HTMLElement).dataset.lanFilter === filter;
        n.classList.toggle("active", active); n.setAttribute("aria-pressed", String(active));
      });
      renderRecords();
    }
    if (d.lanTrust || d.lanReject) void run(async () => { await command("answer_transfer_pairing", { id: d.lanTrust ?? d.lanReject, accepted: !!d.lanTrust }); await refresh(); });
    if (d.lanForget) void run(async () => { await command("forget_transfer_peer", { id: d.lanForget }); note("已解除信任，后续连接需要重新确认。"); await refresh(); });
    if (d.lanCancel) void run(async () => { await command("cancel_transfer", { id: d.lanCancel }); await refresh(); });
    if (d.lanCopy) void run(async () => { const text = await command<string>("read_transfer_text", { id: d.lanCopy }); await command("copy_text", { text }); note("文字已复制。"); });
    if (d.lanRead) void run(async () => { $("lan-full-text").textContent = await command<string>("read_transfer_text", { id: d.lanRead }); dialog.showModal(); });
    if (d.lanReveal) void run(async () => { await command("reveal_transfer_file", { id: d.lanReveal, index: Number(d.index) }); });
  });
  if (isTauri()) {
    void run(async () => {
      await listen<string>("transfer-event", event => { if (event.payload === "pairing") showPage(); schedule(); });
      await getCurrentWebviewWindow().onDragDropEvent(event => {
        if (root.hidden) return;
        const payload = event.payload;
        $("lan-add-files").classList.toggle("dragging", payload.type === "over" || payload.type === "enter");
        if (payload.type === "drop" && !sending) void run(async () => addFiles(await command<PickedFile[]>("inspect_transfer_files", { paths: payload.paths })));
      });
      await refresh();
    });
  } else {
    $("lan-status").textContent = "桌面预览";
    $("lan-peers").innerHTML = `<div class="lan-empty"><span aria-hidden="true">⌁</span><strong>设备会自动出现在这里</strong><p>发现、配对和传输需要打开桌面应用。</p></div>`;
    note("当前是界面预览，没有连接真实设备。"); composeState(); renderRecords();
  }
  return { show() { if (isTauri()) { schedule(); void command("mark_transfer_seen").catch(() => {}); } } };
}
