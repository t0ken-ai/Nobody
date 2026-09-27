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

## 语言包重复提示修复

用户反馈每次划词都像在重新下载语言包。检查旧代码发现，每次请求都创建“正在准备系统翻译 / 首次使用可能需要下载语言包”的窗口，并无条件调用 `prepareTranslation()`；所有异常还统一追加“首次使用请等待语言包下载完成后重试”。本次读取旧版译文窗口，实际错误为 `Something went wrong. Please try again later.`，下载提示来自应用追加文字，不能据此判断系统真的再次下载了模型。

修复仅涉及原生 macOS 翻译模块：每次请求通过 `LanguageAvailability` 查询语言组合；已安装且运行于 macOS 26+ 时使用 `TranslationSession(installedSource:target:)`，不创建准备窗口；缺少语言包时保留系统下载确认，旧系统保留 SwiftUI 会话。不支持的语言组合直接报错，其余异常保留真实原因；没有新增跨模块接口或存储。Apple 文档说明，[已安装或正在下载时，prepareTranslation 不会重复提示下载](https://developer.apple.com/documentation/translation/translationsession/preparetranslation())；[无界面会话只适用于已安装语言](https://developer.apple.com/documentation/translation/translationsession/init(installedsource:target:))。

验证结果：

- 生产前端和最终 `.app` 构建成功，原有 7 个 Rust 测试通过；最终包的 `codesign --verify --deep --strict` 通过，保留 hardened runtime。
- 诊断程序 `artifacts/translation-repeat-check/main.swift` 直接调用生产 Swift C 桥接入口，连续执行两次中文→英文、两次英文→简体中文。其中两次为批量多段文本；四次都检查到 `installed`，都返回非空译文，整个程序没有创建窗口。诊断只使用固定测试句，不读取其他应用。
- 在最终 `.app` 工作台连续翻译“请在请求失败时显示错误信息，并允许用户重试。”与“更新结果时，请保留现有数据。”，真实返回 `Please display an error message when the request fails and allow the user to try again.` 和 `When updating the results, please keep the existing data.`。
- 最终构建 cdhash 为 `300e39ee8300095d8e274cea73245e78b1dd5ca3`。更新后旧开发签名授权失效，已通过系统设置移除旧 TranslateMe 项、重新添加同一路径并重启；界面确认“跨应用翻译已就绪”。未修改系统权限数据库或安全保护。
- 尚未实测缺少语言包的下载分支及 macOS 15–25 兼容分支，未删除已安装语言包。此次构建的 Codex 划词仍需用户复试，不能把工作台验证当作跨应用端到端验证。

## 跟随选区浮窗（用户已确认方案）

用户确认了选区上方、半透明背景、可拖动、滚动跟随和空间不足时避让的方案。实现沿用现有结果窗口与翻译引擎，新增 `popover.rs` 隔离浮窗状态与坐标计算；原生取词仅增补选区几何，不涉及尚待确认的代码结构识别方案。

实现内容：

- macOS 获取范围矩形，必要时兼容 Chromium 文本标记范围；坐标不足时使用触发时的指针位置。Windows 读取 UIA 的可见行矩形并释放 SAFEARRAY，使用目标显示器的 DPI 和工作区。
- 浮窗宽度 380 逻辑像素，优先在选区上方留 8 像素，顶边空间不足时改放下方；高度按内容和可用空间限制，长文滚动。
- 使用无系统标题栏的磨砂结果窗口，顶部拖动、本次固定，下次选区重新跟随；关闭后不会因同一选区再次弹出。复制按钮与正文选取不触发拖动。
- 翻译过程中继续观察选区位置，新选区会使旧任务的显示结果失效。没有选区的“翻译整个输入框”保留原有写入校验，不会被阅读轮询取消。
- 配置跨桌面显示，并在 macOS 使用 `fullScreenAuxiliary` / `canJoinAllApplications` 让浮窗具备加入其他应用全屏空间和 Stage Manager 集合的能力；最终体验仍需要目标应用实测。

验证与已知限制：

- TypeScript 检查、前端构建、原生 Swift 静态链接和 `.app` 打包通过。12 个 Rust 测试通过，其中新增测试覆盖上下避让、屏幕边缘、负坐标显示器、2 倍 DPI、关闭/拖动、新旧请求隔离、滚动离屏、鼠标回退及整框写入。
- Windows 适配模块再次通过 `x86_64-pc-windows-gnu` 编译检查；没有 Windows 实机，不能声称已验证磨砂、拖动或 UIA 几何。
- 创建了独立测试应用 `artifacts/popover-fixture/TranslateMeFixture.app`，只包含固定英文测试句。原生 UI 工具可以在后台选中文字，但不能把它当作真实前台划词触发；没有通过脚本操作 Codex 窗口。
- 第一轮用户实测反馈“没有出现浮窗”。随后在 TranslateMe 自己的工作台看到相应问题列表的中文译文，确认取词和翻译已完成，显示问题仍存在。
- 针对这个反馈，修正了几何降级：零尺寸矩形按不支持处理；不再用可能与选区无关的焦点容器裁剪矩形；首次位置不可确认时先使用指针回退，只有取得过可靠位置后才按离屏隐藏。另补齐跨桌面/全屏行为。
- 修订版已构建并打开，通过系统设置刷新同一应用的开发签名授权，工作台确认“跨应用翻译已就绪”。用户在后续复试中明确回复“出现了”，浮窗显示已得到用户确认；工作台同时显示测试句的真实译文“加载时请保持当前数据可见。”。精确位置、磨砂观感和手动拖动仍由用户继续体验，不能把显示成功扩大为所有交互均已验收。
- 排查期间准备过只记录几何的临时诊断，但在它进入运行中的 `.app` 之前用户已确认显示成功，因此已撤销诊断源码，保留当前已授权构建运行。

## 主界面与操作动画

主界面改为顶部导航，打开应用先进入“使用演示”。两段动画使用明确标注的固定样例，不连接翻译服务，不读取用户输入，不派发系统按键；工作台与设置保持原有功能，实际译文浮窗样式没有改动。

- 写入演示依次展示中文逐字输入、组合键下压、英文回填。键帽读取已保存的写入快捷键，并按平台显示 Command / Ctrl。
- 用户指出划词不需要快捷键后，阅读演示改为鼠标按住拖选、松开、选区稳定、自动显示译文；移除阅读卡里的键盘，保留设置中的手动触发快捷键。演示明确说明对应“自动划词”开启的情况。
- 动画支持独立暂停、重播；切换到工作台/设置或页面不可见时停止调度。“减少动态效果”偏好下默认显示静态完成画面，用户可主动播放一次。
- `npm run check`、生产前端构建、最终 `.app` 打包、`codesign --verify --deep --strict` 与 `git diff --check` 通过。改动仅涉及前端与说明文件，没有新增翻译引擎或原生接口。
- browser-act 检查了逐字输入、按键按下时替换、暂停/重播、隐藏页面停止动画、切换页面保留草稿、LLM 设置字段显示。通过同源 iframe 检查 1060×780 与 800×630 布局，无横向溢出；模拟 Windows 平台得到 Ctrl 键帽，模拟减少动态效果得到静态结果。平台与偏好模拟不等同于 Windows 实机验收。
- 阅读修订版检查：拖选时左键高亮且没有译文；松开后选区完整、等待稳定；之后自动显示译文。阅读卡中没有键帽。最终浏览器截图为忽略目录中的 `artifacts/ui-demo-final.png`；检查到浮窗与选区间距 8 像素。
- 最终 macOS 应用已打开，原生窗口截图确认两段动画与底部状态完整显示。刷新同一应用的开发签名授权后，界面显示“跨应用翻译已就绪”；设置中的“划词自动翻译”为开启状态。
- 在最终应用工作台实际翻译“请在请求失败时显示错误，并允许重试。”，Apple 系统返回 `Please display an error when the request fails and allow a retry.`，与固定演示文案不同，确认工作台仍调用真实引擎。应用最后停留在动画首页，分发 ZIP 已更新。

## 自动划词白名单与误触发过滤

用户确认默认仅为 Codex 和 Claude Desktop 启用自动划词，并明确 Claude 指桌面应用。通过本机 Info.plist 核实 macOS Bundle ID 后接入原生识别；没有按窗口标题匹配或将终端/Claude Code URL Handler 加入白名单。

- 原生层在新自动请求读取选中文字前检查应用身份，再检查焦点控件与祖先的编辑/对话框语义。明确的手动请求仍可使用；浮窗跟踪只放行原 PID 与原控件，不能给其他自动请求扩大权限。
- 自动触发要求最近完成的鼠标拖选或双击选词，再等待选区稳定；不监听键盘内容。旧选区、已消费动作、拖选中的停顿，以及已识别的纯路径/数字/代码不会产生自动请求。macOS 使用鼠标事件元数据；Windows 在原 COM 线程内采样，遇到很短动作或缓慢 UIA 调用可能漏触发，使用手动入口兜底。
- 19 个 Rust 测试通过，新增内容覆盖默认白名单迁移与空列表、动作只消费一次、忽略上下文、拖选与焦点变更、忙碌状态、纯技术文本过滤，以及精确手动来源跟踪。TypeScript 检查、生产前端构建、Swift 静态链接、macOS 打包、签名校验与 Windows 适配模块交叉编译通过。
- 最终原生设置界面显示 Codex 和 Claude Desktop 默认均勾选。实际取消 Claude Desktop 并保存，再恢复并保存；读取本应用设置文件确认 `autoSelection: true`、`automaticApps: ["codex", "claude"]`。分发 ZIP 已更新。
- 最终应用工作台真实翻译“请在失败时允许重试。”，Apple 系统返回 `Please allow retry in case of failure.`，确认白名单不会限制独立工作台。
- 更新应用后旧开发签名授权失效。系统在移除旧记录时要求 Touch ID；截至此记录，等待用户亲自在系统对话框验证，尚未完成本构建的授权刷新。未代填密码、修改 TCC 数据库或新增系统权限。
- 自动触发规则单元测试不等于 Codex / Claude Desktop 中真实拖选、保存框与输入框的端到端验收。需要在最终构建获授权后分别验证；未用工具操作 Codex，也未声称 Windows 已有实机结果。

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
