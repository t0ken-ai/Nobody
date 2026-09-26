# 验证记录

日期：2026-09-27。环境：Apple Silicon、macOS 27.0。

## 已完成

- `npm run check`：TypeScript 类型检查通过。
- `npm run build`：生产前端构建通过。
- `cargo test --manifest-path src-tauri/Cargo.toml`：7 个测试通过，覆盖代码块、未闭合代码块、Unicode 空白、变量名、相对/绝对路径、产品名上下文、LLM 响应解析和 API 地址约束。
- Windows 适配模块使用 `x86_64-pc-windows-gnu` 完成 `cargo check`；检查工程位于忽略目录 `artifacts/windows-check/`。
- 修复后的 macOS 本地 `.app` 打包成功，约 32.4 MiB，具有 ad-hoc 签名并保留 hardened runtime；`codesign --verify --deep --strict` 通过。
- 应用界面通过原生 UI 自动化检查，工作台、语言切换和结果显示正常。
- Apple 系统翻译首次下载了中文/英文语言包。没有使用预制译文或远程免费接口替代系统翻译。
- 用户在当前 Codex 聊天输入框亲手按 `⌘⇧E`，确认中文“已替换成英文”。这项记录来自用户实测，验证了当前构建在该输入框中的快捷键触发、取词、翻译和回填。

## 真实翻译结果

中文样例取自当前聊天需求的整理：

> 我想写一个 macOS 和 Windows 都能用的翻译器。通过快捷键把输入翻译成英文，选中文字时翻译成我设置的语言。请保留代码、路径和变量名。

Apple 系统实际返回：

> I want to write a translator that can be used by both macOS and Windows. Translate the input into English through the shortcut key, and translate the text into the language I set when I select it. Please keep the code, path and variable name.

反向翻译输入：

> Please keep the code unchanged and translate only the explanation.

实际返回：

> 请保持代码不变，只翻译说明。

代码保护实测中，行内代码 `user_id`、路径 `src/main.rs` 和下面的代码块均保持原样，解释文字被翻译为中文：

```rust
let 用户 = 1;
```

实测发现并修复：`macOS` 被当作 camelCase 标识符，导致句子被错误拆开。另补充回归测试，确保相对路径的首级目录也被保护。

## 启动崩溃修复与复验

用户反馈当前聊天划词和快捷键无响应。复现发现原最终打包版本启动即崩溃：hardened runtime 拒绝 `dlopen` 独立 ad-hoc 签名的 Swift dylib，错误包含 `different Team IDs`。仅 `codesign --verify` 通过不足以说明应用能够运行。

修复将同一个 Swift 适配层静态链接进主程序，移除动态库加载和随包资源，未关闭 library validation。修复后通过原生 UI 工具启动最终 `.app`，界面正常显示；再次提交上面的中文样例，Apple 系统翻译真实返回了相同英文译文，应用持续运行。`otool -L` 确认没有项目本地 dylib 依赖，Translation 框架保留弱链接；7 个 Rust 测试重新通过。

修复启动后，最终构建一度仍报告辅助功能权限未生效。TextEdit 测试文档中的自动化快捷键尝试未观察到回填；后续授权修复和用户实测结果见下文。

用户关闭再开启权限后仍不可用。系统设置实测显示 TranslateMe 开关已开；退出并重新启动同一份 `.app` 后，应用仍报告未授权。随后系统 `tccd` 日志明确返回 `Failed to match existing code requirement`，旧授权绑定的 cdhash 为 `9f0a0ea811f5d33aab6c6f1651195511be16e69a`，当前应用为 `4c8a3043a9b60c1d083b5f07c03d27ac481d5ea7`。这证明仅切换开关不会更新旧签名记录，先前让用户反复切换开关的建议不足。

用户已完成系统要求的 Touch ID / 登录密码验证。直接再次添加同一路径未能更新旧记录；随后在系统设置中先移除 TranslateMe 条目，再添加当前 `.app`，重启后工作台明确显示“跨应用翻译已就绪”。权限故障已解除；没有修改 TCC 数据库、关闭系统保护或重新构建应用。

自动化向 TextEdit 发送快捷键后未观察到译文或回填；工具连接中途断开一次并已恢复。这次尝试不能作为跨应用成功证据。随后用户在当前聊天输入框亲手按 `⌘⇧E`，确认“已替换成英文”，写入路径验收通过。自动划词与 `⌘⇧D` 阅读路径单独验收。

## 尚未完成的验收

- **当前 Codex 聊天的划词阅读**：用户提供了混合说明、代码和 Git 命令的译文，出现关键字及命令被翻译的问题，尚不满足开发场景验收。用户未说明此次结果是自动划词还是 `⌘⇧D` 触发，不能据此认定两条触发路径均通过。已用生产解析器复现围栏丢失后的代码泄漏，具体证据与待确认方案见 `structured-selection-proposal.md`。写入快捷键 `⌘⇧E` 仍保留用户已验证成功的记录。
- **Windows 运行**：只完成适配模块编译检查；本机不能验证 Windows UI Automation、焦点行为、快捷键冲突和安装包。随附 CI 尚未运行。
- **真实 LLM 服务**：接口、凭据存储和响应验证已实现；未提供服务地址、模型与 API Key，尚未进行真实服务联调。
- **Windows 默认翻译**：尚未选定通用默认引擎，当前可配置 LLM；不会把 Apple 系统翻译标为 Windows 可用。

## 用户验收步骤

1. 启动 `src-tauri/target/debug/bundle/macos/TranslateMe.app`。
2. 工作台点击“使用当前聊天示例”，再点击“翻译成英文”。
3. 在系统设置 → 隐私与安全 → 辅助功能中启用 TranslateMe；macOS 27 对应入口为 Device Control and Data Access。本地开发包重建后若开关已开但仍未授权，需移除旧项、重新添加当前 `.app`；仅切换开关可能无效。正式分发需稳定的开发者签名。
4. 在聊天输入框输入一段中文，按 `⌘⇧E`；应得到英文，消息不应自动发送。可在输入框使用 `⌘Z` 撤销。
5. 选中英文，等待自动翻译，或按 `⌘⇧D`；应弹出中文译文，原文不变。
6. 在较长翻译进行中修改原文或切换应用；译文应仅展示供复制，不覆盖新内容。

测试时创建了独立的 TextEdit 文档“Untitled 3”，仅包含本次测试句；未修改原有文档。
