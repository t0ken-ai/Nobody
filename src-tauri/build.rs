use std::{env, path::PathBuf, process::Command};

/// Compile the small native adapter with the active Apple SDK. Older SDKs still
/// build selection support, but the app reports system translation as unavailable.
fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let source = root.join("../native/macos/Native.swift");
        let out = root.join("../native/macos/build");
        std::fs::create_dir_all(&out).unwrap();
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
        println!("cargo:rerun-if-env-changed=TRANSLATEME_SWIFT_DEVELOPER_DIR");
        let target = if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") {
            "arm64-apple-macosx13.0"
        } else {
            "x86_64-apple-macosx13.0"
        };
        let mut compiler = Command::new("xcrun");
        // A locally extracted SDK can build the Swift bridge without changing
        // the system-wide toolchain used by Rust or other projects.
        if let Ok(path) = env::var("TRANSLATEME_SWIFT_DEVELOPER_DIR") {
            compiler.env("DEVELOPER_DIR", path);
        }
        let result = compiler
            .args([
                "swiftc",
                "-emit-library",
                "-O",
                "-target",
                target,
                "-module-name",
                "TranslateMeNative",
            ])
            .arg(&source)
            .arg("-o")
            .arg(out.join("libTranslateMeNative.dylib"))
            .status()
            .expect("xcrun is required to build the macOS adapter");
        assert!(
            result.success(),
            "Could not compile the native macOS adapter"
        );
    }
    tauri_build::build();
}
