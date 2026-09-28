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
# 本地安装包（不产生正式签名更新包）
npm run bundle -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
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
node scripts/test-updater.mjs
node --test scripts/test-release-metadata.mjs
```

更新验签检查在 macOS / Windows 均使用临时签名密钥及 loopback 服务，验证篡改包和错配版本被拒绝。macOS 同时安装到一次性 `.app`；Windows 安装检查由 `scripts/test-windows-installer.ps1` 在独立的 GitHub runner 内执行，覆盖干净安装与 `/UPDATE` 文件替换，不对维护者电脑的已安装 Nobody 执行。

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
# 维护者的更新签名私钥路径；不要将私钥放进仓库
TAURI_SIGNING_PRIVATE_KEY="$HOME/.translateme-release/nobody-updater.key" node scripts/package-release.mjs
```

脚本执行 Release 构建、签名验证、DMG / ZIP 完整性检查，输出 `artifacts/releases/Nobody-<版本>-macOS-arm64.{dmg,zip,sha256}`。DMG 包含应用、Applications 链接及 [安装说明](release-macos.md)；不打包用户配置和密钥。该脚本仅支持 Apple 芯片，同时生成 `.app.tar.gz`、`.sig` 和 `manifest-darwin-aarch64.json`。Windows 签名分发使用以下命令（私钥仍在仓库外）：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = "$env:USERPROFILE\.translateme-release\nobody-updater.key"
node scripts/package-windows.mjs
```

Windows 产物为 `Nobody-<版本>-Windows-x64-setup.exe`、`.sig`、`.sha256` 和 `manifest-windows-x86_64.json`。默认按当前用户安装，NSIS 使用中英文语言并检查 WebView2；该更新签名不等同于 Authenticode 签名。

当前 macOS 配置使用 ad-hoc 签名。正式分发的 Developer ID 签名与 Apple 公证需另行配置；Release 优化不代表已经公证。

## GitHub Actions 双平台安装包

[Build installers](../.github/workflows/macos-installers.yml) 独立于常规检查工作流，复用上述打包脚本。使用 `macos-26` Apple Silicon 和 `windows-latest` x64 两个独立 runner，无需上传本地工具链或用户配置。工作流文件名保留 `macos-installers.yml`，以兼容现有徽章与链接。

| 触发方式 | 结果 |
| --- | --- |
| 推送到 `main` | 构建该提交的安装包，上传 Actions artifact |
| 推送 `v<版本>` 标签 | 校验版本后构建，完整上传所有文件后发布正式 Release |
| Actions 页面点击 Run workflow | 构建选择的分支／标签 |

工作流执行 TypeScript、Rust 测试和更新清单合并检查；macOS 另跑原生翻译规则，Windows 另跑真实 NSIS 安装与更新模式检查。两个平台各自生成并验签，之后上传平台独占的更新元数据。安装包和校验清单保留 30 天，成功运行的摘要提供下载链接。下载方式见 [安装说明](release-macos.md#下载)。

构建任务只有仓库读取权限，更新签名使用 `TAURI_SIGNING_PRIVATE_KEY` 这个 Actions Secret（如私钥有密码，还需 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）。独立发布任务等待两个平台都成功，再用 `scripts/merge-updater.mjs` 校验平台文件、版本、摘要、签名对应关系和 SHA-256，合并生成包含 `darwin-aarch64` 与 `windows-x86_64` 的唯一 `latest.json`。只有这个任务获得 `contents: write`，先完整上传到草稿，再发布；已发布版本不覆盖，修复应增加版本号。

发布新版本时，同步 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 和 `src-tauri/tauri.conf.json`，并添加 `docs/releases/v<版本>.md`。此文件同时用作两个平台的 Release 正文和 `latest.json` 中的 `notes`，客户端直接显示，避免摘要与版本脱节。确认检查通过后推送对应 `v<版本>` 标签。

更新签名私钥仅由维护者保管，放在仓库外并备份，不能提交或分发。应用只包含公钥；使用 Tauri CLI 2.12+ 生成带版本约束的签名，客户端要求 `requireSignedVersion`，拒绝清单版本与签名版本不一致的包。签名不依赖系统钥匙串，也不等同于 Apple 公证。分叉项目发布自己的版本时，应替换端点、公钥和私钥。

提交工作流后必须在实际仓库检查首次运行结果；本地打包成功不代表 GitHub 环境已经验证。仓库如禁用 Actions，需先由维护者启用。Runner 规格见 [GitHub 官方说明](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。

## 代码结构

| 路径 | 职责 |
| --- | --- |
| `src/` | TypeScript 界面、操作演示、快捷键显示和 Markdown 渲染 |
| `src-tauri/src/main.rs` | 应用生命周期、快捷键、请求协调和窗口入口 |
| `src-tauri/src/selection.rs`、`popover.rs` | 自动划词规则、来源跟踪和浮窗位置 |
| `src-tauri/src/translation.rs`、`document.rs` | 引擎请求、输出校验和技术文本保护 |
| `src-tauri/src/config.rs`、`llm_store.rs`、`private_files.rs` | 配置、本地加密存储与文件权限 |
| `src-tauri/src/updater.rs`、`src/updates.ts` | 更新检查、提醒、下载和验签；协调器提供安装前的空闲锁 |
| `src-tauri/src/desktop.rs`、`src/desktop.ts` | 关于信息与登录时启动；开关以系统实际注册状态为准，默认不注册；`--background` 只控制窗口初始显示 |
| `src-tauri/src/transfer/` | 独立的局域网发现、TLS 身份、信任、传输和记录 |
| `src-tauri/src/platform/`、`native/macos/` | 系统取词、回填、应用身份、图标及原生翻译适配 |
| `src-tauri/prompts/developer-translator.txt` | 默认的可编辑翻译角色与规则 |
| `design/nobody/` | 图标源素材和重建说明 |

前端通过显式 Tauri 命令调用原生能力，不直接处理服务密钥。互传模块不依赖翻译引擎或选区内容。Bundle ID `app.translateme.desktop`、`~/.translateme/` 和局域网协议标识为升级兼容而保留，不能仅因产品更名而批量替换。

## 常驻性能约束

选区检查先看手势与浮窗状态，空闲时跳过 AX/UIA 正文读取；关闭自动划词后停止手势监听。隐藏主窗口时暂停演示与互传页面快照刷新，后台收发及首次配对提示继续工作。原生回填快照、图标缓存和任务队列有生命周期或容量限制。

检查性能时分别记录应用主进程与 WebView 工作进程，并注明构建类型、系统版本、窗口状态及采样时长。不要用短时间主进程读数代表整个应用的内存，也不要将编译或模拟界面测试描述为 Windows 实机表现。
