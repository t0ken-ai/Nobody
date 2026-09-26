#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Application coordinator: owns preferences and in-flight work. Platform and
//! engine modules never call each other or reach into the UI.
mod config;
mod document;
mod platform;
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
    generation: AtomicU64,
    last: Mutex<Option<TranslationResult>>,
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
    Ok(status)
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
    let (write, read) = parse_shortcuts(&settings)?;
    let previous = preferences(&state);
    // Reject changes during translation so an in-flight job cannot target a
    // newly selected provider, key, language or shortcut configuration.
    let _busy = start(&state)?;
    if let Some(key) = api_key {
        let endpoint = settings.endpoint.clone();
        tauri::async_runtime::spawn_blocking(move || config::set_key(&endpoint, key.trim()))
            .await
            .map_err(|e| e.to_string())??;
    }
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
    if let Err(error) = config::write(&state.settings_path, &settings) {
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

fn publish(app: &tauri::AppHandle, state: &AppState, result: &TranslationResult, popup: bool) {
    *state.last.lock().unwrap() = Some(result.clone());
    let _ = app.emit("translation-result", result);
    if popup {
        if let Some(window) = app.get_webview_window("result") {
            // Showing without focus preserves the source app's selection.
            let _ = window.show();
        }
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
                .call(json!({"op":"capture", "whole":write, "ticket":write}))
                .await?
        }
    };
    let source = capture["text"]
        .as_str()
        .ok_or("没有可翻译的文字。")?
        .to_string();
    let target = if write { "en" } else { &s.target_language };
    let _ = app.emit("translation-progress", json!({"message":"正在翻译…"}));
    let translated =
        translation::translate(&state.platform, &state.http, &s, &source, target).await?;
    if state.generation.load(Ordering::SeqCst) != generation {
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
    publish(&app, &state, &result, popup);
    Ok(())
}

fn report_error(app: &tauri::AppHandle, error: String, popup: bool) {
    let _ = app.emit("translation-error", &error);
    if popup {
        if let Some(window) = app.get_webview_window("result") {
            let _ = window.show();
        }
    }
}

/// Debounce a stable selection for 700 ms and translate it once. Polling reads
/// only the focused accessibility element; no global keystroke/mouse recording.
fn watch_selection(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut candidate = String::new();
        let mut handled = String::new();
        let mut stable_since = std::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(350)).await;
            let state = app.state::<AppState>();
            let s = preferences(&state);
            if !s.auto_selection || state.busy.load(Ordering::SeqCst) {
                candidate.clear();
                continue;
            }
            let Ok(selected) = state
                .platform
                .call(json!({"op":"selection", "whole":false, "ticket":false}))
                .await
            else {
                candidate.clear();
                handled.clear();
                continue;
            };
            let text = selected["text"].as_str().unwrap_or("");
            if text.trim().chars().count() < 2 {
                continue;
            }
            let identity = format!("{}:{}:{}", selected["pid"], s.target_language, text);
            if candidate != identity {
                candidate = identity.clone();
                stable_since = std::time::Instant::now();
                continue;
            }
            if identity == handled || stable_since.elapsed() < Duration::from_millis(700) {
                continue;
            }
            handled = identity;
            if let Err(error) = translate_selection(app.clone(), false, Some(selected)).await {
                report_error(&app, error, true);
            }
        }
    });
}

fn main() {
    tauri::Builder::default()
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
            let settings = config::read(&path).map_err(std::io::Error::other)?;
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
                generation: AtomicU64::new(0),
                last: Mutex::new(None),
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
            tauri::tray::TrayIconBuilder::new()
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
            watch_selection(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            request_permission,
            translate_text,
            copy_text,
            get_last_result
        ])
        .run(tauri::generate_context!())
        .expect("TranslateMe could not start");
}
