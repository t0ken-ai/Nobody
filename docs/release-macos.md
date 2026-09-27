# Nobody · macOS Release

面向开发者的翻译与局域网文字、文件互传工具。

## 安装与使用

- 当前包：`artifacts/releases/Nobody-0.1.0-macOS-arm64.dmg`，适用于 Apple 芯片 Mac；同目录提供 ZIP 和 SHA-256 校验文件。
- 打开 DMG，把 `Nobody.app` 拖到「应用程序」。升级前退出正在运行的旧版 TranslateMe / Nobody，避免两个进程争用快捷键和互传端口。
- 不需要安装 Node.js、Rust、Xcode、Python 或额外的 SQLCipher。
- 应用最低系统版本声明为 macOS 13；Apple 系统翻译需 macOS 15+，较旧系统使用自定义 LLM。首次使用系统翻译可能要下载语言包。
- 划词及快捷键回填需在系统设置中为当前 Nobody 授予辅助功能权限；翻译工作台和局域网互传不依赖这项权限。
- 两台 Mac 连接同一局域网、打开 Nobody，允许本地网络访问；首次核对双方校验码并信任，之后自动接收。文字和文件通过加密直连传输，文件夹不支持。
- 新电脑需要自己配置 LLM API Key。安装包不包含本机的密钥、已信任设备、聊天、传输记录或收到的文件，也不读取钥匙串。

## 签名与升级

当前是优化后的 Release 构建，采用本地 ad-hoc 签名，尚无 Developer ID 分发签名或 Apple 公证。因此从另一台电脑接收后，macOS 可能阻止首次打开；确认包来源后由使用者按系统提示处理，勿关闭系统安全保护。

品牌更名保留 Bundle ID `app.translateme.desktop`、数据根目录 `~/.translateme/` 与局域网协议，已有设置、设备身份及信任记录仍可读取，也可发现旧版 TranslateMe。无需手动移动数据目录。开发签名变化可能使旧的辅助功能授权失效，处理方法见主 README。

## 可复现构建

在安装好项目依赖的 Apple 芯片 Mac 运行：

```sh
node scripts/package-release.mjs
```

脚本调用 Tauri 默认的 Release profile（不传 `--debug`），使用已有优化／精简配置，只打包应用和这份说明，并生成 DMG、ZIP 及 SHA-256 清单。临时输出位于被 Git 忽略的 `artifacts/release-staging/`。
