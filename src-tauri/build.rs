use std::{env, path::PathBuf, process::Command};

/// Resolve tools through the optional Swift-only SDK override. Keeping this out
/// of the parent environment avoids changing Rust's proc-macro linker toolchain.
fn swift_tool() -> Command {
    let mut command = Command::new("xcrun");
    if let Ok(path) = env::var("TRANSLATEME_SWIFT_DEVELOPER_DIR") {
        command.env("DEVELOPER_DIR", path);
    }
    command
}

/// Read a toolchain path without assuming that Xcode is installed globally.
fn tool_path(args: &[&str]) -> PathBuf {
    let output = swift_tool().args(args).output().expect("xcrun is required");
    assert!(
        output.status.success(),
        "Could not resolve Swift toolchain path"
    );
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}

/// Statically include the Swift adapter in the signed executable. A separately
/// ad-hoc-signed dylib passes codesign verification but is rejected by hardened
/// runtime library validation because it has no matching Team ID. Static linking
/// preserves that protection and the existing C callback boundary.
fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let source = root.join("../native/macos/Native.swift");
        let out = PathBuf::from(env::var("OUT_DIR").unwrap());
        std::fs::create_dir_all(&out).unwrap();
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
        println!("cargo:rerun-if-env-changed=TRANSLATEME_SWIFT_DEVELOPER_DIR");
        let target = if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("aarch64") {
            "arm64-apple-macosx13.0"
        } else {
            "x86_64-apple-macosx13.0"
        };
        let result = swift_tool()
            .args([
                "swiftc",
                "-emit-library",
                "-static",
                "-O",
                "-target",
                target,
                "-module-name",
                "TranslateMeNative",
            ])
            .arg(&source)
            .arg("-o")
            .arg(out.join("libTranslateMeNative.a"))
            .status()
            .expect("xcrun is required to build the macOS adapter");
        assert!(
            result.success(),
            "Could not compile the native macOS adapter"
        );

        // Swift objects carry framework/runtime autolink directives. Resolve
        // these against the SDK that compiled them, including Translation on
        // hosts whose default SDK predates that framework.
        let sdk = tool_path(&["--sdk", "macosx", "--show-sdk-path"]);
        let compiler = tool_path(&["--find", "swiftc"]);
        let swift_libraries = compiler
            .parent().unwrap().parent().unwrap()
            .join("lib/swift/macosx");
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=TranslateMeNative");
        println!("cargo:rustc-link-search=native={}", swift_libraries.display());
        println!("cargo:rustc-link-search=native={}", sdk.join("usr/lib/swift").display());
        println!("cargo:rustc-link-search=framework={}", sdk.join("System/Library/Frameworks").display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
    tauri_build::build();
}
