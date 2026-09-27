# TranslateMe

面向编码工作流的 macOS / Windows 桌面翻译器。Tauri 2 + Rust + TypeScript；macOS 使用小型 Swift 适配层。

## 使用

- **使用演示**：首页用两段可暂停、重播的动画展示操作。左侧演示输入中文、组合键按下、英文回填；右侧演示鼠标拖选、松开、自动弹出译文。样例只用于说明交互，不会调用翻译服务或发送按键。
- **写入英文**：在其他应用的输入框中按 `⌘⇧E`（Windows：`Ctrl+Shift+E`）。优先翻译选中文字；无选区时尝试读取当前可编辑输入框。译文只回填，**不会按发送**。
- **划词阅读**：开启“划词自动翻译”后，在其他应用中选中文字，稳定约 1 秒后触发翻译，完成后显示译文，**无需快捷键**。`⌘⇧D` / `Ctrl+Shift+D` 是备用的手动触发方式。默认阅读语言是简体中文。
- **跟随浮窗**：译文优先出现在选区上方，空间不足时放到下方，采用磨砂背景。拖动顶部可移动，本次保持手动位置；新的选区重新跟随。长译文在浮窗内滚动，关闭按钮或浮窗获得焦点后的 `Esc` 可收起。窗口不主动抢输入焦点。
- **翻译工作台**：粘贴文字、选择目标语言、点击翻译。此功能无需辅助功能权限。
- **设置**：修改阅读语言、快捷键、自动划词开关，或配置兼容 Chat Completions 的 LLM。
- 关闭窗口后应用继续驻留菜单栏 / 系统托盘；从菜单中退出。

### 已开启权限，但仍提示未授权

本地 ad-hoc 开发包重建后，系统可能仍将授权绑定到旧版本的签名；列表里的开关保持开启，也不代表当前版本已获授权。此时仅关闭再开启开关或重启应用可能无效。

先退出 TranslateMe，在系统设置 → 隐私与安全 → 辅助功能中移除旧 TranslateMe 条目，再添加当前实际运行的 `.app` 并开启权限，然后重新启动应用。macOS 27 的相应入口显示为 **Device Control and Data Access**。若系统要求 Touch ID / 登录密码，需要在系统对话框中完成验证。

以 TranslateMe 工作台显示“跨应用翻译已就绪”为准。完成后使用同一份构建测试；再次重建开发包可能需要重新授权。正式版本应使用稳定的开发者签名。

## 引擎和当前边界

| 能力 | macOS | Windows |
| --- | --- | --- |
| Apple 系统翻译 | macOS 15+，构建时需要含 Translation 的 SDK；首次使用可能下载语言包 | 不支持 Apple API，默认引擎待选定 |
| 自定义 LLM | 支持 | 支持 |
| 读取选区 / 回填 | Accessibility；需要用户授予辅助功能权限 | UI Automation；目标应用必须暴露相应文本接口 |
| 当前验证 | 见 `docs/verification.md` | 适配模块已交叉编译，尚未在 Windows 实机验收 |

Windows 版本目前使用自定义 LLM。没有添加未经确认的云端中转服务，也不会在系统翻译失败时自动把文字发到网络。目标语言是否受系统引擎支持，由 Apple Translation 在运行时检查。

系统翻译每次查询实际语言包状态。使用 macOS 26+ SDK 构建并运行于 macOS 26+ 时，已安装的语言组合直接翻译，不显示准备窗口；macOS 15–25 或旧 SDK 构建保留系统要求的界面会话。只有缺少语言包时才进入准备流程，普通翻译错误不会提示重新下载。语言包由系统管理，应用不缓存一个永久有效的“已下载”标记。

浮窗定位取决于源应用暴露的选区坐标。坐标可靠时随选区滚动、离屏后收起；首次拿不到有效坐标时固定在触发时的鼠标附近，不跟着鼠标乱跑。macOS 配置了跨桌面及全屏辅助窗口行为；不同应用的坐标质量、多显示器实机表现及 Windows 磨砂效果仍需逐项验收。

**LLM 设置**：填写基础地址（例如 `https://your-provider.example/v1`）、模型 ID 和 API Key。完整 `/chat/completions` 地址也可使用。本机模型支持 `http://localhost:11434/v1` 等回环地址；远程服务要求 HTTPS。密钥留空保留已存密钥，勾选删除才会移除。密钥按接口地址独立保存在 macOS Keychain / Windows Credential Manager。

## 安全回填和文本保护

- 获取翻译前的应用身份、控件、内容和选区。回填前逐项核对；变更、超时、不可写或接口不完整时，只展示译文供复制。
- macOS 通过普通粘贴回填，并在剪贴板没有被用户再次修改时恢复原来的所有数据格式；Windows 发送 Unicode 输入，不发送 Enter 键、不修改剪贴板。
- 代码围栏、行内代码、可识别的 URL、路径及常见标识符原样保留。保护是语法规则，不是完整代码解析器；复杂命令和表达式请放在反引号或代码块中。
- 纯自然语言片段按顺序翻译，段落和换行保持。较短片段可能缺少上下文，重要需求仍应检查译文。
- LLM 返回不完整、空白或段数不一致时不回填。不开启翻译历史，不记录原文、译文、API Key 或服务端响应正文。
- 不截图、不 OCR、不读取密码框。对不支持 Accessibility/UI Automation 的软件，使用工作台。

## 开发与构建

需要 Node.js 22+、Rust stable。macOS 需要 Apple Command Line Tools 或 Xcode；Windows 需要 Visual Studio C++ Build Tools 和 WebView2。

```sh
npm ci
npm run desktop
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run bundle
```

此工作区检测到系统 Apple SDK 较旧，因此从 Apple 官方软件更新目录下载、验证签名后，**只解压**了一份新工具链至被 Git 忽略的 `artifacts/toolchain-downloads/`。未运行安装包脚本，未覆盖系统工具。该构建辅助工具不包含在分发应用中。

```sh
# 此机器：自动选择项目内的 Swift 工具链，构建本地可运行 .app
node scripts/local-build.mjs
# 此机器：运行开发模式
node scripts/local-build.mjs dev
```

其他机器使用已安装的现代 Xcode / Command Line Tools 即可。也可通过 `TRANSLATEME_SWIFT_DEVELOPER_DIR` 指定 Swift 工具链，保持 Rust 的主工具链不变。macOS 应用输出在 `src-tauri/target/debug/bundle/macos/TranslateMe.app`。

本地调试包没有开发者分发签名和 Apple 公证；正式对外发布需要签名、公证、Windows 安装包和实机验收。

## 模块边界

- `src/`：共享界面，调用显式 Tauri 命令；不直接联网或访问系统。
- `src-tauri/src/main.rs`：快捷键、选区防抖、请求互斥、结果展示与设置协调。
- `popover.rs`：浮窗的选区生命周期、位置和尺寸计算；独立于翻译引擎，不存储翻译历史。
- `config.rs`：设置验证与系统凭据存储；`document.rs`：代码片段保护。
- `translation.rs`：系统 / LLM 翻译和输出验证。
- `platform/`、`native/macos/Native.swift`：系统取词及坐标、回填、系统翻译和 macOS 浮窗行为；不依赖前端 DOM。

源码没有配置 Git 远程仓库。添加自己的远程仓库后可启用随附的 macOS / Windows CI。
