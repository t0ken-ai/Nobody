#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Application coordinator: owns preferences and in-flight work. Platform and
//! engine modules never call each other or reach into the UI.
mod config;
mod document;
mod llm_store;
mod platform;
mod popover;
mod selection;
mod transfer;
mod translation;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

struct AppState {
    settings: Mutex<config::Settings>,
    settings_path: PathBuf,
    platform: platform::Platform,
    http: reqwest::Client,
    busy: AtomicBool,
    picking_app: AtomicBool,
    generation: AtomicU64,
    last: Mutex<Option<TranslationResult>>,
    popover: Mutex<popover::Popover>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranslationResult {
    source: String,
    text: String,
    target: String,
    engine: String,
    origin: String,
    message: String,
    replaced: bool,
}

/// RAII releases the single-flight gate on every error path and cancellation.
struct BusyGuard<'a>(&'a AtomicBool);
impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
fn start(state: &AppState) -> Result<BusyGuard<'_>, String> {
    state
        .busy
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "正在翻译，请稍候。")?;
    Ok(BusyGuard(&state.busy))
}
fn preferences(state: &AppState) -> config::Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
async fn get_settings(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let settings = preferences(&state);
    let mut status = state.platform.call(json!({"op":"status"})).await?;
    status["settings"] = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    status["defaultLlmPrompt"] = json!(config::DEFAULT_LLM_PROMPT);
    Ok(status)
}

/// Exercise the unsaved LLM draft through the production request/validation
/// path. Do not publish a translation event or persist the draft/key on success.
#[tauri::command]
async fn test_llm_connection(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    settings: config::Settings,
    api_key: Option<String>,
) -> Result<translation::ConnectionTest, String> {
    if window.label() != "main" {
        return Err("请在偏好设置中测试连接。".into());
    }
    let _busy = start(&state)?;
    translation::test_connection(&state.http, &settings, api_key).await
}

/// Only the main settings window may open a chooser. Its own gate prevents
/// overlapping panels without holding the translation lock while a user browses.
#[tauri::command]
async fn pick_applications(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("请在偏好设置中添加应用。".into());
    }
    state
        .picking_app
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "应用选择器已经打开。")?;
    let _guard = BusyGuard(&state.picking_app);
    let request = json!({"op":"pickApplications"});
    #[cfg(target_os = "windows")]
    let request = {
        let mut request = request;
        request["owner"] = json!(window.hwnd().map_err(|e| e.to_string())?.0 as usize);
        request
    };
    state.platform.call(request).await
}

/// Read presentation-only icons for the settings draft, including unsaved picker
/// entries. Native adapters revalidate local paths; icons never enter preferences
/// or participate in application identity / automatic-selection authorization.
#[tauri::command]
async fn get_application_icons(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
    apps: Vec<config::Application>,
) -> Result<Value, String> {
    if window.label() != "main" || apps.len() > 100 {
        return Err("请在偏好设置中读取应用图标。".into());
    }
    state
        .platform
        .call(json!({"op":"applicationIcons", "apps":apps}))
        .await
}

fn parse_shortcuts(s: &config::Settings) -> Result<(Shortcut, Shortcut), String> {
    let write: Shortcut = s
        .write_shortcut
        .parse()
        .map_err(|_| "写入快捷键格式无效。")?;
    let read: Shortcut = s
        .read_shortcut
        .parse()
        .map_err(|_| "阅读快捷键格式无效。")?;
    if write == read {
        return Err("两个快捷键不能相同。".into());
    }
    // Require a modifier to avoid hijacking normal typing across other apps.
    if write.mods.is_empty() || read.mods.is_empty() {
        return Err("快捷键至少需要一个修饰键。".into());
    }
    Ok((write, read))
}

