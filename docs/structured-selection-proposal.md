# 结构化划词方案（待确认，尚未实施）

## 已确认的问题

当前原生适配器只返回 `AXSelectedText` 字符串，没有返回选区内的代码边界。共享解析器依赖 Markdown 围栏、反引号、路径和标识符等规则。渲染后的选区丢失围栏时，这些规则不能保护完整代码块和命令。

使用本次聊天的英文 TypeScript / Git 样例做本地对照，调用生产 `Document::parse`，没有访问 Codex 窗口或翻译服务：

| 输入 | 进入翻译的片段数 | 检出的代码/命令泄漏片段数 |
| --- | ---: | ---: |
| 含围栏的 Markdown | 3 | 0 |
| 模拟去掉围栏、带语言标签及对象占位符的渲染选区 | 14 | 7 |

泄漏片段包括 `async function`、`const response = await fetch(`、`git add` 和完整 `git commit -m ...`。模拟输入并非从 Codex 窗口实际采集，不能据此断定该应用具体暴露了哪些属性。实验位于忽略目录 `artifacts/selection-repro/`；运行方法：`CARGO_TARGET_DIR="$PWD/src-tauri/target" cargo run --offline --quiet --manifest-path artifacts/selection-repro/Cargo.toml`。

## 建议的行为

仅获取用户在前台应用选择的内容及对应的文本结构。显示“原文 / 译文”对照；说明文字翻译为目标语言，代码、命令及其注释和字符串保持原样，列表及段落边界尽量保留。

| 选中的内容 | 期望结果 |
| --- | --- |
| `The API request should be cancelled when the user leaves the page.` | 翻译为中文 |
| 完整 `async function fetchUser(...) { ... }` 代码块 | 逐字符保留，包括注释和错误消息 |
| `git commit -m "fix: support cancellation for user API requests"` | 整条命令保留，包括提交说明 |
| 问题列表的自然语言说明 | 翻译文字，保留顺序和编号 |
| 单独选择代码块中一句注释 | 优先遵循选区的代码语义；需要解释时另用工作台明确请求 |

## 最小接口调整

沿用现有原生适配器、共享解析器、翻译引擎和结果窗口，不添加新服务、存储或第三方引擎。

取词结果保留现有 `text`，另携带由原生适配器提供的、相对于该字符串的受保护范围及结构来源。macOS 原生范围使用 UTF-16 单位；共享层先验证范围顺序、边界和代理对，再转换为 Rust 字节范围。文本与结构不一致时拒绝套用范围，不能将不确定范围用于回填。

影响文件：`native/macos/Native.swift` 读取选区结构；`src-tauri/src/main.rs` / `translation.rs` 传递结构；`document.rs` 在拆分正文之前保护完整范围；`src/main.ts` 利用已有的 `Result.source` 展示原文对照。Windows 的结构能力需单独实现与实机验证，不能把 macOS 的验证结果外推。

## 系统能力探测与降级

先在独立的原生/渲染测试窗口验证选区范围及 attributed text 接口，再由用户在 Codex 手工验收。公开 Accessibility 范围接口存在，但不能保证每个应用都暴露足够的代码语义；字体或等宽样式也不能单独证明一段文字是代码。

Chromium 源码包含跨节点的选区和 attributed text 实现，可作为兼容性研究依据。它不能证明当前 Codex 版本支持同样的结果，更不等于能拿到原始 Markdown。若必须使用不稳定的平台扩展，先记录支持范围与失败方式，不以成功读取纯文字冒充结构完整。

只有结构足以确定代码边界时，才自动翻译混合内容。若检测到代码但结构不足，保留原文并明确提示通过“复制原始 Markdown”到工作台继续；不把语法猜测当成完整解析。不会为了找 Markdown 扫描聊天数据库、扩大到整窗内容、注入页面脚本或改用截图 OCR。

## 验收条件

使用本次完整样例及部分代码选区：代码块、命令、注释、字符串均不进入引擎输入，恢复后逐字符一致；普通说明仍能翻译。补测跨节点选区、Emoji / 中文的 UTF-16 偏移、失效范围、只有纯文本的目标应用，以及回填前原文/选区变化的保护。

## 依据

- 当前实现：`native/macos/Native.swift` 的 `capture` 与 `src-tauri/src/document.rs`。
- [Apple 选区范围接口](https://developer.apple.com/documentation/applicationservices/kaxselectedtextrangeattribute)。
- [Chromium 选区与 attributed text 实现](https://chromium.googlesource.com/chromium/src/+/4f95b302f521a2474bebf4e1e9d8337c1449bafe/content/browser/accessibility/browser_accessibility_cocoa.mm)。

用户提供的 AGENTS.md 要求架构方向及跨模块协同改动先确认；本方案涉及选区数据接口，确认前不修改运行中的应用。
