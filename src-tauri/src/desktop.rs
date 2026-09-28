//! Small desktop preferences: read the OS login item rather than duplicating
//! its state in settings.json. Only explicit main-window actions may change it.
use serde::Serialize;
use std::path::Path;
use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStatus {
    version: String,
    launch_on_login: bool,
    startup_available: bool,
    startup_message: String,
    main_visible: bool,
}

/// Register only a stable installed path. DMGs and App Translocation disappear
/// after eject/reboot; accepting either would leave a broken login item.
fn startup_available(path: &Path, mac: bool, development: bool) -> bool {
    if development { return false; }
    if !mac { return true; }
    !path.starts_with("/Volumes")
        // auto-launch 0.5 writes this path inside XML without escaping it.
        // Reject that unsupported case rather than registering a broken plist.
        && !path.to_string_lossy().contains(['<', '>', '&'])
        && !path.components().any(|c| c.as_os_str() == "AppTranslocation")
        && path.ancestors().any(|p| p.extension().is_some_and(|e| e == "app"))
}

/// Read actual OS state on mount/focus; a change in System Settings is visible
/// without a resident polling timer or an extra application preference file.
#[tauri::command]
pub fn get_desktop_status(app: tauri::AppHandle) -> Result<DesktopStatus, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let available = startup_available(&executable, cfg!(target_os = "macos"), cfg!(debug_assertions));
    Ok(DesktopStatus {
        version: app.package_info().version.to_string(),
        launch_on_login: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
        startup_available: available,
        startup_message: if available { "登录系统后在后台启动，不弹出主窗口。切换后立即生效。" }
            else { "请先安装正式版再开启；macOS 请移出安装磁盘，并避免路径含 < > &。" }.into(),
        main_visible: app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false)),
    })
}

/// Do not silently enable on first run or update. Allow disabling an existing
/// entry even from a moved build, so the user can always remove a stale item.
#[tauri::command]
pub fn set_launch_on_login(app: tauri::AppHandle, window: tauri::WebviewWindow, enabled: bool) -> Result<DesktopStatus, String> {
    if window.label() != "main" { return Err("请从主窗口调整开机启动。".into()); }
    if enabled && !get_desktop_status(app.clone())?.startup_available {
        return Err("请先安装正式版，再开启登录时启动。".into());
    }
    if enabled { app.autolaunch().enable() } else { app.autolaunch().disable() }
        .map_err(|e| e.to_string())?;
    get_desktop_status(app)
}

/// Keep external navigation bounded to the two About destinations. A webview
/// cannot turn this command into an arbitrary URL/file or shell opener.
#[tauri::command]
pub fn open_about_link(app: tauri::AppHandle, window: tauri::WebviewWindow, destination: String) -> Result<(), String> {
    if window.label() != "main" { return Err("请从主窗口打开项目链接。".into()); }
    let url = match destination.as_str() {
        "github" => "https://github.com/t0ken-ai/Nobody",
        "email" => "mailto:spridu@gmail.com",
        _ => return Err("未知的项目链接。".into()),
    };
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_or_development_builds_cannot_be_registered_at_login() {
        for path in ["/Volumes/Nobody/Nobody.app/Contents/MacOS/nobody", "/private/var/folders/x/AppTranslocation/y/Nobody.app/Contents/MacOS/nobody", "/tmp/nobody", "/Users/A&B/Nobody.app/Contents/MacOS/nobody"] {
            assert!(!startup_available(Path::new(path), true, false));
        }
        let installed = Path::new("/Applications/Nobody.app/Contents/MacOS/nobody");
        assert!(startup_available(installed, true, false));
        assert!(!startup_available(installed, true, true));
        assert!(startup_available(Path::new(r"C:\Users\me\AppData\Local\Nobody\nobody.exe"), false, false));
    }

    /// Exercise real per-user registration in disposable CI only. A unique
    /// identity plus Drop cleanup keeps the Nobody login item untouched.
    #[test]
    #[ignore = "creates a disposable login item on hosted CI only"]
    fn isolated_login_item_round_trip() {
        assert_eq!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true"));
        let name = format!("app.translateme.startup-test.{}", uuid::Uuid::new_v4());
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_autostart::Builder::new().app_name(&name).arg("--background").build())
            .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let manager = app.autolaunch();
        assert!(!manager.is_enabled().unwrap());
        struct Cleanup<'a>(&'a tauri_plugin_autostart::AutoLaunchManager);
        impl Drop for Cleanup<'_> { fn drop(&mut self) { let _ = self.0.disable(); } }
        let _cleanup = Cleanup(&manager);
        manager.enable().unwrap();
        assert!(manager.is_enabled().unwrap());
        #[cfg(target_os = "macos")]
        {
            let file = dirs::home_dir().unwrap().join("Library/LaunchAgents").join(format!("{name}.plist"));
            assert!(std::fs::read_to_string(file).unwrap().contains("<string>--background</string>"));
        }
        #[cfg(target_os = "windows")]
        {
            let output = std::process::Command::new("reg").args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", &name]).output().unwrap();
            assert!(output.status.success());
            assert!(String::from_utf8_lossy(&output.stdout).contains("--background"));
        }
        manager.disable().unwrap();
        assert!(!manager.is_enabled().unwrap());
    }
}
