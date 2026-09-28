# Nobody 图标资源

[返回 README](../../README.md)

标志以无面部特征的人物和小写 n 形轮廓表达 Nobody；主色为深绿与暖白。图案最初通过 AI 图像生成及参考图编辑制作，应用构建只使用仓库中的本地资源，不调用图像服务。

## 资源

| 文件 | 用途 |
| --- | --- |
| [b-app-icon.png](exports/b-app-icon.png) | 桌面应用图标和主界面彩色图标的源图 |
| [b-menu-template.png](exports/b-menu-template.png) | 菜单栏与译文浮窗使用的透明单色源图 |
| [b-symbol-black-transparent.png](exports/b-symbol-black-transparent.png) | 标准透明符号 |
| [b-symbol-black-on-white.png](exports/b-symbol-black-on-white.png) | 白底黑色符号 |
| [b-logo-system.png](exports/b-logo-system.png) | 图标应用参考板，不参与构建 |

README 使用 [nobody-logo.svg](../../docs/media/nobody-logo.svg)：内嵌标准透明符号的原始图像，通过透明蒙版适配深浅配色，避免将 App 图标的方形底色带入文档。

`concepts/` 保留设计源素材，不参与应用构建。实际使用的是 B / Human。

## 重建图标

安装项目依赖后，在仓库根目录运行：

```sh
node scripts/generate-icons.mjs
```

脚本从两个构建源图生成 `src-tauri/icons/` 和 `src/assets/` 的桌面资源；中间文件位于被忽略的 `artifacts/`。普通构建使用已提交的图标，无需再次生成。

菜单栏的单色版本增加头身留白，以适应小尺寸；不要直接用标准符号替换。macOS 按系统外观着色，Windows 托盘使用彩色图。桌面、界面与浮窗共享标志形态，菜单栏另有显示裁切以保持辨识度。
