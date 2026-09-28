# Nobody · Windows 安装说明

[返回 README](../README.md) · [macOS 安装](release-macos.md) · [兼容性范围](verification.md)

## 下载与安装

从 [GitHub Releases](https://github.com/t0ken-ai/Nobody/releases/latest) 下载 `Nobody-<版本>-Windows-x64-setup.exe`，适用于 Windows 10/11 的 x64 电脑。当前不提供 Windows ARM 原生包。

1. 升级前先从托盘退出正在运行的 Nobody。
2. 运行 `.exe`，按安装器提示安装；默认使用当前用户目录。
3. 缺少 Microsoft WebView2 时，安装器会联网下载运行时；使用已有 WebView2 的电脑不需重复安装。

安装器根据系统语言显示中文或英文，不要求预装 Node.js、Rust 或 SQLCipher。当前 Windows 包尚未配置 Authenticode 发布者证书，系统可能显示未知发布者提示；请核对来源及校验值，勿关闭系统防护。

## 首次使用

- Windows 使用 **自定义 LLM**，不提供 Apple 系统翻译。在偏好设置中配置端点、模型和密钥，测试连接后点击保存。
- 写入英文快捷键默认 `Ctrl+Shift+E`，手动阅读默认 `Ctrl+Shift+D`，均可修改；自动划词无需快捷键。
- 自动划词默认白名单为 ChatGPT Desktop、Claude Desktop，可以添加本机应用。特殊控件或权限等级不同的应用可能无法取词／回填。
- 局域网互传需要双方运行 Nobody 并完成首次信任；防火墙设置可能影响本地发现和接收。
- 关闭窗口后继续驻留托盘，从托盘菜单退出才停止后台工作。
- 「偏好设置 → 登录时启动」默认关闭，开启后下次登录 Windows 时在后台运行，不弹主窗口；切换立即生效。窗口底部或托盘中的「关于 Nobody」可查看版本、项目主页和作者邮箱。

## 自动更新

0.1.2 起提供 Windows 正式安装包及签名更新包。首次需要手动安装，以后可通过托盘「检查更新」或「偏好设置 → Nobody 更新」检查。

启动 30 秒后及每 6 小时自动检查正式版本，显示更新摘要；支持稍后提醒、跳过该版本及关闭自动检查。点击「更新并重启」后先下载并验证签名与版本，待翻译和互传空闲时进入安装器，安装完成后重新启动。Windows 更新安装器显示进度。

更新签名与 Authenticode 发布者证书是不同机制。用户数据保存在 `%USERPROFILE%\.translateme\`，更新不会迁移该目录。完整备份请参考 [本地存储](local-storage.md)。

## 校验

发布页提供对应 `.sha256` 文件。PowerShell 中计算安装包摘要，与清单中的同名行比较：

```powershell
Get-FileHash .\Nobody-<版本>-Windows-x64-setup.exe -Algorithm SHA256
```

发布流程执行 Windows 构建、Rust 测试、真实更新包验签、安装及更新模式覆盖检查。以对应版本的 Actions 成功结果为准；CI 不能替代真实应用中的划词、回填、托盘和双机互传验收。
