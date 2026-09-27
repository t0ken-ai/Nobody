# 本地凭据存储

用户选择从现在起不再访问系统凭据库，不导入旧密钥，重新填写 API Key、重新配对设备。macOS 与 Windows 使用相同文件布局，`~` 表示当前用户主目录。

| 路径（相对 `~/.translateme/`） | 用途 |
| --- | --- |
| `llm/llm.db` | SQLCipher 保存按接口隔离的 API Key 及最后保存的 LLM 配置 |
| `llm/master.key` | 随机 256 位解密密钥；先持久保存，再创建数据库 |
| `transfer/device/identity.json` | 本机 TLS 证书和私钥；首次创建后身份固定 |
| `transfer/device/trusted.json` | 新本地身份确认过的设备；首次配对成功后保存 |
| `transfer/inbox.json`、`transfer/settings.json` | 沿用原有记录、接收目录和互传设置 |
| `llm.db`、`transfer/trusted.json` | 旧版数据原位保留作备份；新版不解锁、不使用旧信任 |

应用不读取、导入、更新或删除 Keychain / Credential Manager 中的旧条目。`keyring` 依赖及旧 endpoint 密钥回退已移除。非密钥 JSON 配置仍可提供已有的模型、接口、提示词、快捷键等设置；无需重新填写这些项目。

新文件先写入受保护的临时文件，再原子发布，避免半写密钥或覆盖同时初始化的另一份密钥。macOS 目录权限为 `0700`、文件为 `0600`；Windows 使用当前用户 SID 的受保护 DACL，去掉继承的其他账户授权。系统凭据接口不参与文件权限设置。管理员或同账户程序仍可能取得文件；获得完整 `llm/` 目录的人可以解密 API Key，不应将本地加密描述为独立密钥保管。

重装应用时保留 `llm/` 整个目录即可恢复 LLM 设置和密钥。只保留 `llm.db` 不够。恢复同一台设备的互传身份时应整体保留 `transfer/device/`；多台电脑不要同时使用同一份设备私钥，应各自生成身份并配对。新目录中密钥缺失、损坏或与数据库/证书不匹配时返回错误，保留原文件，不自动重置。更换到新本地身份后，旧的收发记录与已接收文件保留，但设备双方需要重新确认信任。

验证：53 项 Rust 测试通过，覆盖本地重新打开、旧库不改写、丢失/损坏密钥、错误解密密钥、权限、符号链接、旧信任隔离和 TLS 配对/传输；3 项需要显式执行的外部测试默认忽略。Windows 原生适配层和新文件 ACL 代码通过 `x86_64-pc-windows-gnu` 编译检查，尚未做 Windows 真机 ACL/重启验证。

macOS 最终应用已直接启动、保存原有非密钥设置到新数据库，并两次重启同一构建，没有触发钥匙串验证。新数据库、主密钥和设备身份重启前后哈希一致；旧数据库、旧信任、互传设置和历史的哈希保持不变。主程序未引用 `SecKeychain*`、`SecItemCopyMatching/Add/Delete/Update` 等凭据读写符号。同一应用的开发签名辅助功能记录已通过系统设置刷新，最终界面显示“跨应用翻译已就绪”。本轮没有重新填写真实 API Key 或替用户确认新设备信任。

Apple Silicon 测试包为 `artifacts/TranslateMe-macOS-arm64.zip`，签名校验与 ZIP 完整性检查通过。SHA-256：`1b8aa546a029aa6d174b323a411515452aafcc23f04a42b69971c421fed4b6fb`。
