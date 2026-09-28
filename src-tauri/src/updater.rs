//! Release updates have their own state, timer and HTTP client. They never read
//! translation input, LLM credentials or LAN identities. Only the coordinator
//! supplies a short-lived installation lease that keeps other work idle.
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio_util::sync::CancellationToken;

const CHECK_INTERVAL: u64 = 6 * 60 * 60;
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;
type PrepareInstall = fn(&tauri::AppHandle) -> Result<Box<dyn Send>, String>;

/// Persist reminder choices only; no downloaded binaries or signing keys live
/// in the application data directory. A skipped version never skips newer ones.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Preferences {
    automatic: bool,
    skipped_version: String,
    notified_version: String,
    remind_after: u64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            skipped_version: String::new(),
            notified_version: String::new(),
            remind_after: 0,
        }
    }
}

/// A revision makes the initial snapshot and streamed progress safe to receive
/// in either order. Metadata is bounded and never includes an executable URL.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    revision: u64,
    current_version: String,
    version: Option<String>,
    notes: String,
    date: String,
    phase: String,
    message: String,
    downloaded: u64,
    total: Option<u64>,
    prompt: bool,
    automatic: bool,
}
struct Inner {
    snapshot: Snapshot,
    preferences: Preferences,
    update: Option<Update>,
}
/// Owns a single operation gate for menu, timer and UI requests. The injected
/// install lease coordinates restart without depending on other feature state.
pub struct UpdateState {
    inner: Mutex<Inner>,
    operation: tokio::sync::Mutex<()>,
    storage: Result<PathBuf, String>,
    prepare_install: PrepareInstall,
}

/// Persist wall-clock seconds across launches; never use the LAN millisecond
/// clock or a process-relative Instant for reminder deadlines.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Stable channel only. SemVer comparison prevents lexical mistakes such as
/// treating 0.1.10 as older than 0.1.9; release candidates never auto-install.
fn newer_stable(current: &Version, next: &Version) -> bool {
    next.pre.is_empty() && next > current
}
/// Reminder suppression is version-specific; a newer release bypasses snooze.
fn should_prompt(p: &Preferences, version: &str, time: u64) -> bool {
    version != p.skipped_version
        && (version != p.notified_version
        || time >= p.remind_after
        // A clock rollback must not suppress reminders indefinitely.
        || p.remind_after.saturating_sub(time) > CHECK_INTERVAL)
}

/// The fixed feed may redirect to GitHub's asset CDN, but it cannot nominate an
/// unrelated site's installer or a different repository's signed package.
fn trusted_download(url: &url::Url, version: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url
            .path()
            .starts_with(&format!("/t0ken-ai/Nobody/releases/download/v{version}/"))
}

/// Atomic replacement avoids losing skip/notification choices if the process
/// exits while writing. Corrupt pre-existing state is never silently replaced.
fn persist(state: &UpdateState, p: &Preferences) -> Result<(), String> {
    let path = state.storage.as_ref().map_err(Clone::clone)?;
    let parent = path.parent().ok_or("更新设置目录不可用。")?;
    crate::private_files::directory(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    crate::private_files::protect(temp.path())?;
    serde_json::to_writer(&mut temp, p).map_err(|e| e.to_string())?;
    temp.flush()
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Emit only to the main webview; the translation popover cannot initiate an
/// update or display release text. Background discovery does not steal focus.
fn change(app: &tauri::AppHandle, action: impl FnOnce(&mut Inner)) {
    let state = app.state::<UpdateState>();
    let (snapshot, newly_prompted) = {
        let mut i = state.inner.lock().unwrap();
        let prompted = i.snapshot.prompt;
        action(&mut i);
        i.snapshot.revision += 1;
        (i.snapshot.clone(), !prompted && i.snapshot.prompt)
    };
    if let Some(window) = app.get_webview_window("main") {
        if newly_prompted {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.emit("main-window-visible", true);
        }
        let _ = window.emit("update-state", snapshot);
    }
}
/// The label comes from Tauri's caller context, never from a frontend argument.
fn require_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("请在 Nobody 主窗口管理更新。".into())
    }
}

