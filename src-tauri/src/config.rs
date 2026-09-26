//! Preferences contain no secrets. The API key lives in the OS credential store
//! and is scoped to an endpoint, so switching providers cannot leak an old key.
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub engine: String,
    pub target_language: String,
    pub auto_selection: bool,
    pub write_shortcut: String,
    pub read_shortcut: String,
    pub endpoint: String,
    pub model: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: "system".into(),
            target_language: "zh-Hans".into(),
            auto_selection: true,
            write_shortcut: "CommandOrControl+Shift+E".into(),
            read_shortcut: "CommandOrControl+Shift+D".into(),
            endpoint: String::new(),
            model: String::new(),
        }
    }
}

pub const LANGUAGES: &[(&str, &str)] = &[
    ("zh-Hans", "Simplified Chinese"),
    ("zh-Hant", "Traditional Chinese"),
    ("en", "English"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("fr", "French"),
    ("de", "German"),
    ("es", "Spanish"),
    ("pt", "Portuguese"),
    ("ru", "Russian"),
    ("it", "Italian"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("vi", "Vietnamese"),
    ("th", "Thai"),
    ("id", "Indonesian"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
];

/// Validate before saving or sending data. Plain HTTP is accepted only for a
/// loopback LLM (e.g. Ollama); redirects are disabled separately in the client.
pub fn endpoint_url(raw: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(raw.trim())
        .map_err(|_| "请输入完整 API 地址，如 https://example.com/v1")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("API 地址需要 HTTPS；本机服务可以使用 HTTP。".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("请将密钥填写在 API Key 字段，地址中不要包含账号、查询参数或片段。".into());
    }
    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/chat/completions") {
        url.set_path(&format!("{path}/chat/completions"));
    }
    Ok(url)
}

pub fn validate(s: &Settings) -> Result<(), String> {
    if !["system", "llm"].contains(&s.engine.as_str()) {
        return Err("未知的翻译引擎。".into());
    }
    if !LANGUAGES.iter().any(|(code, _)| *code == s.target_language) {
        return Err("请选择支持的目标语言。".into());
    }
    if s.engine == "llm" {
        endpoint_url(&s.endpoint)?;
        if s.model.trim().is_empty() {
            return Err("请填写 LLM 模型名称。".into());
        }
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Settings, String> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("设置文件损坏：{e}"))
}

/// Replace the preferences only after a complete write; the previous file stays
/// valid if serialization or disk I/O fails. No translation history is written.
pub fn write(path: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(path.parent().ok_or("设置目录不可用")?).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

fn credential(endpoint: &str) -> Result<keyring::Entry, String> {
    let account = endpoint_url(endpoint)?.to_string();
    keyring::Entry::new("app.translateme.desktop", &account)
        .map_err(|e| format!("无法打开系统凭据库：{e}"))
}
pub fn get_key(endpoint: &str) -> Result<Option<String>, String> {
    match credential(endpoint)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("无法读取 API Key：{e}")),
    }
}
pub fn set_key(endpoint: &str, key: &str) -> Result<(), String> {
    let entry = credential(endpoint)?;
    if key.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("无法删除 API Key：{e}")),
        }
    } else {
        entry
            .set_password(key)
            .map_err(|e| format!("无法保存 API Key：{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_credential_leaks_and_normalizes_compatible_endpoints() {
        for raw in [
            "http://example.com/v1",
            "https://key@example.com",
            "https://example.com?key=secret",
        ] {
            assert!(endpoint_url(raw).is_err());
        }
        assert_eq!(
            endpoint_url("https://example.com/v1/").unwrap().as_str(),
            "https://example.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint_url("http://127.0.0.1:11434/v1/chat/completions")
                .unwrap()
                .path(),
            "/v1/chat/completions"
        );
    }
}
