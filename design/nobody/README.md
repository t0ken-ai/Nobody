# Nobody · Logo 候选

2026-09-28。用户选定 Nobody，并要求设计新 Logo。本轮使用内置 ImageGen 生成三个独立候选；尚未选定最终标志，未替换现有应用图标或修改应用标识、安装路径和数据目录。

创意主题「语言不同，价值不减」是本轮的设计提案。产品背景是供开发者使用的 macOS／Windows 常驻工具，提供翻译与局域网文字／文件互传。

| 候选 | 图片 | 方向与评估 |
| --- | --- | --- |
| A · Equal | [a-equal.png](concepts/a-equal.png) | 厚实的 N 字母，辨识直接、小轮廓清晰；与普通字母标志相比个性仍偏弱。 |
| B · Human | [b-human.png](concepts/b-human.png) | 无面部、身份或性别特征的人物，下部借用小写 n 的形态；最贴近 Nobody 的人本含义。细腿与头身比例需在实际菜单栏尺寸下继续验证。 |
| C · Voices | [c-voices.png](concepts/c-voices.png) | 两个等重形体通过中间的留白与连接相遇；更偏抽象交流符号，较宽的外形和中部连接需做小尺寸验证。 |

图片是提案展示板，板中的小图标与文字仅供比较，不是可直接发布的应用图标。最终菜单栏版本应只含符号；待用户选定源图后，再以该图进行透明背景、单色及应用图标输出，并检查形状一致性。本轮不扩展成完整 VI 手册。

## 生成记录

- 工具：内置 `image_gen.imagegen`；每个候选一次独立调用。
- 背景参数：`transparent_background: false`（展示板）。
- 无参考图。每次输入为 [prompts.md](prompts.md) 的 Common 部分，加上对应候选段落。
- 原始图片留存在 `/Users/censor/.codex/generated_images/01a0df91-f972-7892-8dc3-ff6ef78a159e/`；工作区保存的是原文件副本，没有描摹、重画或裁切。

下一步由用户选择 A／B／C，或指定修改、组合或重新探索。
