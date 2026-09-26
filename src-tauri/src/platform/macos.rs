use libloading::Library;
use serde_json::Value;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
};
use tokio::sync::oneshot;

type Callback = unsafe extern "C" fn(*const c_char, *mut c_void);
type Request = unsafe extern "C" fn(*const c_char, Callback, *mut c_void);

pub struct Platform {
    _library: Library,
    request: Request,
}
impl Platform {
    /// Keep the library alive for every pending Swift callback. The only loaded
    /// binary is the adapter bundled with this app, never a user-provided path.
    pub fn new(resources: &Path) -> Result<Self, String> {
        let bundled = resources.join("native/libTranslateMeNative.dylib");
        let path = if cfg!(debug_assertions) && !bundled.exists() {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../native/macos/build/libTranslateMeNative.dylib")
        } else {
            bundled
        };
        unsafe {
            let library = Library::new(path).map_err(|e| e.to_string())?;
            let request = *library
                .get::<Request>(b"tm_native_request\0")
                .map_err(|e| e.to_string())?;
            Ok(Self {
                _library: library,
                request,
            })
        }
    }
    pub async fn call(&self, value: Value) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel::<Value>();
        let context = Box::into_raw(Box::new(tx)) as *mut c_void;
        let json = CString::new(value.to_string()).map_err(|e| e.to_string())?;
        // Swift copies the C string synchronously, then uses its main queue.
        unsafe {
            (self.request)(json.as_ptr(), complete, context);
        }
        // A slow model download may still finish after timeout. The callback owns
        // its sender until then, so dropping this receiver cannot cause a UAF.
        let response = tokio::time::timeout(std::time::Duration::from_secs(120), rx)
            .await
            .map_err(|_| "操作超时；如首次使用系统翻译，请完成语言包下载后重试。".to_string())?
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
