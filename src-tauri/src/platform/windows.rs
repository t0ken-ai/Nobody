//! Windows UI Automation lives on a dedicated COM thread. COM objects and
//! selection snapshots never cross apartment/thread boundaries.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use windows::Win32::{
    System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        Variant::{VariantClear, VariantToBoolean, VT_BOOL},
    },
    UI::{Accessibility::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

type Job = (Value, oneshot::Sender<Result<Value, String>>);
pub struct Platform {
    sender: mpsc::Sender<Job>,
}
struct Snapshot {
    element: IUIAutomationElement,
    range: Option<IUIAutomationTextRange>,
    document: String,
    selected: String,
    whole: bool,
    pid: u32,
    created: Instant,
}

impl Platform {
    pub fn new(_resources: &Path) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            let init = CoInitializeEx(None, COINIT_MULTITHREADED);
            if init.is_err() {
                let _ = ready_tx.send(Err("无法初始化 Windows 辅助功能。".to_string()));
                return;
            }
            let automation: IUIAutomation =
                match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = ready_tx.send(Err("无法创建 UI Automation。".to_string()));
                        CoUninitialize();
                        return;
                    }
                };
            let _ = ready_tx.send(Ok(()));
            let mut snapshots = HashMap::new();
            for (request, reply) in receiver {
                let _ = reply.send(dispatch(&automation, &mut snapshots, request));
            }
            drop(snapshots);
            drop(automation);
            CoUninitialize();
        });
        ready_rx
            .recv_timeout(Duration::from_secs(8))
            .map_err(|_| "Windows 辅助功能初始化超时。")??;
        Ok(Self { sender })
    }
    pub async fn call(&self, value: Value) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send((value, tx))
            .map_err(|_| "Windows 系统服务已停止。")?;
        tokio::time::timeout(Duration::from_secs(8), rx)
            .await
            .map_err(|_| "目标应用没有响应辅助功能请求。")?
            .map_err(|_| "Windows 系统服务已停止。")?
    }
}

fn failure(_: windows::core::Error) -> String {
    "此控件未提供所需的文字接口。请选中文字重试，或粘贴到翻译工作台。".into()
}

unsafe fn foreground_pid() -> u32 {
    let mut pid = 0;
    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
    pid
}
unsafe fn text_pattern(element: &IUIAutomationElement) -> Option<IUIAutomationTextPattern> {
    element.GetCurrentPatternAs(UIA_TextPatternId).ok()
}
unsafe fn selection(pattern: &IUIAutomationTextPattern) -> Option<IUIAutomationTextRange> {
    let ranges = pattern.GetSelection().ok()?;
    if ranges.Length().ok()? != 1 {
        return None;
    }
    ranges.GetElement(0).ok()
}

