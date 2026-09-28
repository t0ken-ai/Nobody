<p align="center">
  <img src="src/assets/nobody-icon.png" width="96" alt="Nobody logo">
</p>

# Nobody

**语言不同，价值不减。**

Nobody 是面向开发者的桌面翻译与局域网互传工具。用母语写下想法，通过快捷键转成英文；划选外语内容，直接阅读译文；在自己的电脑之间发送文字和文件。

Translation and local-network sharing for developers.

[使用指南](docs/usage.md) · [macOS 安装](docs/release-macos.md) · [开发指南](docs/development.md) · [参与贡献](CONTRIBUTING.md) · [MIT 许可证](LICENSE)

## 功能

- **快捷键写入英文**：翻译选中文字或当前输入框，核对原内容后回填，不自动发送。
- **划词阅读**：在允许的应用中选中文字，松开后显示译文，无需快捷键。
- **跟随浮窗**：优先显示在选区上方，支持拖动、长文滚动和系统深浅外观；macOS 关闭浮窗后返回来源应用。
- **两种翻译引擎**：macOS 系统翻译，或兼容 Chat Completions 的自定义 LLM。支持编辑翻译角色与规则、测试连接和 Markdown 展示。
- **局域网互传**：自动发现在线设备，首次双方确认信任，之后加密直传文字和多个文件，无云端中转。
- **后台常驻**：关闭主窗口后仍可翻译和接收文件，通过菜单栏或系统托盘打开界面、进入互传或退出。

## 平台支持

当前版本处于早期阶段，主要在 Apple 芯片 Mac 上验证。Windows 已有适配代码，尚未完成整应用实机验收。

| 平台 | 系统翻译 | 自定义 LLM | 验证状态 |
| --- | --- | --- | --- |
| macOS | macOS 15+，首次使用可能下载语言包 | 支持 | Apple 芯片 Mac 已验证主要流程 |
| Windows | 不提供 Apple 系统翻译 | 支持 | 原生适配模块已交叉编译，实机待验证 |
| Linux | — | — | 暂未支持 |

macOS 应用最低版本声明为 13.0；13/14 需要使用 LLM，旧系统及 Intel Mac 仍待实机验证。能力边界见 [兼容性与验证范围](docs/verification.md)。

## 快速开始

### 安装

macOS 安装包为 `Nobody-<版本>-macOS-arm64.dmg`，仅适用于 Apple 芯片。打开后将 Nobody 拖入「应用程序」，升级前先退出旧版本。也可使用 ZIP 中的应用。

当前打包配置使用 ad-hoc 签名，尚未进行 Developer ID 签名与 Apple 公证。安装、权限及首次打开问题见 [macOS 安装说明](docs/release-macos.md)。仓库不包含构建产物；从源码构建见下文。

### 翻译

1. macOS 跨应用取词与回填需要授予 Nobody 辅助功能权限；只使用翻译工作台不需要这项权限。
2. 在「偏好设置」选择系统翻译，或配置自定义 LLM，点击 **保存设置**。**测试连接不会保存或切换实际引擎**。
3. 在输入框中按写入快捷键，或在白名单应用正文中划选文字阅读译文。

| 操作 | macOS | Windows |
| --- | --- | --- |
| 将输入翻译成英文 | `⌘⇧E` | `Ctrl+Shift+E` |
| 手动翻译选中文字 | `⌘⇧D` | `Ctrl+Shift+D` |
| 自动划词翻译 | 选中文字后松开 | 选中文字后松开 |

快捷键可修改。自动划词默认仅允许 **ChatGPT Desktop、Claude Desktop**，可添加本机其他应用；使用 Codex 时需手动加入白名单并保存。阅读目标语言默认为简体中文。

### 局域网互传

两台电脑连接同一局域网并运行 Nobody，在「局域网互传」中选择设备，输入文字或添加文件。首次发送前核对两端校验码并分别确认信任，之后自动接收。支持文字及单个／多个文件，暂不支持文件夹。

默认接收目录为 `~/.translateme/received/`，可在界面修改。网络隔离、防火墙或 VPN 可能阻止发现，更多说明见 [使用指南](docs/usage.md#局域网互传)。

## 隐私与数据

- 系统引擎使用 Apple Translation；LLM 将完整选中文字发送到你配置的接口，包含其中的代码。系统翻译失败不会自动改用在线服务。
- 不截图、不 OCR，不读取密码框，不保存翻译历史，也不记录原文、译文或 API Key。
- LLM 配置、密钥及互传数据位于用户主目录 `~/.translateme/`；不使用 macOS Keychain 或 Windows Credential Manager。
- API Key 保存在 SQLCipher 数据库中，解密密钥存放在同一目录。**获得完整目录即可解密，不能将其视为独立的密钥保险库。**
- 局域网传输使用 TLS 1.3，首次信任前不发送正文和文件内容。传输记录和接收文件不做 SQLCipher 静态加密，收到的文件不会自动执行。

目录布局、备份和旧版迁移见 [本地存储](docs/local-storage.md)。

## 从源码运行

需要 Node.js 22+、Rust stable，以及对应平台的构建工具：macOS 使用 Xcode / Command Line Tools；Windows 使用 Visual Studio C++ Build Tools、Perl 和 WebView2。

```sh
npm ci
npm run desktop
```

构建桌面应用：

```sh
npm run bundle
```

技术栈为 **Tauri 2 + Rust + TypeScript**，macOS 原生能力通过 Swift 适配。开发环境、测试命令、打包与模块职责见 [开发指南](docs/development.md)。

## 贡献与许可

欢迎提交可复现的 Bug、兼容性反馈和 Pull Request。提交前请阅读 [贡献指南](CONTRIBUTING.md)，特别是 Windows 实机验证和代码混排翻译的边界。

项目采用 [MIT License](LICENSE)。