#[tauri::command]
async fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    settings: config::Settings,
    api_key: Option<String>,
) -> Result<(), String> {
    config::validate(&settings)?;
    state
        .platform
        .call(json!({"op":"validateApplications", "apps":settings.automatic_apps}))
        .await?;
    let (write, read) = parse_shortcuts(&settings)?;
    let previous = preferences(&state);
    // Reject changes during translation so an in-flight job cannot target a
    // newly selected provider, key, language or shortcut configuration.
    let _busy = start(&state)?;
    let shortcuts_changed = settings.write_shortcut != previous.write_shortcut
        || settings.read_shortcut != previous.read_shortcut;
    if shortcuts_changed {
        app.global_shortcut()
            .unregister_all()
            .map_err(|e| e.to_string())?;
        if let Err(error) = app.global_shortcut().register_multiple([write, read]) {
            // Registration may fail after the first key succeeds. Remove that
            // partial registration before restoring the complete previous pair.
            let _ = app.global_shortcut().unregister_all();
            if let Ok((a, b)) = parse_shortcuts(&previous) {
                let _ = app.global_shortcut().register_multiple([a, b]);
            }
            return Err(format!("快捷键注册失败，已恢复原快捷键：{error}"));
        }
    }
    // Keychain and SQLCipher KDF/I/O are blocking; keep them off the UI executor.
    let path = state.settings_path.clone();
    let next = settings.clone();
    let old = previous.clone();
    let saved =
        tauri::async_runtime::spawn_blocking(move || config::persist(&path, &next, &old, api_key))
            .await
            .map_err(|_| "LLM 配置保存任务中断。".to_string())
            .and_then(|r| r);
    if let Err(error) = saved {
        if shortcuts_changed {
            let _ = app.global_shortcut().unregister_all();
            if let Ok((a, b)) = parse_shortcuts(&previous) {
                let _ = app.global_shortcut().register_multiple([a, b]);
            }
        }
        return Err(error);
    }
    *state.settings.lock().unwrap() = settings;
    state.generation.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
async fn request_permission(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    state.platform.call(json!({"op":"permission"})).await
}
#[tauri::command]
async fn copy_text(state: tauri::State<'_, AppState>, text: String) -> Result<(), String> {
    state
        .platform
        .call(json!({"op":"copy","text":text}))
        .await?;
    Ok(())
}
#[tauri::command]
fn get_last_result(state: tauri::State<'_, AppState>) -> Option<TranslationResult> {
    state.last.lock().unwrap().clone()
}

/// Window state is computed under its own mutex; platform/UI calls happen only
/// after release, keeping the polling worker independent of the main thread.
fn sync_popover(app: &tauri::AppHandle, state: &AppState) {
    let update = state.popover.lock().unwrap().update();
    popover::apply(app, update);
}

/// Only the result webview may drag, dismiss or resize its own popover. Pinning
/// precedes the native drag loop so a concurrent selection poll cannot snap it.
#[tauri::command]
fn drag_popover(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if window.label() != "result" {
        return Err("只允许拖动译文浮窗。".into());
    }
    state.popover.lock().unwrap().pin();
    window.start_dragging().map_err(|e| e.to_string())
}
#[tauri::command]
fn dismiss_popover(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if window.label() != "result" {
        return Err("只允许关闭译文浮窗。".into());
    }
    state.popover.lock().unwrap().dismiss();
    sync_popover(window.app_handle(), &state);
    window.hide().map_err(|e| e.to_string())
}
/// The frontend measures content, while the backend clamps height and applies
/// screen geometry. Long translations scroll instead of covering the display.
#[tauri::command]
fn resize_popover(window: tauri::WebviewWindow, state: tauri::State<'_, AppState>, height: f64) {
    if window.label() != "result" {
        return;
    }
    state.popover.lock().unwrap().resize(height);
    sync_popover(window.app_handle(), &state);
}

#[tauri::command]
async fn translate_text(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
    target: String,
) -> Result<TranslationResult, String> {
    let _busy = start(&state)?;
    let s = preferences(&state);
    let translated =
        translation::translate(&state.platform, &state.http, &s, &text, &target).await?;
    let result = TranslationResult {
        source: text,
        text: translated,
        target,
        engine: s.engine,
        origin: "翻译工作台".into(),
        message: "翻译完成".into(),
        replaced: false,
    };
    publish(&app, &state, &result, false);
    Ok(result)
}

/// Workbench results update only the workbench. A live selection's popover must
/// not silently switch to unrelated text translated in the main window.
fn publish(app: &tauri::AppHandle, state: &AppState, result: &TranslationResult, popup: bool) {
    *state.last.lock().unwrap() = Some(result.clone());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("translation-result", result);
    }
    if popup {
        state.popover.lock().unwrap().present(true);
        if let Some(window) = app.get_webview_window("result") {
            let _ = window.emit("translation-result", result);
        }
        sync_popover(app, state);
    }
}

