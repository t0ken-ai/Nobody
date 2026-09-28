//! Integration fixture uses the actual updater plugin with a mock app handle.
//! Only loopback and a disposable bundle are used; no UI is driven, no installed
//! app is replaced and no application data or production signing key is read.
use serde_json::json;
use tauri_plugin_updater::UpdaterExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "run node scripts/test-updater.mjs to create an ephemeral signed fixture"]
async fn isolated_signed_updater_install() {
    let fixture = std::path::PathBuf::from(
        std::env::var_os("NOBODY_UPDATE_FIXTURE_DIR").expect("fixture directory required"),
    );
    let archive = std::fs::read(fixture.join("update.tar.gz")).unwrap();
    let signature = std::fs::read_to_string(fixture.join("update.tar.gz.sig")).unwrap();
    let public_key = std::fs::read_to_string(fixture.join("test.key.pub")).unwrap();
    let version = std::fs::read_to_string(fixture.join("version.txt")).unwrap();
    let wrong_version = format!(
        "{}.0.0",
        version.parse::<semver::Version>().unwrap().major + 1
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            let count = stream.read(&mut request).await.unwrap();
            let path = String::from_utf8_lossy(&request[..count])
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .to_string();
            let body = if path == "/package" {
                archive.clone()
            } else if path == "/tampered-package" {
                let mut changed = archive.clone();
                changed.push(1);
                changed
            } else {
                let version = if path == "/wrong-version" {
                    wrong_version.as_str()
                } else {
                    version.as_str()
                };
                let package = if path == "/tampered" {
                    "tampered-package"
                } else {
                    "package"
                };
                serde_json::to_vec(&json!({"version":version,"notes":"## Signed fixture\n- Update preserved.",
                        "platforms":{"darwin-aarch64":{"signature":signature.trim(),"url":format!("http://{address}/{package}")},
                                     "darwin-x86_64":{"signature":signature.trim(),"url":format!("http://{address}/{package}")}}})).unwrap()
            };
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            stream.write_all(&body).await.unwrap();
        }
    });
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.package_info_mut().version = "0.0.0".parse().unwrap();
    context.config_mut().plugins.0.insert(
        "updater".into(),
        json!({
            "pubkey":public_key.trim(), "requireSignedVersion":true,
            // Only the mock test permits HTTP; the production config requires HTTPS.
            "dangerousInsecureTransportProtocol":true,
            "endpoints":[format!("http://{address}/latest")]
        }),
    );
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)
        .unwrap();
    let installed = fixture.join("installed/Nobody.app/Contents/MacOS/nobody");
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, "old fixture").unwrap();
    let user_data = fixture.join("user-data.txt");
    std::fs::write(&user_data, "preserve this unrelated data").unwrap();
    // Reject both altered bytes and a valid old package paired with a forged
    // higher manifest version before touching the existing application.
    for path in ["tampered", "wrong-version"] {
        let update = app
            .updater_builder()
            .executable_path(&installed)
            .endpoints(vec![format!("http://{address}/{path}").parse().unwrap()])
            .unwrap()
            .build()
            .unwrap()
            .check()
            .await
            .unwrap()
            .unwrap();
        assert!(update.download(|_, _| {}, || {}).await.is_err());
        assert_eq!(std::fs::read_to_string(&installed).unwrap(), "old fixture");
    }
    let update = app
        .updater_builder()
        .executable_path(&installed)
        .build()
        .unwrap()
        .check()
        .await
        .unwrap()
        .unwrap();
    assert!(update.body.as_deref().unwrap().contains("Signed fixture"));
    let bytes = update.download(|_, _| {}, || {}).await.unwrap();
    update.install(bytes).unwrap();
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        std::fs::read(fixture.join("expected-binary")).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&user_data).unwrap(),
        "preserve this unrelated data"
    );
    server.abort();
}