/// Startup is nonblocking, and failures stay inside this feature. One sleeping
/// task checks every six hours; no webview polling or animation is required.
pub fn install(app: &tauri::AppHandle, prepare_install: PrepareInstall) {
    let mut storage = app
        .path()
        .home_dir()
        .map(|home| home.join(".translateme/updates/state.json"))
        .map_err(|e| e.to_string());
    let loaded = storage.as_ref().map_err(Clone::clone).and_then(|path| {
        crate::private_files::read(path, 16 * 1024)?
            .map(|bytes| {
                serde_json::from_slice::<Preferences>(&bytes).map_err(|_| {
                    "更新设置损坏，请检查 ~/.translateme/updates/state.json。".to_string()
                })
            })
            .transpose()
    });
    let mut preferences = Preferences::default();
    let mut message = "启动后自动检查，每 6 小时检查一次。".to_string();
    match loaded {
        Ok(Some(p)) => preferences = p,
        Ok(None) => {}
        Err(e) => {
            preferences.automatic = false;
            message = e.clone();
            storage = Err(e);
        }
    }
    let snapshot = Snapshot {
        revision: 0,
        current_version: app.package_info().version.to_string(),
        version: None,
        notes: String::new(),
        date: String::new(),
        phase: "idle".into(),
        message,
        downloaded: 0,
        total: None,
        prompt: false,
        automatic: preferences.automatic,
    };
    app.manage(UpdateState {
        inner: Mutex::new(Inner {
            snapshot,
            preferences,
            update: None,
        }),
        operation: tokio::sync::Mutex::new(()),
        storage,
        prepare_install,
    });
    // Development runs do not notify against real production releases.
    if !cfg!(debug_assertions) {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            loop {
                if handle
                    .state::<UpdateState>()
                    .inner
                    .lock()
                    .unwrap()
                    .preferences
                    .automatic
                {
                    let _ = check(&handle, false).await;
                }
                tokio::time::sleep(Duration::from_secs(CHECK_INTERVAL)).await;
            }
        });
    }
}

/// Native menu and the settings button share the same single-flight check.
pub async fn check(app: &tauri::AppHandle, manual: bool) -> Result<(), String> {
    let state = app.state::<UpdateState>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "更新操作正在进行，请稍候。")?;
    change(app, |i| {
        i.snapshot.phase = "checking".into();
        i.snapshot.message = "正在检查 GitHub 上的正式版本…".into();
        i.snapshot.prompt = manual;
        i.snapshot.version = None;
        i.snapshot.notes.clear();
        i.update = None;
        i.snapshot.downloaded = 0;
        i.snapshot.total = None;
    });
    let result = async {
        app.updater_builder()
            .timeout(Duration::from_secs(20))
            .version_comparator(|current, release| newer_stable(&current, &release.version))
            .build()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| match e {
                tauri_plugin_updater::Error::TargetNotFound(_)
                | tauri_plugin_updater::Error::TargetsNotFound(_) => {
                    "当前平台尚未发布可用更新包。".into()
                }
                _ => e.to_string(),
            })
    }
    .await;
    match result {
        Ok(Some(mut update)) => {
            if !trusted_download(&update.download_url, &update.version) {
                return fail(app, "更新包地址不属于 Nobody 的 GitHub Release。".into());
            }
            update.timeout = Some(Duration::from_secs(5 * 60));
            change(app, |i| {
                let show = if manual {
                    i.snapshot.prompt
                } else {
                    should_prompt(&i.preferences, &update.version, now())
                };
                i.snapshot.version = Some(update.version.clone());
                i.snapshot.notes = update
                    .body
                    .as_deref()
                    .unwrap_or("此版本未提供更新说明。")
                    .chars()
                    .take(24_000)
                    .collect();
                i.snapshot.date = update
                    .date
                    .map(|d| d.date().to_string())
                    .unwrap_or_default();
                i.snapshot.phase = "available".into();
                i.snapshot.message = "新版本已准备好。安装后将重启 Nobody。".into();
                i.snapshot.prompt = show;
                if show {
                    i.preferences.notified_version = update.version.clone();
                    i.preferences.remind_after = now() + CHECK_INTERVAL;
                    // In-memory deduplication still works if local storage is
                    // unwritable; expose that failure without blocking checks.
                    if let Err(e) = persist(&state, &i.preferences) {
                        i.snapshot.message = format!("新版本可用；提醒设置未保存：{e}");
                    }
                }
                i.update = Some(update);
            });
            Ok(())
        }
        Ok(None) => {
            change(app, |i| {
                i.snapshot.phase = "current".into();
                i.snapshot.message = "已经是最新正式版本。".into();
            });
            Ok(())
        }
        Err(e) => fail(
            app,
            format!(
                "暂时无法检查更新，请稍后重试。{}",
                e.chars().take(240).collect::<String>()
            ),
        ),
    }
}
/// Background failures remain quiet; a previously visible prompt shows the
/// error and can retry the retained update object without trusting a UI URL.
fn fail(app: &tauri::AppHandle, error: String) -> Result<(), String> {
    change(app, |i| {
        i.snapshot.phase = "error".into();
        i.snapshot.message = error.clone();
    });
    Err(error)
}

