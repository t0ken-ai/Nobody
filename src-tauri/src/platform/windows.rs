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
    Foundation::CloseHandle,
    System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        Ole::{
            SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetElement, SafeArrayGetElemsize,
            SafeArrayGetLBound, SafeArrayGetUBound,
        },
        Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
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
            let mut mouse = SelectionMouse::default();
            loop {
                // Sample only mouse button/position metadata between COM jobs.
                // Very short gestures missed during a slow provider call fail
                // closed; keyboard contents and window text are never polled.
                mouse.poll();
                match receiver.recv_timeout(Duration::from_millis(16)) {
                    Ok((request, reply)) => {
                        let _ =
                            reply.send(dispatch(&automation, &mut snapshots, &mut mouse, request));
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
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

/// Executable identity is resolved from the foreground PID, never its window
/// title. Unknown/renamed hosts are excluded, including every terminal host.
unsafe fn product_key(pid: u32) -> Option<&'static str> {
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    let result = QueryFullProcessImageNameW(
        process,
        PROCESS_NAME_WIN32,
        windows::core::PWSTR(path.as_mut_ptr()),
        &mut length,
    );
    let _ = CloseHandle(process);
    result.ok()?;
    let path = String::from_utf16_lossy(&path[..length as usize]);
    match path.rsplit('\\').next()?.to_ascii_lowercase().as_str() {
        "codex.exe" => Some("codex"),
        "claude.exe" => Some("claude"),
        _ => None,
    }
}

#[derive(Default)]
struct SelectionMouse {
    allowed: Vec<String>,
    held: bool,
    start: Option<(u32, i32, i32, bool)>,
    dragged: bool,
    last_click: Option<(u32, i32, i32, Instant)>,
    released: Option<(u32, Instant)>,
    sequence: u64,
}
impl SelectionMouse {
    /// Small jitter is not selection intent; a double click uses the system's
    /// timing/distance. A release must remain in the same allowlisted process.
    unsafe fn poll(&mut self) {
        let held = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
        if !held && !self.held {
            return;
        }
        let pid = foreground_pid();
        let mut point = windows::Win32::Foundation::POINT::default();
        if GetCursorPos(&mut point).is_err() {
            self.start = None;
            self.held = held;
            return;
        }
        if held && !self.held {
            let allowed = product_key(pid).is_some_and(|key| self.allowed.iter().any(|a| a == key));
            let double = self.last_click.is_some_and(|(p, x, y, t)| {
                p == pid
                    && t.elapsed() <= Duration::from_millis(GetDoubleClickTime() as u64)
                    && (x - point.x).abs() <= GetSystemMetrics(SM_CXDOUBLECLK)
                    && (y - point.y).abs() <= GetSystemMetrics(SM_CYDOUBLECLK)
            });
            self.start = allowed.then_some((pid, point.x, point.y, double));
            self.dragged = false;
        }
        if let Some((start_pid, x, y, double)) = self.start {
            self.dragged |= (x - point.x).abs() >= 4 || (y - point.y).abs() >= 4;
            if !held {
                if start_pid == pid && (self.dragged || double) {
                    self.sequence += 1;
                    self.released = Some((pid, Instant::now()));
                }
                self.last_click = (!self.dragged && start_pid == pid).then_some((
                    pid,
                    point.x,
                    point.y,
                    Instant::now(),
                ));
                self.start = None;
            }
        }
        self.held = held;
    }
    fn annotate(&self, capture: &mut Value, pid: u32) {
        capture["mouseDown"] = json!(self.held);
        if let Some((source, time)) = self.released.filter(|(source, _)| *source == pid) {
            capture["gestureId"] = json!(format!("{source}:{}", self.sequence));
            capture["gestureAgeMs"] = json!(time.elapsed().as_millis() as u64);
        }
    }
}

/// Runtime IDs distinguish two controls holding identical text. Own and free
/// the SAFEARRAY immediately; no UIA object or pointer crosses the COM thread.
unsafe fn context_id(element: &IUIAutomationElement, pid: u32) -> Option<String> {
    let array = element.GetRuntimeId().ok()?;
    if array.is_null() {
        return None;
    }
    let result = (|| {
        if SafeArrayGetDim(array) != 1 || SafeArrayGetElemsize(array) != 4 {
            return None;
        }
        let first = SafeArrayGetLBound(array, 1).ok()?;
        let last = SafeArrayGetUBound(array, 1).ok()?;
        if !(1..=128).contains(&(i64::from(last) - i64::from(first) + 1)) {
            return None;
        }
        let mut id = pid.to_string();
        for index in first..=last {
            let mut value = 0i32;
            SafeArrayGetElement(array, &index, &mut value as *mut i32 as _).ok()?;
            id.push_str(&format!(":{value}"));
        }
        Some(id)
    })();
    let _ = SafeArrayDestroy(array);
    result
}

/// Only inspect ancestors of the focused control, not siblings or other
/// windows. Exclude file/modal dialogs and editable ancestors before text reads.
unsafe fn automatic_context(
    automation: &IUIAutomation,
    element: &IUIAutomationElement,
    editable: bool,
    text_readonly: Option<bool>,
) -> &'static str {
    if editable {
        return "editing";
    }
    let Ok(walker) = automation.RawViewWalker() else {
        return "unknown";
    };
    let mut node = element.clone();
    let mut readable = text_readonly == Some(true);
    for _ in 0..24 {
        let Ok(kind) = node.CurrentControlType() else {
            return "unknown";
        };
        let modal = node
            .GetCurrentPatternAs::<IUIAutomationWindowPattern>(UIA_WindowPatternId)
            .ok()
            .and_then(|p| p.CurrentIsModal().ok())
            .is_some_and(|b| b.as_bool());
        let dialog = node
            .GetCurrentPropertyValue(UIA_IsDialogPropertyId)
            .ok()
            .is_some_and(|mut value| {
                let result = value.Anonymous.Anonymous.vt == VT_BOOL
                    && VariantToBoolean(&value).is_ok_and(|b| b.as_bool());
                let _ = VariantClear(&mut value);
                result
            });
        if modal || dialog {
            return "dialog";
        }
        if kind == UIA_EditControlTypeId || kind == UIA_ComboBoxControlTypeId {
            return "editing";
        }
        readable |= kind == UIA_DocumentControlTypeId || kind == UIA_TextControlTypeId;
        if kind == UIA_WindowControlTypeId {
            return if readable { "reading" } else { "unknown" };
        }
        let Ok(parent) = walker.GetParentElement(&node) else {
            break;
        };
        node = parent;
    }
    if readable {
        "reading"
    } else {
        "unknown"
    }
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

/// UIA returns physical screen coordinates for visible lines. Copy the owned
/// SAFEARRAY on this COM thread and always free it; an empty array explicitly
/// means off-screen, whereas an unsupported API falls back to the pointer.
unsafe fn selection_anchor(range: Option<&IUIAutomationTextRange>) -> Option<Value> {
    let rectangles = range.and_then(|range| {
        let array = range.GetBoundingRectangles().ok()?;
        if array.is_null() {
            return None;
        }
        let copied = (|| {
            if SafeArrayGetDim(array) != 1 || SafeArrayGetElemsize(array) != 8 {
                return None;
            }
            let first = SafeArrayGetLBound(array, 1).ok()?;
            let last = SafeArrayGetUBound(array, 1).ok()?;
            let count = i64::from(last) - i64::from(first) + 1;
            if !(0..=64000).contains(&count) || count % 4 != 0 {
                return None;
            }
            let mut values = Vec::<f64>::with_capacity(count as usize);
            for index in first..=last {
                let mut value = 0.0_f64;
                SafeArrayGetElement(array, &index, &mut value as *mut f64 as _).ok()?;
                if !value.is_finite() {
                    return None;
                }
                values.push(value);
            }
            Some(values)
        })();
        let _ = SafeArrayDestroy(array);
        copied
    });
    let rect = rectangles.as_ref().and_then(|values| {
        values
            .chunks_exact(4)
            .filter(|r| r[2] > 0.0 && r[3] > 0.0)
            .reduce(|a, b| if b[1] < a[1] { b } else { a })
    });
    if let Some(r) = rect {
        return Some(
            json!({"rect":{"x":r[0],"y":r[1],"width":r[2],"height":r[3]},
            "space":"physical","kind":"selection","visible":true}),
        );
    }
    let mut point = windows::Win32::Foundation::POINT::default();
    GetCursorPos(&mut point).ok()?;
    Some(
        json!({"rect":{"x":point.x,"y":point.y,"width":1,"height":1},
        "space":"physical","kind":"cursor","visible":rectangles.is_none()}),
    )
}

/// Capture the focused range, optionally adding geometry for the read popover.
/// Geometry failures never weaken password protection or replacement tickets.
unsafe fn capture(
    automation: &IUIAutomation,
    snapshots: &mut HashMap<String, Snapshot>,
    mouse: &SelectionMouse,
    request: &Value,
) -> Result<Value, String> {
    let automatic = request["op"] == "selection";
    let front = foreground_pid();
    let product = product_key(front);
    let allowed = product.is_some_and(|key| {
        request["allowedApps"]
            .as_array()
            .is_some_and(|apps| apps.iter().any(|a| a == key))
    });
    if automatic && !allowed && request["tracking"]["pid"].as_u64() != Some(front as u64) {
        return Ok(json!({"ignored":true,"reason":"application","pid":front}));
    }
    let element = automation.GetFocusedElement().map_err(failure)?;
    let pid = element.CurrentProcessId().map_err(failure)? as u32;
    if pid == std::process::id() || pid != front || foreground_pid() != pid {
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
    let text_readonly = pattern
        .as_ref()
        .and_then(|p| p.DocumentRange().ok())
        .and_then(|r| r.GetAttributeValue(UIA_IsReadOnlyAttributeId).ok())
        .map(|mut raw| {
            let result = (raw.Anonymous.Anonymous.vt == VT_BOOL)
                .then(|| VariantToBoolean(&raw).ok().map(|v| v.as_bool()))
                .flatten();
            let _ = VariantClear(&mut raw);
            result
        })
        .flatten();
    let editable = value
        .as_ref()
        .and_then(|v| v.CurrentIsReadOnly().ok())
        .is_some_and(|v| !v.as_bool())
        || text_readonly == Some(false);
    let context = context_id(&element, pid).unwrap_or_default();
    let kind = if automatic {
        automatic_context(automation, &element, editable, text_readonly)
    } else {
        "manual"
    };
    let mut result = json!({"app":match product {Some("codex")=>"Codex",Some("claude")=>"Claude Desktop",_=>"Windows 应用"},
        "pid":pid,"contextId":context,"autoEligible":allowed && kind == "reading"});
    mouse.annotate(&mut result, pid);
    let following = !context.is_empty()
        && request["tracking"]["pid"].as_u64() == Some(pid as u64)
        && request["tracking"]["contextId"] == context;
    if automatic && (!allowed || kind != "reading") && !following {
        result["ignored"] = json!(true);
        result["reason"] = json!(kind);
        return Ok(result);
    }
    let range = pattern.as_ref().and_then(|p| selection(p));
    let selected = range
        .as_ref()
        .and_then(|r| r.GetText(16001).ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let document = if request["whole"] == true || request["ticket"] == true {
        value
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
            .unwrap_or_default()
    } else {
        String::new()
    };
    let whole = selected.is_empty() && request["whole"] == true && editable;
    let text = if whole { &document } else { &selected };
    if text.trim().is_empty() {
        if automatic {
            return Ok(result);
        }
        return Err("没有选中文字，或当前控件不支持安全读取。".into());
    }
    if text.encode_utf16().count() > 16000 {
        return Err("本次文字超过 16,000 字符，请分段翻译。".into());
    }
    result["text"] = json!(text);
    result["editable"] = json!(editable);
    result["whole"] = json!(whole);
    if request["geometry"] == true {
        result["anchor"] = selection_anchor(range.as_ref()).unwrap_or(Value::Null);
    }
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
    mouse: &mut SelectionMouse,
    request: Value,
) -> Result<Value, String> {
    if request["op"] == "selection" {
        mouse.allowed = request["allowedApps"]
            .as_array()
            .map(|apps| {
                apps.iter()
                    .filter_map(|a| a.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
    }
    match request["op"].as_str().unwrap_or("") {
        "status" | "permission" => {
            Ok(json!({"accessibility":true,"systemTranslation":false,"platform":"Windows"}))
        }
        "selection" if foreground_pid() == std::process::id() => Ok(json!({"self":true})),
        "selection" | "capture" => capture(automation, snapshots, mouse, &request),
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
