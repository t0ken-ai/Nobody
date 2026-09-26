# 本地 Apple 工具链

系统原有 Swift 5.8 / macOS 13.3 SDK 不包含 Translation。本次从 Apple 官方 Software Update 目录下载以下安装包，并通过 `pkgutil --check-signature` 验证为 Apple Software 签名后，使用 `pkgutil --expand-full` 解包。没有运行安装程序或其中的脚本，没有修改全局 `xcode-select`。

- [Apple 官方更新目录](https://swscan.apple.com/content/catalogs/others/index-26-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog.gz)
- [Command Line Tools 可执行文件包](https://swcdn.apple.com/content/downloads/58/48/082-83364-A_KCEBOO2NJS/0d2rj4y5ucjlkgcvqt6f6a4pergug5tu2b/CLTools_Executables.pkg)
- [macOS SDK 包](https://swcdn.apple.com/content/downloads/58/48/082-83364-A_KCEBOO2NJS/0d2rj4y5ucjlkgcvqt6f6a4pergug5tu2b/CLTools_macOSNMOS_SDK.pkg)

解包后的版本是 Swift 6.4 / macOS 27 SDK。放在 `artifacts/toolchain-downloads/`，不提交 Git、不打包到应用。

`scripts/local-build.mjs` 只对 Swift 编译设置 `TRANSLATEME_SWIFT_DEVELOPER_DIR`。Rust 继续使用系统原有链接器，避免此机器中新版链接器与 Rust 宏动态库的兼容问题。

Swift 适配层编译为静态库，由 Rust 链接进最终可执行文件；不再随包携带并通过 `dlopen` 加载独立 dylib。链接时使用编译 Swift 的同一 SDK 解析系统框架和 Swift 运行库，运行时依赖来自 macOS。这样无需关闭 hardened runtime 的 library validation，也避免本地 ad-hoc 签名缺少 Team ID 导致启动崩溃。C 回调接口与模块职责不变。

已有现代 Xcode / Command Line Tools 的机器直接使用 `npm run bundle` 即可，无需下载上述包。

API 参考：[Apple Translation](https://developer.apple.com/documentation/translation/)、[Tauri 2](https://v2.tauri.app/concept/architecture/)、[Chat Completions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)。
