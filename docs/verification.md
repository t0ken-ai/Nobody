# 验证记录

日期：2026-09-27。环境：Apple Silicon、macOS 27.0。

## 已完成

- `npm run check`：TypeScript 类型检查通过。
- `npm run build`：生产前端构建通过。
- `cargo test --manifest-path src-tauri/Cargo.toml`：7 个测试通过，覆盖代码块、未闭合代码块、Unicode 空白、变量名、相对/绝对路径、产品名上下文、LLM 响应解析和 API 地址约束。
- Windows 适配模块使用 `x86_64-pc-windows-gnu` 完成 `cargo check`；检查工程位于忽略目录 `artifacts/windows-check/`。
- macOS 本地 `.app` 打包成功，约 32.5 MiB，具有 ad-hoc 签名；`codesign --verify --deep --strict` 通过。
- 应用界面通过原生 UI 自动化检查，工作台、语言切换和结果显示正常。
- Apple 系统翻译首次下载了中文/英文语言包。没有使用预制译文或远程免费接口替代系统翻译。

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

## 尚未完成的验收

- **当前 Codex 聊天的直接取词/回填**：自动化工具明确禁止操作 Codex，因此只能由用户在聊天中亲手测试。已用聊天内容验证工作台翻译，不能等同于已验证 Codex 的跨应用兼容性。
- **最终构建的跨应用快捷键**：用户曾开启辅助功能，旧构建曾显示授权成功；重建后 macOS 没有沿用旧授权。准备固定版本实测时 Mac 被锁定，等待解锁和授权刷新。
- **Windows 运行**：只完成适配模块编译检查；本机不能验证 Windows UI Automation、焦点行为、快捷键冲突和安装包。随附 CI 尚未运行。
- **真实 LLM 服务**：接口、凭据存储和响应验证已实现；未提供服务地址、模型与 API Key，尚未进行真实服务联调。
- **Windows 默认翻译**：尚未选定通用默认引擎，当前可配置 LLM；不会把 Apple 系统翻译标为 Windows 可用。

## 用户验收步骤

1. 启动 `src-tauri/target/debug/bundle/macos/TranslateMe.app`。
2. 工作台点击“使用当前聊天示例”，再点击“翻译成英文”。
3. 在系统设置 → 隐私与安全 → 辅助功能中启用 TranslateMe；本地开发包重建后若授权失效，需刷新授权。正式分发需稳定的开发者签名。
4. 在聊天输入框输入一段中文，按 `⌘⇧E`；应得到英文，消息不应自动发送。可在输入框使用 `⌘Z` 撤销。
5. 选中英文，等待自动翻译，或按 `⌘⇧D`；应弹出中文译文，原文不变。
6. 在较长翻译进行中修改原文或切换应用；译文应仅展示供复制，不覆盖新内容。

测试时创建了独立的 TextEdit 文档“Untitled 3”，仅包含本次测试句；未修改原有文档。
