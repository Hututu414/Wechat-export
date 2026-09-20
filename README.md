# 微信聊天导出 · wechat-export

Windows x64 原生 Rust 小工具：选择联系人/群聊和时间，导出 JSONL 或 JSON。无需 Python、Node、浏览器或第三方 DLL。

**本机已验证微信 4.1.13.65；其他 4.x 补丁版本为 UNVERIFIED。** 当前实现不依赖 TraceMemo 的闭源 DLL，研究结论和替代实现的许可边界见 [参考调查](docs/reference-analysis.md)。

## 使用

1. 解压 `wechat-export-windows-x64.zip` 到可写文件夹，运行 `wechat-export.exe`，保持微信已登录。
2. 检查数据目录和账号，点击“连接”。支持 `xwechat_files`、账号目录或 `db_storage`；本机的 `D:\xwechat_files` 可自动发现。
3. 选择全部或多个会话、时间范围、输出目录和格式。在“导出字段”中勾选需要的内容，点击“统计消息”，核对数量后“开始导出”。
4. 导出时可取消。重新连接会刷新数据副本；一次连接内的预览和导出使用同一份副本。

原微信 DB/WAL 只以只读文件句柄读取；仅使用进程查询和读取权限，不 Hook、不注入、不暂停、不发送消息。产品没有联网、遥测或 AI 功能。

工具在 EXE 同目录的 `.wechat-export-work` 中创建仅当前运行用户可访问的临时目录。正常断开、取消连接或退出时删除副本；异常断电/强制结束后可能遗留该工具目录，关闭程序后可手动删除。日志不含 Key 或聊天正文。需要为数据库副本及导出文件预留磁盘空间。

## V2：选择导出字段

默认精简输出只有 `datetime`（发送时间，含时区）、`sender_name`（发送者名称）、`type`（消息类型）、`content`（正文）。JSON 和 JSONL 使用同一组勾选，不附带未选中的字段。

“更多字段”可选择会话名称/ID、发送者 ID、是否本人发送、Unix 时间戳（秒）、消息 ID、媒体信息、引用消息、原始类型编号、原始 XML、原始载荷和解析警告。“精简”恢复默认四项，“全选字段”恢复 V1 的完整输出。至少需要选择一项；多会话导出时可加选“会话名称”方便区分来源。

选择只影响输出字段，不改变消息数量、时间范围、排序或去重。缺失的发送者等仍为 `null`；`raw_xml`、`raw_payload`、`warnings` 没有内容时仍省略。需要保留未知消息的原始数据时，请勾选对应原始字段或使用全选。

## 数据约定

- 日期使用本机时区：`开始日 00:00 <= 时间 < 结束日次日 00:00`。近 7/30/90 天包含今天。
- 导出按 `conversation_id` 分组，组内按时间、微信排序序号及稳定编号升序排列；不是跨会话混排。
- 相同会话的非零 `server_id` 跨分片去重；缺失服务器编号时用分片编号和 `local_id` 区分，不误删相同正文。
- JSONL 每行一个消息；JSON 也是流式数组。时间和会话范围在 SQL 读取前约束，COUNT 与导出共用去重规则。
- 完整字段包括 `id`、会话、发送者、`is_self`、时间、`type`、正文、原始类型、媒体元数据和引用；实际输出以勾选项为准。发送者无法确定时相关字段为 `null`，勾选解析警告后可查看原因。
- 媒体只保存可取得的路径、文件名、hash 和元数据，不复制或解码媒体。部分媒体位置仅有未解释的 `packed_info_base64`。
- 不支持的子类型、异常 XML 或无法解码的内容保留为 `unknown`，不丢弃消息；选中的 `raw_xml` / Base64 原始字段会随消息输出。红包、转账等不是本版专用解析器的范围。
- 当前最多读取 125 个消息分片。单条压缩内容解码上限 64 MiB，超限保留原始载荷。
- 导出先写独立 `.partial` 文件，成功后发布为最终文件；取消/错误删除本次临时输出，不覆盖已有文件。

## 开发与验证

要求 Windows x64、Rust stable、MSVC 与 Windows SDK，以及 Cargo.lock 对应依赖。生产逻辑只有 Rust；PowerShell/Python 脚本仅用于开发打包、许可证整理和图标格式转换。

```powershell
cargo test --locked --all-targets
cargo build --locked --release
./scripts/package.ps1 -NoBuild
```

本次开发使用已有本地缓存离线构建，没有安装全局包。该工作区可通过 `.local/cargo-run.ps1` 运行同样的 Cargo 命令。`.cargo/config.toml` 固定静态 CRT 和 SQLite 分片上限。

核心位于 `src/core`，不依赖 egui API；GUI 在 `src/ui`，worker 线程独占数据库连接，通过消息通道返回结果。

验收范围、真实数据与合成基准的区别见 [验证报告](docs/validation.md)。运行只需要 EXE；再分发时请同时保留 [第三方说明](THIRD_PARTY_NOTICES.md) 和 `licenses/`。图标源文件及生成说明在 `assets/`。
