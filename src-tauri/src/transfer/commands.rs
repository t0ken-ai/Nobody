//! Thin Tauri adapter. Every LAN command requires the main window; the result
//! popover cannot initiate sends, choose files or change device trust.
use super::{protocol::FileMeta, transport, Service, Snapshot, TransferService};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// Keep initialization errors queryable and serialize the two native picker types.
pub struct LanState {
    service: Result<Service, String>,
    picker: AtomicBool,
}
/// LAN startup errors are isolated in the feature state, never propagated out
/// of Tauri setup (which would take down working translation functionality).
pub fn install(app: &tauri::AppHandle) {
    let handle = app.clone();
    let events = Arc::new(move |kind: &str| {
        if let Some(window) = handle.get_webview_window("main") {
            if kind == "pairing" {
                let _ = window.show();
                // Pairing can show without focusing. Wake the view explicitly
                // so a paused hidden page still presents the trust challenge.
                let _ = window.emit("main-window-visible", true);
            }
            let _ = window.emit("transfer-event", kind);
        }
        if kind == "received" {
            if let Some(tray) = handle.tray_by_id("main-tray") {
                let _ = tray.set_tooltip(Some("Nobody · 收到新内容"));
            }
            // OS notification policy can suppress this; inbox delivery never
            // depends on notification permissions or the notification result.
            let _ = handle
                .notification()
                .builder()
                .title("Nobody · 收到新内容")
                .body("打开局域网互传，查看收到的文字或文件。")
                .show();
        }
    });
    let service = (|| {
        let legacy = app
            .path()
            .app_config_dir()
            .map_err(|e| e.to_string())?
            .join("transfer");
        let home = app.path().home_dir().map_err(|e| e.to_string())?;
        // This is the pre-Nobody migration source, not a display name. Keep it
        // unchanged so upgrades still recognize the old default receive folder.
        let old_default = app
            .path()
            .download_dir()
            .ok()
            .map(|path| path.join("TranslateMe"));
        let (dir, received) =
            super::store::prepare_location(&home, &legacy, old_default.as_deref())?;
        TransferService::new(dir, received, events)
    })();
    app.manage(LanState {
        service: service.clone(),
        picker: AtomicBool::new(false),
    });
    if let Ok(service) = service {
        tauri::async_runtime::spawn(async move {
            service.start().await;
        });
    }
}
/// Called on actual process exit, not when hiding either window.
pub fn shutdown(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<LanState>() {
        if let Ok(service) = &state.service {
            service.stop();
        }
    }
}
/// Main-window guard is repeated server-side, independent of hidden frontend controls.
fn main_service(window: &tauri::WebviewWindow, state: &LanState) -> Result<Service, String> {
    if window.label() != "main" {
        return Err("请在主窗口使用局域网互传。".into());
    }
    state.service.clone()
}
/// Return bounded UI state; networking continues when the page is hidden.
#[tauri::command]
pub fn get_transfer_state(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
) -> Result<Snapshot, String> {
    Ok(main_service(&window, &state)?.snapshot())
}
/// Commit LAN-only preferences and reconcile discovery/listener lifecycle.
#[tauri::command]
pub async fn save_transfer_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    enabled: bool,
    name: String,
    receive_dir: PathBuf,
) -> Result<(), String> {
    main_service(&window, &state)?
        .configure(enabled, name, receive_dir)
        .await
}
/// Explicitly restart discovery when idle; initialization errors stay visible.
#[tauri::command]
pub async fn retry_transfer_service(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
) -> Result<(), String> {
    main_service(&window, &state)?.retry().await
}
/// Start a job for a discovered identity and native file paths, returning its record id.
/// This command must be async: a synchronous Tauri command runs on the UI thread,
/// outside Tokio, where scheduling the network job previously panicked on Send.
#[tauri::command]
pub async fn send_transfer(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    peer_id: String,
    text: String,
    paths: Vec<PathBuf>,
) -> Result<String, String> {
    main_service(&window, &state)?.send(peer_id, text, paths)
}
/// Answer one current session challenge; stale prompt ids cannot grant trust.
#[tauri::command]
pub fn answer_transfer_pairing(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    id: String,
    accepted: bool,
) -> Result<(), String> {
    main_service(&window, &state)?.decide(&id, accepted)
}
/// Revoke durable trust and cancel that identity's in-flight jobs.
#[tauri::command]
pub fn forget_transfer_peer(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    id: String,
) -> Result<(), String> {
    main_service(&window, &state)?.forget(&id)
}
/// Request cooperative cancellation; the transport owns cleanup of staged files.
#[tauri::command]
pub fn cancel_transfer(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    id: String,
) -> Result<(), String> {
    main_service(&window, &state)?.cancel(&id)
}
/// Fetch complete text on explicit read/copy, avoiding large progress snapshots.
#[tauri::command]
pub fn read_transfer_text(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    id: String,
) -> Result<String, String> {
    main_service(&window, &state)?.text(&id)
}
/// Clear ended history only; received files remain in the chosen directory.
#[tauri::command]
pub fn clear_transfer_records(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
) -> Result<(), String> {
    main_service(&window, &state)?.clear()
}
/// Reveal only paths belonging to completed incoming records; never invoke a
/// shell or automatically execute a received file based on peer input.
#[tauri::command]
pub fn reveal_transfer_file(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    id: String,
    index: usize,
) -> Result<(), String> {
    let path = main_service(&window, &state)?.received_path(&id, index)?;
    if !path.exists() {
        return Err("文件已被移动或删除。".into());
    }
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}
/// Opening the inbox clears the tray hint without changing records or OS notifications.
#[tauri::command]
pub fn mark_transfer_seen(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
) -> Result<(), String> {
    main_service(&window, &state)?;
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_tooltip(Some("Nobody · 写英文，读母语"))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
/// Native selections carry only local paths and validated metadata through IPC.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    path: PathBuf,
    name: String,
    size: u64,
}
/// Share ordinary-file and filename validation between the chooser and drag/drop.
fn picked(paths: Vec<PathBuf>) -> Result<Vec<PickedFile>, String> {
    let meta: Vec<FileMeta> = transport::inspect_files(&paths)?;
    Ok(paths
        .into_iter()
        .zip(meta)
        .map(|(path, m)| PickedFile {
            path,
            name: m.name,
            size: m.size,
        })
        .collect())
}
/// Drag/drop paths go through exactly the same native validation as the chooser.
#[tauri::command]
pub fn inspect_transfer_files(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
    paths: Vec<PathBuf>,
) -> Result<Vec<PickedFile>, String> {
    main_service(&window, &state)?;
    picked(paths)
}
/// Release the picker reservation on cancellation, errors and normal completion.
struct PickerGuard<'a>(&'a AtomicBool);
impl Drop for PickerGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
/// Native dialog callback avoids blocking the UI thread. A shared guard prevents
/// overlapping file/folder panels while a user is deciding what to send.
#[tauri::command]
pub async fn pick_transfer_files(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
) -> Result<Vec<PickedFile>, String> {
    main_service(&window, &state)?;
    state
        .picker
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "文件选择器已经打开。")?;
    let _guard = PickerGuard(&state.picker);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = app.dialog().file().set_title("选择要发送的文件");
    // NSOpenPanel can remember a prior app-only filter. Explicitly enable all
    // item UTIs on macOS; ordinary-file validation below still rejects folders.
    #[cfg(target_os = "macos")]
    let dialog = dialog.add_filter("所有文件", &["public.item"]);
    dialog.pick_files(move |files| {
        let _ = tx.send(files);
    });
    let paths = rx
        .await
        .map_err(|_| "文件选择器已关闭。")?
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.into_path().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    picked(paths)
}
/// Folder selection changes only the draft; save_transfer_settings commits it.
#[tauri::command]
pub async fn pick_transfer_directory(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, LanState>,
) -> Result<Option<PathBuf>, String> {
    let service = main_service(&window, &state)?;
    state
        .picker
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "文件选择器已经打开。")?;
    let _guard = PickerGuard(&state.picker);
    let path = service.inner.lock().unwrap().settings.receive_dir.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("选择文件接收目录")
        .set_directory(path)
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    rx.await
        .map_err(|_| "目录选择器已关闭。")?
        .map(|p| p.into_path().map_err(|e| e.to_string()))
        .transpose()
}