/// Initial state recovers native events sent before the main webview mounted.
#[tauri::command]
pub fn get_update_state(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, UpdateState>,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    Ok(state.inner.lock().unwrap().snapshot.clone())
}
/// Explicit checks bypass skip/snooze, but still share the native operation gate.
#[tauri::command]
pub async fn check_for_updates(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), String> {
    require_main(&window)?;
    check(&app, true).await
}

/// Dismissals never cancel an installation. Skip targets exactly the version
/// visible when the button was pressed, so a delayed click cannot skip a new one.
#[tauri::command]
pub fn dismiss_update(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    version: Option<String>,
    skip: bool,
) -> Result<(), String> {
    require_main(&window)?;
    let state = app.state::<UpdateState>();
    {
        let mut i = state.inner.lock().unwrap();
        if matches!(i.snapshot.phase.as_str(), "downloading" | "installing") {
            return Err("请等待更新操作完成。".into());
        }
        if skip {
            if version.is_none() || version != i.snapshot.version {
                return Err("版本已变化，请重新检查。".into());
            }
            let mut p = i.preferences.clone();
            p.skipped_version = version.unwrap();
            persist(&state, &p)?;
            i.preferences = p;
        } else if let Some(version) = version.filter(|v| Some(v) == i.snapshot.version.as_ref()) {
            // Snooze begins when the reader dismisses, not when a long-lived
            // dialog first appeared. A failed write does not trap them inside it.
            i.preferences.notified_version = version;
            i.preferences.remind_after = now() + CHECK_INTERVAL;
            if let Err(error) = persist(&state, &i.preferences) {
                i.snapshot.message = format!("本次已关闭提醒，但稍后提醒设置未保存：{error}");
            }
        }
    }
    change(&app, |i| i.snapshot.prompt = false);
    Ok(())
}
/// Save immediately and independently of unsaved LLM settings; disabling only
/// affects scheduled checks, leaving the explicit menu action available.
#[tauri::command]
pub fn set_automatic_updates(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    require_main(&window)?;
    let state = app.state::<UpdateState>();
    {
        let mut i = state.inner.lock().unwrap();
        let mut p = i.preferences.clone();
        p.automatic = enabled;
        persist(&state, &p)?;
        i.preferences = p;
    }
    change(&app, |i| i.snapshot.automatic = enabled);
    Ok(())
}