/// Capturing happens before any window is shown. Only an explicit write
/// shortcut carries a replacement ticket; auto-selection can only show text.
async fn translate_selection(
    app: tauri::AppHandle,
    write: bool,
    captured: Option<Value>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _busy = start(&state)?;
    let s = preferences(&state);
    let generation = state.generation.load(Ordering::SeqCst);
    let capture = match captured {
        Some(v) => v,
        None => {
            state
                .platform
                .call(json!({"op":"capture", "whole":write, "ticket":write, "geometry":true}))
                .await?
        }
    };
    let source = capture["text"]
        .as_str()
        .ok_or("没有可翻译的文字。")?
        .to_string();
    let target = if write { "en" } else { &s.target_language };
    // Capture the anchor before the user moves the pointer. A translation that
    // finishes after another selection must never appear over the new text.
    let anchor = popover::initial_anchor(&app, &capture);
    let token = state.popover.lock().unwrap().begin(
        popover::identity(&capture, &s.target_language),
        anchor,
        Some(&capture),
    );
    if write {
        state.popover.lock().unwrap().present(false);
        sync_popover(&app, &state);
    }
    if !write {
        if let Some(window) = app.get_webview_window("result") {
            let _ = window.emit(
                "translation-progress",
                json!({"message":"正在翻译…", "target":target}),
            );
        }
        sync_popover(&app, &state);
    }
    let translated =
        match translation::translate(&state.platform, &state.http, &s, &source, target).await {
            Ok(text) => text,
            Err(error) => {
                if state.popover.lock().unwrap().current(token) {
                    report_error(&app, error, true);
                }
                return Ok(());
            }
        };
    if state.generation.load(Ordering::SeqCst) != generation {
        return Ok(());
    }
    if !state.popover.lock().unwrap().current(token) {
        return Ok(());
    }
    let mut result = TranslationResult {
        source,
        text: translated,
        target: target.into(),
        engine: s.engine,
        origin: capture["app"].as_str().unwrap_or("当前应用").into(),
        message: "翻译完成".into(),
        replaced: false,
    };
    if write {
        if result.text == result.source {
            result.message = "内容无需翻译，原文已保留。".into();
        } else if capture["editable"] != true {
            result.message = "当前区域不可编辑，请复制译文。".into();
        } else {
            match state
                .platform
                .call(json!({"op":"replace", "ticket":capture["ticket"], "text":result.text}))
                .await
            {
                Ok(response) => {
                    result.replaced = response["confirmed"] == true;
                    result.message = if result.replaced {
                        "已替换为英文 · 可在原输入框撤销"
                    } else {
                        "已尝试回填，请检查原输入框；未自动发送。"
                    }
                    .into();
                }
                Err(message) => {
                    result.message = message;
                }
            }
        }
    }
    let popup = !result.replaced;
    if !popup {
        state.popover.lock().unwrap().clear();
        sync_popover(&app, &state);
    }
    publish(&app, &state, &result, popup);
    Ok(())
}

fn report_error(app: &tauri::AppHandle, error: String, popup: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("translation-error", &error);
    }
    if popup {
        let state = app.state::<AppState>();
        if !state.popover.lock().unwrap().active() {
            let anchor = popover::pointer_anchor(app);
            state
                .popover
                .lock()
                .unwrap()
                .begin("error".into(), anchor, None);
        }
        state.popover.lock().unwrap().present(true);
        if let Some(window) = app.get_webview_window("result") {
            let _ = window.emit("translation-error", &error);
        }
        sync_popover(app, &state);
    }
}

