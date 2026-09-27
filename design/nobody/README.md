# Nobody · Logo

2026-09-28。用户选定 Nobody，并要求设计新 Logo。首轮使用内置 ImageGen 生成三个独立候选；用户随后明确纠正选择为 **B / Human**，强调匿名表达。用户确认「用 B 替换 App logo」后，已接入桌面图标、菜单栏、主界面及译文浮窗。随后按用户要求统一应用名称与描述为 Nobody，并准备 [macOS Release](../../docs/release-macos.md)；安装标识、数据目录与局域网协议保留原值，以兼容旧版配置和电脑。

## 已选 B：匿名人物与小写 n

以不包含五官、发型、服饰或身份标签的抽象人物表达匿名；下部保留小写 n 的不对称拱形。源图与不可更改的轮廓特征见 [B 方向锁定记录](selection-b.md)。

| 导出文件 | 用途 |
| --- | --- |
| [b-app-icon.png](exports/b-app-icon.png) | 1254 × 1254，深绿底反白符号、无字标的方形应用图标源图。 |
| [b-symbol-black-on-white.png](exports/b-symbol-black-on-white.png) | 1254 × 1254，白底黑色标志。 |
| [b-symbol-black-transparent.png](exports/b-symbol-black-transparent.png) | 1254 × 1254，黑色符号、真实 RGBA 透明背景；黑底预览可能看不清，应放在浅色背景查看。 |
| [b-logo-system.png](exports/b-logo-system.png) | 1536 × 1024，标志、反白、图标、留白与尺寸示意板。 |
| [b-menu-template.png](exports/b-menu-template.png) | 1254 × 1254，增加头身留白的透明单色小尺寸版本，用于 macOS 菜单栏及译文浮窗。 |

以上为内置 ImageGen 参考图编辑结果，完整提示词见 [export-prompts-b.md](export-prompts-b.md)。效果板里的匿名文案仅描述视觉理念，不代表应用具备匿名通信功能；色值标签与构图网格为设计意向，不等于对生成图逐像素的色彩或几何承诺。

### 检查与边界

- 已对照 B 源图检查圆形头部、头身间隔、细左腿／厚右侧、向下开放的留白；形态保持同一方向，没有引入 C 的符号或面具等额外身份元素。
- 透明 PNG 确有 Alpha 通道，背景与拱形开口样点 alpha 为 0；主体常见 alpha 为 254/255，属于接近全不透明的生成结果。以 128 为阈值，其轮廓与白底单色源图的交并比为 0.99332。
- 原始透明符号在 16／20／24 px 下头身会粘连，所以未直接用作菜单栏素材。通过内置 ImageGen 参考编辑生成独立 `b-menu-template.png`，只调整小尺寸留白。包装后的 64 px 图在 18／20／22／24／36／44 px 只读采样中均保持头、身两个连通区域；当前 macOS 托盘库按 18 pt 绘制，浮窗使用 20 CSS px。该版本不作为 16 px 图标使用。
- 主界面使用 128 px 彩色图，浮窗使用 64 px Alpha 蒙版并沿用正文颜色；macOS 菜单栏使用系统 template tint，Windows 托盘使用彩色图标。无外部图片请求、额外轮询或图片服务运行时依赖。
- 桌面 `.icns`／`.ico` 均来自已选定的深绿不透明方形源图。尝试生成圆角透明底板时出现内部透明破洞，两个失败变体均未接入。前端圆角由既有容器裁切；没有对 B 做代码描摹或 SVG 重建。

### 重建与验证

运行 `node scripts/generate-icons.mjs`，通过已安装的 Tauri CLI 重建桌面资源与前端小图；中间移动端资源仅留在忽略的 `artifacts/` 目录。已删除旧翻译符号的 `source.svg`，避免后续误用。

- `npm run build`、macOS 调试包构建、严格签名验证通过；包内 ICNS 与源资源逐字节一致。
- ICNS 含 16–1024 px 表示；ICO 含 16／24／32／48／64／256 px 表示，均能解码。
- 运行中的 macOS 主界面确认显示 B；独立 WebKit 夹具以生产 CSS 检查深浅色浮窗和 18 px 标志。原有辅助功能权限已刷新，重启后显示「跨应用翻译已就绪」。
- Windows 图标资源已准备，当前主机不能进行 Windows 实机验证。未因视觉资产替换重跑无关的网络或存储测试。

本轮内置 ImageGen 提示词及弃用原因见 [install-prompts-b.json](install-prompts-b.json)。当前不扩展成完整 VI 手册。

## 首轮候选（历史）

创意主题「语言不同，价值不减」是本轮的设计提案。产品背景是供开发者使用的 macOS／Windows 常驻工具，提供翻译与局域网文字／文件互传。

| 候选 | 图片 | 方向与评估 |
| --- | --- | --- |
| A · Equal | [a-equal.png](concepts/a-equal.png) | 厚实的 N 字母，辨识直接、小轮廓清晰；与普通字母标志相比个性仍偏弱。 |
| B · Human | [b-human.png](concepts/b-human.png) | 无面部、身份或性别特征的人物，下部借用小写 n 的形态；最贴近 Nobody 的人本含义。细腿与头身比例需在实际菜单栏尺寸下继续验证。 |
| C · Voices | [c-voices.png](concepts/c-voices.png) | 两个等重形体通过中间的留白与连接相遇；更偏抽象交流符号，较宽的外形和中部连接需做小尺寸验证。 |

首轮图片是提案展示板，板中的小图标与文字仅供比较。用户最终已选 B；C 的导出尝试在纠正选择后放弃，不包含于交付资产。

## 生成记录

- 工具：内置 `image_gen.imagegen`；每个候选一次独立调用。
- 背景参数：`transparent_background: false`（展示板）。
- 无参考图。每次输入为 [prompts.md](prompts.md) 的 Common 部分，加上对应候选段落。
- 原始图片留存在 `/Users/censor/.codex/generated_images/01a0df91-f972-7892-8dc3-ff6ef78a159e/`；工作区保存的是原文件副本，没有描摹、重画或裁切。

正式接入使用 B 的彩色源图与独立小尺寸符号；历史候选和展示板不参与应用构建。