unsafe fn capture(
    automation: &IUIAutomation,
    snapshots: &mut HashMap<String, Snapshot>,
    request: &Value,
) -> Result<Value, String> {
    let element = automation.GetFocusedElement().map_err(failure)?;
    let pid = element.CurrentProcessId().map_err(failure)? as u32;
    if pid == std::process::id() || foreground_pid() != pid {
        return Err("请回到要翻译的应用。".into());
    }
    if element.CurrentIsPassword().map_err(failure)?.as_bool() {
        return Err("密码输入框不参与翻译。".into());
    }
    let value: Option<IUIAutomationValuePattern> =
        element.GetCurrentPatternAs(UIA_ValuePatternId).ok();
    let pattern = text_pattern(&element);
    // Browser contenteditable controls often expose TextPattern without
    // ValuePattern; accept them only when UIA explicitly marks text writable.
    let text_writable = pattern
        .as_ref()
        .and_then(|p| p.DocumentRange().ok())
        .and_then(|r| r.GetAttributeValue(UIA_IsReadOnlyAttributeId).ok())
        .map(|mut raw| {
            let result = raw.Anonymous.Anonymous.vt == VT_BOOL
                && VariantToBoolean(&raw).is_ok_and(|v| !v.as_bool());
            let _ = VariantClear(&mut raw);
            result
        })
        .unwrap_or(false);
    let editable = value
        .as_ref()
        .and_then(|v| v.CurrentIsReadOnly().ok())
        .is_some_and(|v| !v.as_bool())
        || text_writable;
    let range = pattern.as_ref().and_then(|p| selection(p));
    let selected = range
        .as_ref()
        .and_then(|r| r.GetText(16001).ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let document = value
        .as_ref()
        .and_then(|v| v.CurrentValue().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            pattern
                .as_ref()?
                .DocumentRange()
                .ok()?
                .GetText(16001)
                .ok()
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    let whole = selected.is_empty() && request["whole"] == true && editable;
    let text = if whole { &document } else { &selected };
    if text.trim().is_empty() {
        return Err("没有选中文字，或当前控件不支持安全读取。".into());
    }
    if text.encode_utf16().count() > 16000 {
        return Err("本次文字超过 16,000 字符，请分段翻译。".into());
    }
    let mut result =
        json!({"text":text,"app":"Windows 应用","pid":pid,"editable":editable,"whole":whole});
    if request["ticket"] == true {
        snapshots.retain(|_, s| s.created.elapsed() < Duration::from_secs(120));
        if snapshots.len() >= 16 {
            snapshots.clear();
        }
        let id = uuid::Uuid::new_v4().to_string();
        snapshots.insert(
            id.clone(),
            Snapshot {
                element,
                range,
                document,
                selected,
                whole,
                pid,
                created: Instant::now(),
            },
        );
        result["ticket"] = json!(id);
    }
    Ok(result)
}

/// Unicode packets replace selected text without touching the clipboard. They
/// never emit Return/Enter key codes. Elevated apps may reject input (UIPI);
/// that is reported as failure, never worked around by elevating TranslateMe.
unsafe fn replace(
    automation: &IUIAutomation,
    snapshots: &mut HashMap<String, Snapshot>,
    request: &Value,
) -> Result<Value, String> {
    let id = request["ticket"].as_str().ok_or("原选区已过期。")?;
    let original = snapshots.remove(id).ok_or("原选区已过期。")?;
    let text = request["text"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("译文为空。")?;
    let current = automation.GetFocusedElement().map_err(failure)?;
    if original.created.elapsed() >= Duration::from_secs(120)
        || foreground_pid() != original.pid
        || !automation
            .CompareElements(&original.element, &current)
            .map_err(failure)?
            .as_bool()
    {
        return Err("输入焦点已变化，请手动复制译文。".into());
    }
    let pattern = text_pattern(&current).ok_or("此输入框不支持安全选择，请手动复制译文。")?;
    let document = pattern.DocumentRange().map_err(failure)?;
    let current_value: Option<IUIAutomationValuePattern> =
        current.GetCurrentPatternAs(UIA_ValuePatternId).ok();
    let value = current_value
        .as_ref()
        .and_then(|v| v.CurrentValue().ok())
        .map(|s| s.to_string())
        .unwrap_or(document.GetText(16001).map_err(failure)?.to_string());
    let current_range = selection(&pattern).ok_or("原选区已变化。")?;
    if value != original.document
        || current_range.GetText(16001).map_err(failure)?.to_string() != original.selected
        || original
            .range
            .as_ref()
            .is_none_or(|r| !r.Compare(&current_range).is_ok_and(|v| v.as_bool()))
    {
        return Err("原文或选区已变化，请手动复制译文。".into());
    }
    for key in [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN] {
        if GetAsyncKeyState(key.0 as i32) < 0 {
            return Err("请松开快捷键后重试；译文已保留。".into());
        }
    }
    if original.whole {
        document.Select().map_err(failure)?;
    }
    let events: Vec<INPUT> = text
        .encode_utf16()
        .flat_map(|unit| {
            [false, true].map(move |up| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: unit,
                        dwFlags: KEYEVENTF_UNICODE
                            | if up {
                                KEYEVENTF_KEYUP
                            } else {
                                KEYBD_EVENT_FLAGS(0)
                            },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            })
        })
        .collect();
    if SendInput(&events, std::mem::size_of::<INPUT>() as i32) as usize != events.len() {
        return Err("Windows 拒绝了部分输入，请检查原输入框并手动复制译文。".into());
    }
    // Providers do not consistently expose post-input text synchronously. The
    // shared UI accurately reports an attempted, not verified, replacement.
    Ok(json!({"ok":true,"confirmed":false}))
}

unsafe fn dispatch(
    automation: &IUIAutomation,
    snapshots: &mut HashMap<String, Snapshot>,
    request: Value,
) -> Result<Value, String> {
    match request["op"].as_str().unwrap_or("") {
        "status" | "permission" => {
            Ok(json!({"accessibility":true,"systemTranslation":false,"platform":"Windows"}))
        }
        "selection" | "capture" => capture(automation, snapshots, &request),
        "replace" => replace(automation, snapshots, &request),
        "copy" => {
            arboard::Clipboard::new()
                .map_err(|e| e.to_string())?
                .set_text(request["text"].as_str().unwrap_or(""))
                .map_err(|e| e.to_string())?;
            Ok(json!({"ok":true}))
        }
        "translate" => {
            Err("Windows 暂未配置默认翻译引擎。请在设置中选择 LLM 并填写服务地址和模型。".into())
        }
        _ => Err("未知的系统操作。".into()),
    }
}