/// Observe only allowlisted apps or the exact source of an existing result.
/// Native adapters report mouse metadata and control context; the shared gate
/// requires a fresh completed selection before debouncing for 700 ms. Manual
/// result tracking remains independent of the automatic eligibility policy.
fn watch_selection(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut gate = selection::Gate::default();
        let mut generation = 0;
        let mut selection_settings = None;
        let mut native_configured = false;
        loop {
            tokio::time::sleep(Duration::from_millis(350)).await;
            let state = app.state::<AppState>();
            let current_generation = state.generation.load(Ordering::SeqCst);
            // The idle loop needs only selection policy. Do not repeatedly
            // clone LLM prompts/endpoints or serialize the whole settings form.
            if selection_settings.is_none() || generation != current_generation {
                let s = state.settings.lock().unwrap();
                let allowed = if s.auto_selection {
                    json!(s.automatic_apps)
                } else {
                    json!([])
                };
                selection_settings = Some((s.auto_selection, s.target_language.clone(), allowed));
                native_configured = false;
            }
            if generation != current_generation {
                gate.clear();
                state.popover.lock().unwrap().clear();
                sync_popover(&app, &state);
                generation = current_generation;
            }
            if state.busy.load(Ordering::SeqCst) && state.popover.lock().unwrap().waiting_to_write()
            {
                gate.clear();
                continue;
            }
            let tracking = state.popover.lock().unwrap().tracking();
            let (automatic, target, allowed) = selection_settings.as_ref().unwrap();
            // Send a disabled allowlist once so native monitors stop, then skip
            // bridge work unless a manual result still needs source tracking.
            if !automatic && tracking.is_null() && native_configured {
                gate.clear();
                continue;
            }
            let selected = state
                .platform
                .call(json!({"op":"selection", "whole":false,
                "ticket":false, "geometry":true, "allowedApps":allowed,
                "tracking":tracking, "poll":gate.poll_hint()}))
                .await;
            native_configured = selected.is_ok();
            let Ok(selected) = selected else {
                gate.clear();
                state.popover.lock().unwrap().selection_lost();
                sync_popover(&app, &state);
                continue;
            };
            // A click/drag on our own result is not a new source selection.
            if selected["self"] == true {
                gate.clear();
                continue;
            }
            let identity = popover::identity(&selected, target);
            let text = selected["text"].as_str().unwrap_or("");
            if text.trim().is_empty() || selected["ignored"] == true {
                // Consume an ignored gesture before clearing its UI, so moving
                // from a filename/input to a body cannot reuse that gesture.
                gate.observe(&selected, &identity, std::time::Instant::now(), true);
                // Metadata-only idle responses must not dismiss an explicit
                // diagnostic that has no captured source to follow.
                if selected["reason"] == "idle" {
                    state.popover.lock().unwrap().selection_lost();
                } else {
                    state.popover.lock().unwrap().clear();
                }
                sync_popover(&app, &state);
                continue;
            }
            let anchor = popover::anchor(&app, &selected);
            state.popover.lock().unwrap().observe(&identity, anchor);
            sync_popover(&app, &state);
            if !automatic {
                gate.clear();
                continue;
            }
            if !gate.observe(
                &selected,
                &identity,
                std::time::Instant::now(),
                state.busy.load(Ordering::SeqCst),
            ) {
                continue;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = translate_selection(app.clone(), false, Some(selected)).await {
                    // An automatic request must not resurrect a popup after a
                    // context change or a race with an explicit shortcut.
                    report_error(&app, error, false);
                }
            });
        }
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let s = preferences(&app.state::<AppState>());
                    if let Ok((write, read)) = parse_shortcuts(&s) {
                        let is_write = *shortcut == write;
                        if !is_write && *shortcut != read {
                            return;
                        }
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                translate_selection(app.clone(), is_write, None).await
                            {
                                report_error(&app, error, true);
                            }
                        });
                    }
                })
                .build(),
        )
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            let settings = config::load_settings(&path).map_err(std::io::Error::other)?;
            let native = platform::Platform::new(&app.path().resource_dir()?)
                .map_err(std::io::Error::other)?;
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(55))
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            app.manage(AppState {
                settings: Mutex::new(settings.clone()),
                settings_path: path,
                platform: native,
                http,
                busy: AtomicBool::new(false),
                picking_app: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                last: Mutex::new(None),
                popover: Mutex::new(popover::Popover::default()),
            });
            let (write, read) = parse_shortcuts(&settings).map_err(std::io::Error::other)?;
            app.global_shortcut().register_multiple([write, read])?;
            let open = tauri::menu::MenuItem::with_id(
                app,
                "open",
                "打开 TranslateMe",
                true,
                None::<&str>,
            )?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&open, &quit])?;
            tauri::tray::TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("TranslateMe · 写英文，读母语")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<AppState>();
                    // The adapter configures only the already-created result
                    // window. Complete this before observing other applications.
                    if let Err(error) = state.platform.call(json!({"op":"configurePopover"})).await
                    {
                        report_error(&handle, error, false);
                    }
                    watch_selection(handle.clone());
                });
            }
            #[cfg(not(target_os = "macos"))]
            watch_selection(app.handle().clone());
            // LAN state and failures stay separate from translation state.
            transfer::commands::install(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Explicit visibility complements WebKit/WebView2 document state:
            // closing to the tray pauses view work, never the native services.
            if window.label() == "main"
                && matches!(
                    event,
                    tauri::WindowEvent::Focused(_) | tauri::WindowEvent::Resized(_)
                )
            {
                let visible =
                    window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false);
                let _ = window.emit("main-window-visible", visible);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "result" {
                    window.state::<AppState>().popover.lock().unwrap().dismiss();
                }
                if window.label() == "main" {
                    let _ = window.emit("main-window-visible", false);
                }
                let _ = window.hide();
                #[cfg(target_os = "macos")]
                if window.label() == "main" {
                    // The native adapter rechecks visibility after an idle
                    // delay. Reopening promptly must not trigger allocator work.
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = app
                            .state::<AppState>()
                            .platform
                            .call(json!({"op":"background"}))
                            .await;
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            test_llm_connection,
            pick_applications,
            get_application_icons,
            request_permission,
            translate_text,
            copy_text,
            get_last_result,
            drag_popover,
            dismiss_popover,
            resize_popover,
            transfer::commands::get_transfer_state,
            transfer::commands::save_transfer_settings,
            transfer::commands::retry_transfer_service,
            transfer::commands::send_transfer,
            transfer::commands::answer_transfer_pairing,
            transfer::commands::forget_transfer_peer,
            transfer::commands::cancel_transfer,
            transfer::commands::read_transfer_text,
            transfer::commands::clear_transfer_records,
            transfer::commands::reveal_transfer_file,
            transfer::commands::mark_transfer_seen,
            transfer::commands::inspect_transfer_files,
            transfer::commands::pick_transfer_files,
            transfer::commands::pick_transfer_directory
        ])
        .build(tauri::generate_context!())
        .expect("TranslateMe could not start")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                transfer::commands::shutdown(app);
            }
        });
}
