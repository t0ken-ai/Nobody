//! Preferences contain no secrets. The API key lives in the OS credential store
//! and is scoped to an endpoint, so switching providers cannot leak an old key.
use serde::{Deserialize, Deserializer, Serialize};
use std::{fs, path::Path};

/// Presets follow a known native identity even before installation. User-picked
/// apps bind to a canonical local path (and a bundle ID on macOS), so unrelated
/// applications with the same display name cannot gain automatic capture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Application {
    Preset(String),
    Local {
        name: String,
        path: String,
        platform: String,
        #[serde(rename = "bundleId", default)]
        bundle_id: String,
    },
}

/// The previous release exposed only Codex/Claude checkboxes. Correct that old
/// Codex default to ChatGPT, preserving deliberate removals (especially []).
/// New custom entries never go through this product-key migration.
fn read_applications<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Application>, D::Error> {
    let mut apps = Vec::<Application>::deserialize(d)?;
    for app in &mut apps {
        if *app == Application::Preset("codex".into()) {
            *app = Application::Preset("chatgpt".into());
        }
    }
    let mut unique = Vec::new();
    for app in apps {
        if !unique.contains(&app) {
            unique.push(app);
        }
    }
    Ok(unique)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub engine: String,
    pub target_language: String,
    pub auto_selection: bool,
    /// Automatic selection only; explicit shortcuts remain user initiated.
    #[serde(deserialize_with = "read_applications")]
    pub automatic_apps: Vec<Application>,
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
            automatic_apps: vec![
                Application::Preset("chatgpt".into()),
                Application::Preset("claude".into()),
            ],
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
    // Structural validation is platform independent. Native validation at save
    // time additionally verifies the file/bundle exists and matches its identity.
    if s.automatic_apps.len() > 100 {
        return Err("白名单最多支持 100 个应用。".into());
    }
    for app in &s.automatic_apps {
        match app {
            Application::Preset(key) if ["chatgpt", "claude"].contains(&key.as_str()) => {}
            Application::Local {
                name,
                path,
                platform,
                bundle_id,
            } => {
                let valid_path = match platform.as_str() {
                    "macOS" => {
                        path.starts_with('/')
                            && path.to_ascii_lowercase().ends_with(".app")
                            && !bundle_id.trim().is_empty()
                    }
                    "Windows" => {
                        let bytes = path.as_bytes();
                        bytes.len() > 3
                            && bytes[0].is_ascii_alphabetic()
                            && bytes[1] == b':'
                            && bytes[2] == b'\\'
                            && path.to_ascii_lowercase().ends_with(".exe")
                    }
                    _ => false,
                };
                if name.trim().is_empty() || path.contains('\0') || !valid_path {
                    return Err("请选择本机有效的应用文件。".into());
                }
            }
            _ => return Err("未知的默认应用，请重新选择。".into()),
        }
    }
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
    fn defaults_and_legacy_migration_preserve_user_removals() {
        let missing: Settings = serde_json::from_str(r#"{"autoSelection":true}"#).unwrap();
        let legacy: Settings =
            serde_json::from_str(r#"{"automaticApps":["codex","claude"]}"#).unwrap();
        assert_eq!(missing.automatic_apps, Settings::default().automatic_apps);
        assert_eq!(legacy.automatic_apps, missing.automatic_apps);
        let empty: Settings = serde_json::from_str(r#"{"automaticApps":[]}"#).unwrap();
        assert!(empty.automatic_apps.is_empty());
        let removed: Settings = serde_json::from_str(r#"{"automaticApps":["claude"]}"#).unwrap();
        assert_eq!(
            removed.automatic_apps,
            [Application::Preset("claude".into())]
        );
    }
    #[test]
    fn chosen_apps_roundtrip_without_becoming_presets() {
        let raw = r#"{"automaticApps":[{"name":"Codex","path":"/Applications/Codex.app","platform":"macOS","bundleId":"com.openai.codex"}]}"#;
        let s: Settings = serde_json::from_str(raw).unwrap();
        validate(&s).unwrap();
        let copy: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(copy.automatic_apps, s.automatic_apps);
    }
    #[test]
    fn rejects_non_application_paths_and_unknown_presets() {
        for raw in [
            r#"["terminal"]"#,
            r#"[{"name":"Web","path":"https://example.com/app.app","platform":"macOS","bundleId":"web"}]"#,
            r#"[{"name":"File","path":"/Applications/test.txt","platform":"macOS","bundleId":"test"}]"#,
            r#"[{"name":"Script","path":"C:\\Tools\\run.bat","platform":"Windows"}]"#,
        ] {
            let mut s = Settings::default();
            s.automatic_apps = serde_json::from_str(raw).unwrap();
            assert!(validate(&s).is_err());
        }
    }
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
