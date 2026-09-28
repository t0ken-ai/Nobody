# 开发指南

[返回 README](../README.md) · [验证范围](verification.md) · [贡献指南](../CONTRIBUTING.md)

## 环境

- Node.js 22+，使用仓库中的 `package-lock.json` 安装前端依赖。
- Rust stable，保留 `src-tauri/Cargo.lock`。
- macOS：Xcode 或 Command Line Tools。系统翻译需要包含 Translation 框架的 SDK；推荐 macOS 26+ SDK 以启用已安装模型的无界面会话。
- Windows：Visual Studio C++ Build Tools（桌面 C++ 工具链）、Perl、WebView2。SQLCipher 的 Windows 构建会编译随依赖打包的 OpenSSL。

macOS 的 SQLCipher 使用系统 CommonCrypto，无需单独安装 SQLCipher。Swift 适配层静态链接进应用；分发包不需要携带开发机工具链。

## 运行与构建

在仓库根目录执行：

```sh
npm ci
npm run desktop
```

`npm run dev` 只启动前端开发服务器，不能替代带原生能力的桌面应用。

```sh
# 生产前端
npm run build
# 当前平台的桌面安装包
npm run bundle
```

Tauri 输出位于 `src-tauri/target/release/bundle/`。macOS 的应用为其中的 `macos/Nobody.app`。

如果需要单独指定 Swift 工具链，可在构建命令前设置 `TRANSLATEME_SWIFT_DEVELOPER_DIR` 指向 Xcode 的 `Contents/Developer` 或 Command Line Tools 目录；该覆盖仅用于 Swift，不改变 Rust 的工具链。

`scripts/local-build.mjs` 可选择已经存在于 `artifacts/toolchain-downloads/` 的本地工具链，不会下载或安装它。普通开发环境不需要准备这个目录：

```sh
# 本地调试 .app
node scripts/local-build.mjs
# 使用相同的工具链选择逻辑启动开发模式
node scripts/local-build.mjs dev
```

## 检查

常规检查不使用真实 LLM 密钥：

```sh
npm run check
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

macOS 原生规则检查会编译生产 Swift 适配层，使用合成数据，不读取其他应用窗口：

```sh
node scripts/test-selection-layout.mjs
node scripts/test-translation-models.mjs
```

可选实测需明确选择，不能用常规测试结果替代：

```sh
# macOS 26+：使用已经安装的中英文模型，不允许下载
node scripts/test-translation-models.mjs --live
# 在当前局域网发布两个临时测试设备，验证真实 mDNS 发现
cargo test --manifest-path src-tauri/Cargo.toml --locked local_multicast_discovery -- --ignored --nocapture
# 使用进程已有的 ZAI_KEY 向配置在测试中的 Z.ai 接口发送固定样例
cargo test --manifest-path src-tauri/Cargo.toml --locked live_zai_translation -- --ignored --nocapture
```

不要批量运行全部 ignored 测试；其中还有需要专用目录和人工核码的原生互传诊断。不要将真实密钥写进命令、测试夹具或提交记录。

[GitHub Actions](../.github/workflows/check.yml) 配置了 macOS / Windows 的前端构建、Rust 测试及桌面调试构建。工作流存在不代表已在远程执行成功，原生 UI、权限与双机行为仍需手动验证。

## macOS 分发包

在 Apple 芯片 Mac 完成依赖安装后运行：

```sh
node scripts/package-release.mjs
```

脚本执行 Release 构建、签名验证、DMG / ZIP 完整性检查，输出 `artifacts/releases/Nobody-<版本>-macOS-arm64.{dmg,zip,sha256}`。DMG 包含应用、Applications 链接及 [安装说明](release-macos.md)；不打包用户配置和密钥。该脚本仅支持 Apple 芯片，Windows 使用 `npm run bundle` 在 Windows 环境构建。

当前 macOS 配置使用 ad-hoc 签名。正式分发的 Developer ID 签名与 Apple 公证需另行配置；Release 优化不代表已经公证。

## 代码结构

| 路径 | 职责 |
| --- | --- |
| `src/` | TypeScript 界面、操作演示、快捷键显示和 Markdown 渲染 |
| `src-tauri/src/main.rs` | 应用生命周期、快捷键、请求协调和窗口入口 |
| `src-tauri/src/selection.rs`、`popover.rs` | 自动划词规则、来源跟踪和浮窗位置 |
| `src-tauri/src/translation.rs`、`document.rs` | 引擎请求、输出校验和技术文本保护 |
| `src-tauri/src/config.rs`、`llm_store.rs`、`private_files.rs` | 配置、本地加密存储与文件权限 |
| `src-tauri/src/transfer/` | 独立的局域网发现、TLS 身份、信任、传输和记录 |
| `src-tauri/src/platform/`、`native/macos/` | 系统取词、回填、应用身份、图标及原生翻译适配 |
| `src-tauri/prompts/developer-translator.txt` | 默认的可编辑翻译角色与规则 |
| `design/nobody/` | 图标源素材和重建说明 |

前端通过显式 Tauri 命令调用原生能力，不直接处理服务密钥。互传模块不依赖翻译引擎或选区内容。Bundle ID `app.translateme.desktop`、`~/.translateme/` 和局域网协议标识为升级兼容而保留，不能仅因产品更名而批量替换。

## 常驻性能约束

选区检查先看手势与浮窗状态，空闲时跳过 AX/UIA 正文读取；关闭自动划词后停止手势监听。隐藏主窗口时暂停演示与互传页面快照刷新，后台收发及首次配对提示继续工作。原生回填快照、图标缓存和任务队列有生命周期或容量限制。

检查性能时分别记录应用主进程与 WebView 工作进程，并注明构建类型、系统版本、窗口状态及采样时长。不要用短时间主进程读数代表整个应用的内存，也不要将编译或模拟界面测试描述为 Windows 实机表现。
