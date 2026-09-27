//! Independent LAN service: owns identity, trust, discovery and transfer jobs.
//! Only the commands adapter knows Tauri. No translation lock or AX/UIA calls
//! participate in networking, so a slow peer cannot block translation.
pub mod commands;
mod discovery;
mod identity;
mod protocol;
mod store;
#[cfg(test)]
mod tests;
mod transport;

use identity::Identity;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use store::{Record, Settings, Store, TrustedPeer};
use tokio::sync::{oneshot, Semaphore};
use tokio_util::sync::CancellationToken;

pub type Service = Arc<TransferService>;
pub type EventSink = Arc<dyn Fn(&str) + Send + Sync>;
/// Runtime network addresses are candidates only; the certificate fingerprint
/// must still match this id before a connection can reach pairing or data.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub online: bool,
    pub trusted: bool,
    #[serde(skip)]
    pub addresses: Vec<std::net::SocketAddr>,
    #[serde(skip)]
    pub service_name: String,
}
/// Short-lived UI challenge; id addresses one connection, while peer_id is the
/// stable certificate identity. The code is derived from that TLS session.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub code: String,
    pub incoming: bool,
    pub expires_at: u64,
}
struct Pending {
    view: Pairing,
    job: String,
    reply: oneshot::Sender<bool>,
}
struct Active {
    peer_id: String,
    cancel: CancellationToken,
    last_event: Instant,
}
struct Runtime {
    stop: CancellationToken,
    mdns: mdns_sd::ServiceDaemon,
    fullname: String,
}
struct Inner {
    settings: Settings,
    trusted: Vec<TrustedPeer>,
    records: Vec<Record>,
    peers: HashMap<String, Peer>,
    pending: HashMap<String, Pending>,
    active: HashMap<String, Active>,
    runtime: Option<Runtime>,
    status: String,
    error: String,
    device_id: String,
    /// An explicit refusal throttles repeat prompts, including reconnect spam.
    refused: HashMap<String, Instant>,
}
/// All public snapshots omit file source paths and truncate text previews. Full
/// received text is fetched explicitly; progress events carry no user content.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    settings: Settings,
    device_id: String,
    status: String,
    error: String,
    peers: Vec<Peer>,
    pairings: Vec<Pairing>,
    records: Vec<Record>,
}
/// Shared service owner. Mutexes protect short state updates; sockets run in
/// independent tasks, and the lifecycle mutex serializes reconfiguration.
pub struct TransferService {
    inner: Mutex<Inner>,
    store: Store,
    identity: Mutex<Option<Arc<Identity>>>,
    lifecycle: tokio::sync::Mutex<()>,
    events: EventSink,
    slots: Arc<Semaphore>,
}
/// Human-facing timestamps use milliseconds; pairing timeouts use monotonic
/// Tokio timers so clock adjustments cannot keep untrusted connections alive.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
/// Wire/UI platform label; the shipping application supports macOS and Windows.
fn platform_name() -> String {
    if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Windows"
    }
    .into()
}
/// Limit UTF-8 bytes as well as control characters before advertising metadata.
fn bounded_name(s: &str) -> Result<String, String> {
    let name = s.trim();
    if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        Err("设备名称需要 1–80 字节，不能含控制字符。".into())
    } else {
        Ok(name.into())
    }
}
impl TransferService {
    /// Load only this feature's data; a corrupt LAN file returns an error to its
    /// adapter rather than modifying translation settings or clearing trust.
    /// The adapter resolves/migrates locations; the service receives the exact
    /// default inbox path, and always preserves a saved custom destination.
    pub fn new(
        dir: PathBuf,
        default_receive_dir: PathBuf,
        events: EventSink,
    ) -> Result<Service, String> {
        let store = Store::new(dir)?;
        let name = hostname::get()
            .ok()
            .and_then(|s| s.into_string().ok())
            .filter(|s| bounded_name(s).is_ok())
            .unwrap_or_else(|| format!("我的 {}", platform_name()));
        let settings = store.read("settings.json")?.unwrap_or(Settings {
            enabled: true,
            name,
            receive_dir: default_receive_dir,
        });
        let trusted = store.read("trusted.json")?.unwrap_or_default();
        let mut records: Vec<Record> = store.read("inbox.json")?.unwrap_or_default();
        for r in &mut records {
            if !["completed", "failed", "cancelled"].contains(&r.phase.as_str()) {
                r.phase = "failed".into();
                r.error = "应用退出，传输未完成；请重新发送。".into();
            }
        }
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                settings,
                trusted,
                records,
                peers: HashMap::new(),
                pending: HashMap::new(),
                active: HashMap::new(),
                runtime: None,
                status: "stopped".into(),
                error: String::new(),
                device_id: String::new(),
                refused: HashMap::new(),
            }),
            store,
            identity: Mutex::new(None),
            lifecycle: tokio::sync::Mutex::new(()),
            events,
            slots: Arc::new(Semaphore::new(8)),
        }))
    }
    /// Notify adapters only after releasing state locks; callbacks may snapshot.
    fn emit(&self, kind: &str) {
        (self.events)(kind);
    }
    /// Merge offline trusted devices with the live discovery list so trust can
    /// be revoked even while the other computer is powered off.
    pub fn snapshot(&self) -> Snapshot {
        let i = self.inner.lock().unwrap();
        let mut peers: Vec<_> = i.peers.values().cloned().collect();
        for t in &i.trusted {
            if !peers.iter().any(|p| p.id == t.id) {
                peers.push(Peer {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    platform: t.platform.clone(),
                    online: false,
                    trusted: true,
                    addresses: vec![],
                    service_name: String::new(),
                });
            }
        }
        for p in &mut peers {
            p.trusted = i.trusted.iter().any(|t| t.id == p.id);
        }
        peers.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then(a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        Snapshot {
            settings: i.settings.clone(),
            device_id: i.device_id.clone(),
            status: i.status.clone(),
            error: i.error.clone(),
            peers,
            pairings: i.pending.values().map(|p| p.view.clone()).collect(),
            records: i
                .records
                .iter()
                .rev()
                .take(100)
                .map(|r| {
                    let mut v = r.clone();
                    v.text = r.text.chars().take(180).collect();
                    v
                })
                .collect(),
        }
    }
    /// Start at most one listener/discovery pair. Credential and network errors
    /// remain visible in this feature while the rest of the app keeps running.
    pub async fn start(self: &Service) {
        let _lifecycle = self.lifecycle.lock().await;
        {
            let mut i = self.inner.lock().unwrap();
            if !i.settings.enabled || i.runtime.is_some() {
                return;
            }
            i.status = "starting".into();
            i.error.clear();
        }
        self.emit("changed");
        let result = self.start_inner().await;
        if let Err(e) = result {
            let mut i = self.inner.lock().unwrap();
            i.status = "error".into();
            i.error = e;
        }
        self.emit("changed");
    }
    /// Load the stable credential before binding and publishing a single runtime.
    async fn start_inner(self: &Service) -> Result<(), String> {
        let existing = self.identity.lock().unwrap().clone();
        let identity = if let Some(id) = existing {
            id
        } else {
            Arc::new(
                tokio::task::spawn_blocking(Identity::load)
                    .await
                    .map_err(|e| e.to_string())??,
            )
        };
        *self.identity.lock().unwrap() = Some(identity.clone());
        self.inner.lock().unwrap().device_id = identity.id.clone();
        // Only this installation's reserved staging directories are eligible;
        // normal received files and another installation's work are untouched.
        let folder = self.inner.lock().unwrap().settings.receive_dir.clone();
        store::cleanup_staging(&folder, &identity.id)?;
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0")
            .await
            .map_err(|e| format!("无法启动局域网监听：{e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let stop = CancellationToken::new();
        let (mdns, fullname) = discovery::start(self.clone(), port, &identity.id, stop.clone())?;
        self.inner.lock().unwrap().runtime = Some(Runtime {
            stop: stop.clone(),
            mdns,
            fullname,
        });
        self.inner.lock().unwrap().status = "running".into();
        let service = self.clone();
        tokio::spawn(async move {
            transport::listen(service, identity, listener, stop).await;
        });
        Ok(())
    }
    /// Stopping or disabling also cancels all active tasks and pending prompts.
    /// Closing the main window does not call this; exiting the app does.
    pub fn stop(&self) {
        let mut i = self.inner.lock().unwrap();
        if let Some(r) = i.runtime.take() {
            r.stop.cancel();
            let _ = r.mdns.unregister(&r.fullname);
            let _ = r.mdns.shutdown();
        }
        for job in i.active.values() {
            job.cancel.cancel();
        }
        i.pending.clear();
        i.peers.clear();
        i.status = "stopped".into();
        drop(i);
        self.emit("changed");
    }
    /// Apply independent settings atomically. Destination/name changes wait for
    /// current jobs; disabling is always available and immediately cancels them.
    pub async fn configure(
        self: &Service,
        enabled: bool,
        name: String,
        receive_dir: PathBuf,
    ) -> Result<(), String> {
        let lifecycle = self.lifecycle.lock().await;
        let name = bounded_name(&name)?;
        if !receive_dir.is_absolute() {
            return Err("请选择绝对路径的接收目录。".into());
        }
        {
            let mut i = self.inner.lock().unwrap();
            if enabled && !i.active.is_empty() {
                return Err("请先等待或取消当前传输，再修改互传设置。".into());
            }
            let settings = Settings {
                enabled,
                name,
                receive_dir,
            };
            self.store.write("settings.json", &settings)?;
            i.settings = settings;
        }
        self.stop();
        drop(lifecycle);
        if enabled {
            self.start().await;
        }
        Ok(())
    }
    /// Restart a failed discovery worker explicitly, but never interrupt a live
    /// transfer as a side effect of the UI's reconnect action.
    pub async fn retry(self: &Service) -> Result<(), String> {
        let lifecycle = self.lifecycle.lock().await;
        if !self.inner.lock().unwrap().active.is_empty() {
            return Err("请等待或取消当前传输后再重新连接。".into());
        }
        self.stop();
        drop(lifecycle);
        self.start().await;
        Ok(())
    }
    /// A dead listener must not leave a green ready indicator. The lifecycle
    /// lock and token prevent a stale worker from stopping a newer generation.
    async fn network_failed(&self, stop: &CancellationToken, message: String) {
        let _lifecycle = self.lifecycle.lock().await;
        if stop.is_cancelled() {
            return;
        }
        self.stop();
        {
            let mut i = self.inner.lock().unwrap();
            i.status = "error".into();
            i.error = message;
        }
        self.emit("changed");
    }
    /// Check the stable certificate identity, never a mutable name or address.
    fn is_trusted(&self, id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .trusted
            .iter()
            .any(|t| t.id == id)
    }
    /// Persist first, then publish trust in memory; failed disk writes grant nothing.
    fn trust(&self, peer: TrustedPeer) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let mut trusted = i.trusted.clone();
        trusted.retain(|p| p.id != peer.id);
        trusted.push(peer);
        self.store.write("trusted.json", &trusted)?;
        i.trusted = trusted;
        Ok(())
    }
    /// Remove persisted trust before reporting success, then abort all work for
    /// that identity so existing connections cannot outlive the user's decision.
    pub fn forget(&self, peer_id: &str) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let mut trusted = i.trusted.clone();
        trusted.retain(|t| t.id != peer_id);
        self.store.write("trusted.json", &trusted)?;
        i.trusted = trusted;
        for job in i.active.values().filter(|j| j.peer_id == peer_id) {
            job.cancel.cancel();
        }
        i.refused.insert(peer_id.into(), Instant::now());
        i.pending.retain(|_, p| p.view.peer_id != peer_id);
        drop(i);
        self.emit("changed");
        Ok(())
    }
    /// Pairing responses are opaque per-session tokens, never just a peer name.
    pub fn decide(&self, id: &str, accepted: bool) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let p = i.pending.remove(id).ok_or("此配对请求已结束。")?;
        if !accepted {
            i.refused.insert(p.view.peer_id.clone(), Instant::now());
        }
        p.reply.send(accepted).map_err(|_| "此配对请求已结束。")?;
        drop(i);
        self.emit("changed");
        Ok(())
    }
    /// Wait for a bounded, connection-specific UI decision without holding locks.
    async fn consent(
        &self,
        job: &str,
        peer: &TrustedPeer,
        code: String,
        incoming: bool,
    ) -> Result<bool, String> {
        let (tx, rx) = oneshot::channel();
        let id = uuid::Uuid::new_v4().to_string();
        {
            let mut i = self.inner.lock().unwrap();
            i.refused
                .retain(|_, at| at.elapsed() < Duration::from_secs(30));
            if i.refused.contains_key(&peer.id) {
                return Err("此设备刚被拒绝，请稍后重试。".into());
            }
            if i.pending.len() >= 4 || i.pending.values().any(|p| p.view.peer_id == peer.id) {
                return Err("已有配对请求正在等待处理。".into());
            }
            i.pending.insert(
                id.clone(),
                Pending {
                    view: Pairing {
                        id: id.clone(),
                        peer_id: peer.id.clone(),
                        peer_name: peer.name.clone(),
                        code,
                        incoming,
                        expires_at: now() + protocol::PAIR_TIMEOUT.as_millis() as u64,
                    },
                    job: job.into(),
                    reply: tx,
                },
            );
        }
        self.emit("pairing");
        let answer = tokio::time::timeout(protocol::PAIR_TIMEOUT, rx).await;
        self.inner.lock().unwrap().pending.remove(&id);
        self.emit("changed");
        answer
            .map_err(|_| "首次信任确认超时，请重新发送。".to_string())?
            .map_err(|_| "配对已取消。".into())
    }
    /// Reserve a cancellable job only while enabled; cap state and socket concurrency.
    fn begin(
        &self,
        peer_id: String,
        peer_name: String,
        incoming: bool,
    ) -> Result<(String, CancellationToken), String> {
        let mut i = self.inner.lock().unwrap();
        if i.status != "running" || !i.settings.enabled {
            return Err("局域网互传已关闭。".into());
        }
        if i.active.len() >= 8 {
            return Err("同时传输任务过多，请稍后重试。".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        i.records.push(Record {
            id: id.clone(),
            peer_id: peer_id.clone(),
            peer_name,
            direction: if incoming { "received" } else { "sent" }.into(),
            phase: "connecting".into(),
            created_at: now(),
            bytes: 0,
            total: 0,
            text: String::new(),
            files: vec![],
            paths: vec![],
            error: String::new(),
        });
        i.active.insert(
            id.clone(),
            Active {
                peer_id,
                cancel: cancel.clone(),
                last_event: Instant::now() - Duration::from_secs(1),
            },
        );
        drop(i);
        self.emit("changed");
        Ok((id, cancel))
    }
    /// Publish a phase/metadata change immediately, with no file bodies in events.
    fn update(&self, id: &str, change: impl FnOnce(&mut Record)) {
        let mut i = self.inner.lock().unwrap();
        if let Some(r) = i.records.iter_mut().find(|r| r.id == id) {
            change(r);
        }
        drop(i);
        self.emit("changed");
    }
    /// Keep byte counts current but throttle UI events to roughly three per second.
    fn progress(&self, id: &str, bytes: u64) {
        let mut i = self.inner.lock().unwrap();
        if let Some(r) = i.records.iter_mut().find(|r| r.id == id) {
            r.bytes = bytes;
        }
        let notify = if let Some(a) = i.active.get_mut(id) {
            if a.last_event.elapsed() >= Duration::from_millis(300) {
                a.last_event = Instant::now();
                true
            } else {
                false
            }
        } else {
            false
        };
        drop(i);
        if notify {
            self.emit("changed");
        }
    }
    /// Persist receipt before acknowledging it to the sender. Incomplete files
    /// remain owned by transport guards until this succeeds.
    fn received(&self, id: &str, text: String, paths: Vec<PathBuf>) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let previous = i.records.clone();
        if let Some(r) = i.records.iter_mut().find(|r| r.id == id) {
            r.text = text;
            r.paths = paths;
            r.phase = "completed".into();
            r.bytes = r.total;
        }
        Self::prune(&mut i);
        if let Err(e) = self.store.write("inbox.json", &i.records) {
            i.records = previous;
            return Err(e);
        }
        Ok(())
    }
    /// Bound retained text and metadata while never evicting an active task.
    fn prune(i: &mut Inner) {
        while i.records.len() > 100
            || i.records.iter().map(|r| r.text.len()).sum::<usize>() > 8 * 1024 * 1024
        {
            if let Some(n) = i.records.iter().position(|r| !i.active.contains_key(&r.id)) {
                i.records.remove(n);
            } else {
                break;
            }
        }
    }
    /// Release job/pairing state and save the terminal record; a durable receipt
    /// stays completed even when its final acknowledgment could not reach the peer.
    fn finish(&self, id: &str, result: Result<(), String>) {
        let mut i = self.inner.lock().unwrap();
        let cancelled = i.active.get(id).is_some_and(|j| j.cancel.is_cancelled());
        let mut received = false;
        if let Some(r) = i.records.iter_mut().find(|r| r.id == id) {
            if r.phase != "completed" {
                match result {
                    Ok(()) => {
                        r.phase = "completed".into();
                        r.bytes = r.total;
                    }
                    Err(e) => {
                        r.phase = if cancelled { "cancelled" } else { "failed" }.into();
                        r.error = e;
                    }
                }
            }
            received = r.phase == "completed" && r.direction == "received";
        }
        i.active.remove(id);
        i.pending.retain(|_, p| p.job != id);
        Self::prune(&mut i);
        if let Err(e) = self.store.write("inbox.json", &i.records) {
            i.error = format!("接收记录保存失败：{e}");
        }
        drop(i);
        self.emit(if received { "received" } else { "changed" });
    }
    /// Local cancellation drops the socket and all uncommitted file guards.
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let i = self.inner.lock().unwrap();
        i.active.get(id).ok_or("任务已结束。")?.cancel.cancel();
        Ok(())
    }
    /// Full text is fetched explicitly from a completed record, not each snapshot.
    pub fn text(&self, id: &str) -> Result<String, String> {
        self.inner
            .lock()
            .unwrap()
            .records
            .iter()
            .find(|r| r.id == id && r.phase == "completed")
            .map(|r| r.text.clone())
            .ok_or("接收记录不可用。".into())
    }
    /// Clear records only; this operation deliberately does not delete files.
    pub fn clear(&self) -> Result<(), String> {
        let mut i = self.inner.lock().unwrap();
        let remaining: Vec<_> = i
            .records
            .iter()
            .filter(|r| i.active.contains_key(&r.id))
            .cloned()
            .collect();
        self.store.write("inbox.json", &remaining)?;
        i.records = remaining;
        drop(i);
        self.emit("changed");
        Ok(())
    }
    /// Resolve a local received-file index; the frontend cannot supply arbitrary paths.
    pub fn received_path(&self, id: &str, index: usize) -> Result<PathBuf, String> {
        self.inner
            .lock()
            .unwrap()
            .records
            .iter()
            .find(|r| r.id == id && r.direction == "received" && r.phase == "completed")
            .and_then(|r| r.paths.get(index))
            .cloned()
            .ok_or("找不到已接收的文件。".into())
    }
    /// File bodies are only opened after explicit selection and revalidated on
    /// send. Runtime peer selection is by fingerprint, not a frontend URL.
    pub fn send(
        self: &Service,
        peer_id: String,
        text: String,
        paths: Vec<PathBuf>,
    ) -> Result<String, String> {
        let peer = {
            let i = self.inner.lock().unwrap();
            if i.status != "running" {
                return Err("请先开启局域网互传。".into());
            }
            i.peers
                .get(&peer_id)
                .filter(|p| p.online)
                .cloned()
                .ok_or("目标设备已离线。")?
        };
        let metadata = transport::inspect_files(&paths)?;
        protocol::validate_offer(text.len() as u64, &metadata)?;
        let identity = self
            .identity
            .lock()
            .unwrap()
            .clone()
            .ok_or("设备身份尚未准备好。")?;
        // Fail before creating a job if a future adapter accidentally calls this
        // from a plain UI/OS thread. The Tauri send command enters Tokio for us.
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "无法启动传输任务：请重试或重启应用。".to_string())?;
        let (id, cancel) = self.begin(peer.id.clone(), peer.name.clone(), false)?;
        self.update(&id, |r| {
            r.text = text.clone();
            r.files = metadata.clone();
            r.total = text.len() as u64 + metadata.iter().map(|f| f.size).sum::<u64>();
        });
        let service = self.clone();
        let job = id.clone();
        runtime.spawn(async move {
            let result = tokio::select! {_ = cancel.cancelled()=>Err("传输已取消。".into()),r=transport::send(service.clone(),identity,&job,peer,text,paths,metadata)=>r};
            service.finish(&job, result);
        });
        Ok(id)
    }
}
