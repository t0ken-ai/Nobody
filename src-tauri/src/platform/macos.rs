use serde_json::Value;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
};
use tokio::sync::oneshot;

type Callback = unsafe extern "C" fn(*const c_char, *mut c_void);
extern "C" {
    // The build script links this implementation into the main executable, so
    // hardened runtime never needs to authorize a separately signed library.
    fn tm_native_request(json: *const c_char, callback: Callback, context: *mut c_void);
}

/// JSON/callback boundary to the statically linked macOS adapter. The resource
/// path remains part of the shared platform constructor but is unused on macOS.
pub struct Platform;
impl Platform {
    /// No runtime module loading or additional platform resources are required.
    pub fn new(_resources: &Path) -> Result<Self, String> {
        Ok(Self)
    }
    /// Swift copies the request before returning and owns the callback context
    /// until its single reply, including when the awaiting operation times out.
    pub async fn call(&self, value: Value) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel::<Value>();
        let context = Box::into_raw(Box::new(tx)) as *mut c_void;
        let json = CString::new(value.to_string()).map_err(|e| e.to_string())?;
        // Swift copies the C string synchronously, then uses its main queue.
        unsafe {
            tm_native_request(json.as_ptr(), complete, context);
        }
        // A slow model download may still finish after timeout. The callback owns
        // its sender until then, so dropping this receiver cannot cause a UAF.
        let response = if value["op"] == "pickApplications" {
            // A person browsing Applications has no deadline. Swift's panel
            // completion replies exactly once on either selection or cancel.
            rx.await
        } else {
            tokio::time::timeout(std::time::Duration::from_secs(120), rx)
                .await
                .map_err(|_| "操作超时；如首次使用系统翻译，请完成语言包下载后重试。".to_string())?
        }
        .map_err(|_| "原生操作已结束。".to_string())?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            Err(error.into())
        } else {
            Ok(response)
        }
    }
}

unsafe extern "C" fn complete(json: *const c_char, context: *mut c_void) {
    let sender = Box::from_raw(context as *mut oneshot::Sender<Value>);
    let value = if json.is_null() {
        serde_json::json!({"error": "原生操作没有返回结果"})
    } else {
        serde_json::from_slice(CStr::from_ptr(json).to_bytes())
            .unwrap_or_else(|_| serde_json::json!({"error": "原生结果格式错误"}))
    };
    let _ = sender.send(value);
}