/// Download first so normal work can continue. The signed bytes are verified
/// by Tauri before acquiring the coordinator's idle lease and installing them.
/// Errors release that lease; installation failures are surfaced for retry.
#[tauri::command]
pub async fn install_update(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    version: String,
) -> Result<(), String> {
    require_main(&window)?;
    if cfg!(debug_assertions) {
        return Err("开发模式仅支持检查更新，请使用已安装的正式版本更新。".into());
    }
    let state = app.state::<UpdateState>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "更新操作正在进行，请稍候。")?;
    let update = state
        .inner
        .lock()
        .unwrap()
        .update
        .clone()
        .filter(|u| u.version == version)
        .ok_or("请先检查并选择可用的新版本。")?;
    change(&app, |i| {
        i.snapshot.phase = "downloading".into();
        i.snapshot.prompt = true;
        i.snapshot.message = "正在下载更新，完成后校验签名…".into();
        i.snapshot.downloaded = 0;
        i.snapshot.total = None;
    });
    let cancel = CancellationToken::new();
    let mut downloaded = 0u64;
    let mut last = Instant::now() - Duration::from_secs(1);
    let result = tokio::select! {
        _ = cancel.cancelled() => Err("更新包超过 256 MiB，已停止下载。".to_string()),
        result = update.download(|bytes, total| {
            downloaded = downloaded.saturating_add(bytes as u64);
            if downloaded > MAX_DOWNLOAD || total.is_some_and(|n| n > MAX_DOWNLOAD) { cancel.cancel(); }
            // At most ten small events/second, regardless of network chunk size.
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                change(&app, |i| { i.snapshot.downloaded = downloaded; i.snapshot.total = total; });
            }
        }, || {}) => result.map_err(|e| format!("下载或验签失败：{e}")),
    };
    let bytes = match result {
        Ok(bytes) if !cancel.is_cancelled() => bytes,
        Ok(_) => return fail(&app, "更新包超过允许大小。".into()),
        Err(e) => return fail(&app, e),
    };
    let lease = match (state.prepare_install)(&app) {
        Ok(lease) => lease,
        Err(e) => return fail(&app, e),
    };
    change(&app, |i| {
        i.snapshot.phase = "installing".into();
        i.snapshot.downloaded = bytes.len() as u64;
        i.snapshot.message = "签名已验证，正在安装并重启…".into();
    });
    let result = tokio::task::spawn_blocking(move || (update.install(bytes), lease)).await;
    match result {
        // Keep the idle lease until restart, not just until extraction ends.
        Ok((Ok(()), _lease)) => {
            app.restart();
        }
        Ok((Err(e), _lease)) => fail(&app, format!("安装未完成，请稍后重试：{e}")),
        Err(e) => fail(&app, format!("安装任务未完成：{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_versions_use_semver_and_never_downgrade() {
        let v = |s| Version::parse(s).unwrap();
        assert!(newer_stable(&v("0.1.9"), &v("0.1.10")));
        assert!(!newer_stable(&v("0.1.10"), &v("0.1.9")));
        assert!(!newer_stable(&v("0.1.0"), &v("0.2.0-beta.1")));
        assert!(!newer_stable(&v("0.1.0"), &v("0.1.0")));
    }
    #[test]
    fn skipped_and_snoozed_versions_do_not_hide_new_releases() {
        let mut p = Preferences::default();
        p.skipped_version = "0.1.1".into();
        assert!(!should_prompt(&p, "0.1.1", 100));
        assert!(should_prompt(&p, "0.1.2", 100));
        p.notified_version = "0.1.2".into();
        p.remind_after = 100 + CHECK_INTERVAL;
        assert!(!should_prompt(&p, "0.1.2", 101));
        assert!(should_prompt(&p, "0.1.2", p.remind_after));
        assert!(should_prompt(&p, "0.1.3", 101));
        assert!(should_prompt(&p, "0.1.2", 0));
    }
    #[test]
    fn installer_url_is_bound_to_our_repository_and_version() {
        let valid = "https://github.com/t0ken-ai/Nobody/releases/download/v0.1.1/Nobody.app.tar.gz";
        assert!(trusted_download(&valid.parse().unwrap(), "0.1.1"));
        assert!(!trusted_download(&valid.parse().unwrap(), "0.1.2"));
        for url in [
            valid.replace("https:", "http:"),
            valid.replace("github.com", "github.com.evil.test"),
            valid.replace("t0ken-ai/Nobody", "other/Nobody"),
            format!("{valid}?token=secret"),
        ] {
            assert!(!trusted_download(&url.parse().unwrap(), "0.1.1"));
        }
    }
}
