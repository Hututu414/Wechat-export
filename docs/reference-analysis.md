# TraceMemo 微信 4.x 数据访问调查

调查日期：2026-09-19。范围：Windows x64、本机账号发现、数据库 Key、加密数据库、联系人/会话/消息与结构化导出。本文先于任何产品实现生成。

## 结论与证据等级

**Phase 0 已完成，随后按获许可的替代路线完成 Rust 核心、GUI 和 Windows release。** 当前 TraceMemo 公开内容足以确认 TypeScript 调用链、部分 schema 和解析行为，不能完整审计其原生密钥获取、密码参数、数据库打开权限及 WAL 行为。下列阻塞针对 TraceMemo 直接复用；文末给出替代实现及真实验证结果，完整验收边界见 [validation.md](validation.md)。

- `SOURCE-CONFIRMED`：在下列固定提交的可读源码中确认，只说明源码行为。
- `RUNTIME-VERIFIED`：必须注明实测范围；文末记录微信 4.1.13.65 的真实读取验证，不将其推广为所有 4.x 版本均兼容。
- `UNVERIFIED`：原生内部实现、未跑通的路径、未取得授权或未测的兼容性。
- 下文“本机元数据实测”和“PE 静态检查”分别指目录/文件头检查、二进制导入导出表检查；不提升为真实消息读取成功。

主要阻塞：

1. 两个分支均未找到覆盖 TraceMemo 主体和关键 Windows DLL 的明确许可证；现有局部 MIT / GPL 许可证不覆盖这条链。
2. 数据库 Key 使用 `wx_key.dll` 的 Hook 接口。该 DLL 静态导入 `WriteProcessMemory`，没有对应源码可验证执行分支和最小权限。不能将其当作经过证明的纯 `ReadProcessMemory` 实现。
3. `wcdb_open_account` 没有公开 read-only 参数；包装层还提供实际发出建表/触发器 SQL 的可选路径。禁用该功能不能证明 DLL 打开数据库时绝不写入。
4. 密码算法参数、各 shard 的 Key 关系、WAL 合并和数据库内表名的一部分在未公开的 native 层，不能凭旧教程填补。

以上是本项目要求与当前证据的差距，不是断言 TraceMemo 所有普通导出都会写库，也不是断言该软件一定发起网络请求。

## 固定参考版本与研究方式

| 项目/分支 | 精确提交 | 提交时间 / 状态 |
| --- | --- | --- |
| TraceMemo develop | `a9f264107b2d6c57aca4f1488c397fc26cb07e62` | 2026-09-18 14:41:50 +08:00；本次主要分析对象 |
| TraceMemo main | `276fb72bc9bb8b4757ba269e9da3fc01b09e81f7` | 2026-09-15 16:47:56 +08:00；package version 2.4.0 |
| WeFlow main / HEAD | `318d5ac4a968eaef9a5cbd7c663151b6ba2efcf0` | 本次 `git ls-remote` 核对 |

通过 `git ls-remote` 固定分支；只在工作区 `.reference/TraceMemo` 稀疏检出相关源码、文档和三个 DLL，未安装或运行上游。完整 develop 文件树共 776 个文件；没有 Cargo.toml、Rust/C/C++ 实现或 native 构建项目。`main` 同样未提供这些原生实现。使用 `rg` 追踪真实入口、读具体函数、比较两个提交；使用 GNU objdump 2.45 静态检查 PE。

develop 相对 main 的核心差异包括 `wcdb4-client.ts`（337 行新增、1 行删除）、`message-parser.ts`（88 行新增、7 行删除）及 `wcdb_api.dll` 更新。Windows Key 包装、目录发现、账号发现、`wechat-db.ts` 在这两个提交间没有差异。不能只按 main 的系统消息解析或旧 DLL 得出 develop 的结论。

已读 README；docs 的 getting-started、privacy、export、development/overview、how-it-works、wechat-system-message-parsing 和相关第三方说明。package.json 确认上游为 Electron/React/TypeScript，相关底层依赖是 Koffi、fs-extra、fzstd 和随包 DLL。没有可直接作为 Rust crate 引入的核心。

关键源码索引（链接均固定到 develop 提交）：

| 代号 | 源文件与重点位置 |
| --- | --- |
| S1 | [index.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/index.ts#L940)：db:init、account discovery、key:autoGetDbKey（1120 附近） |
| S2 | [settings-store.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/services/settings-store.ts#L53)：默认目录与 validateDbRoot |
| S3 | [windows-db-root-discovery.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/windows-db-root-discovery.ts)：磁盘目录扫描辅助函数 |
| S4 | [account-discovery.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/services/account-discovery.ts)：候选账号枚举 |
| S5 | [local-account-identity.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/services/local-account-identity.ts)：本地 profile 解码与目录身份匹配 |
| S6 | [key-service-win.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/key-service-win.ts#L124)：DLL 绑定、进程/窗口发现、autoGetDbKey（626 附近） |
| S7 | [wcdb4-client.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/wcdb4-client.ts)：native ABI、shard、SQL、sender、解压、群成员 |
| S8 | [wechat-db.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/wechat-db.ts)：WechatDb.create 与导出读入口 |
| S9 | [chat-service.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/services/chat-service.ts#L215)：类型低 32 位、发送者、消息格式化 |
| S10 | [message-parser.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/message-parser.ts#L125)：消息类型、XML、引用、系统模板 |
| S11 | [export-service.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/export-service.ts#L649)：读入数组与 JSON.stringify |
| S12 | [database-key-store.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/database-key-store.ts)：Key 格式与账号绑定保存 |
| S13 | [wcdb-message-shards.test.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/tests/unit/wcdb-message-shards.test.ts)：跨 shard、重复 local_id、公众号与中文路径的 mock 测试 |
| S14 | [connection-diagnostics.ts](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/src/main/services/connection-diagnostics.ts)：进程版本检测 |

### 最小依赖链

```text
settings-store / account-discovery / local-account-identity
    -> 用户选定 accountRoot
key:autoGetDbKey -> KeyServiceWin.autoGetDbKey
    -> Koffi -> wx_key.dll -> InitializeHook / PollKeyData / CleanupHook
    -> testConnection(所选账号) -> 可选 DatabaseKeyStore
db:init -> WechatDb.create -> Wcdb4Client.openAsync
    -> resource-paths -> WCDB.dll / wcdb_api.dll（另有 SDL2 预加载尝试）
    -> InitProtection -> wcdb_init -> wcdb_open_account(session.db, key)
    -> get_sessions / get_display_names / get_group_members / get_group_nicknames
    -> open_message_cursor / fetch_message_batch
    -> get_message_table_stats + exec_query（补历史 shard）
    -> normalizeMessage -> chat-service -> message-parser
    -> export-service（上游导出不是全程 streaming）
```

## 1. 如何定位 Windows 微信数据目录

`SOURCE-CONFIRMED`（S2、S7）：默认候选为用户 home 下的 `Documents/xwechat_files` 和 `AppData/Roaming/Tencent/xwechat_files`。已配置或用户选择的目录优先用于连接。验证要求目录本身或一级子目录含 `db_storage`；旧 `WeChat Files` 不是 V4 数据根，设置加载中有迁移到同级 `xwechat_files` 的纠正逻辑。

S3 另有枚举 C: 至 Z:、最大深度 3、每盘最多访问 20,000 个目录的辅助实现，会跳过若干系统目录和符号链接。本次已审查的实际连接调用链未发现它被调用，不能把“存在一个辅助文件”写成默认启动时一定扫描所有磁盘。本项目也不会复用这种全盘扫描。

`SOURCE-CONFIRMED`（S7:327）：上游遇到非 ASCII 账号路径时可能在 Public/TraceMemo/path-bridges 创建 junction。该操作并非纯读取，不能照搬到本项目。

本机元数据实测：两处默认候选不存在。用户明确指定 `D:\xwechat_files` 后，只在该目录及账号的 `db_storage` 内检查，发现一个候选账号。未递归扫描磁盘根、用户文档或其他个人位置。

## 2. 如何判断当前微信账号

`SOURCE-CONFIRMED`（S4、S5）：账号目录以 `db_storage` 识别；以规范化账号路径的 SHA-256 形成应用内部账号 ID。目录名可移除四位后缀或提取 `wxid_` 部分作为候选身份，但不应把它当作登录凭证。

`readLocalAccountIdentity` 从 `all_users/config/global_config` 读取有限大小的 profile，跳过四字节前缀，用固定格式参数的 AES-128-CFB 解码，再读取长度编码记录中的用户名、昵称、头像字段。这里的格式常量不是数据库 Key。唯一匹配目录时使用 profile；否则可回退到应用缓存和目录名。

`loginStatus=current` 来自应用当前连接的 accountRoot，并非操作系统证明的“当前微信已登录账号”。低层 `findLatestAccountRoot` 有优先含 session.db、再按目录修改时间排序的回退，但实际 `db:init` 要求明确选中账号，避免把根目录直接当账号。

本项目应分别表示“候选账号”“当前选择”“数据库 Key 验证通过”。多候选不能默认选最新目录。群内 `is_self` 必须依赖已确认的自身账号身份，不能仅凭昵称、路径后缀或一段正文猜测。

本机只确认一个目录候选；没有读取个人 profile 内容或确认登录身份。当前账号身份与自身 sender 映射：`UNVERIFIED`。

## 3. 数据库 Key 如何获取

`SOURCE-CONFIRMED`（S6、S1）：Windows 路径加载 `resources/key/win32/x64/wx_key.dll`，查找 `Weixin.exe` / `WeChat.exe`，辅助检查可见窗口和登录组件，调用 `InitializeHook(pid)` 后轮询 `PollKeyData`，最后 `CleanupHook`。外层入口传入 60 秒超时；等待期间处理进程退出/重开。

返回字符串长 64 时包装层视为获取结果，存储层再验证 64 位十六进制。`key:autoGetDbKey` 随后用所选 accountRoot 试连接，避免取到其他账号的 Key。Key 按账号路径隔离，经 Electron safeStorage 加密保存；这部分依赖 Electron，V1 不照搬。V1 可以先只在进程内存中保持 Key，不增加持久化需求。

原生 Hook 如何查找数据库 Key、支持哪些补丁版本、是否存在附加派生材料：`UNVERIFIED`。不能由 64 个十六进制字符推出具体 SQLCipher 参数。

## 4. Windows API、进程内存、Hook 与 native code

`SOURCE-CONFIRMED`（S6）：使用 tasklist，以及 kernel32/user32 的进程、句柄、窗口枚举等 API。数据库 Key 接口明确带 Hook 初始化/清理步骤，真正实现位于 DLL，不在 TS。

另有 `VirtualQueryEx` / `ReadProcessMemory` 扫描逻辑，但函数名和调用链表明它服务于**图片 AES Key**，不是数据库 Key 的可替代实现；其中存在 `OpenProcess(0x1f0fff, ...)`，也不符合本项目要求的最小读取权限。V1 不需要图片解密，应排除整段逻辑。

PE 静态检查：`wx_key.dll` 导入表出现 `WriteProcessMemory` 和 `ReadProcessMemory`，导出表含 `InitializeHook`、`PollKeyData`、`CleanupHook` 等。导入表只能说明二进制引用这些 API，不能证明哪条执行路径实际使用它们；但在缺少实现源码时，不能认证其“绝不修改微信进程”。本次没有加载或调用 DLL，没有附加调试器、注入、挂 Hook、读取微信进程内存。

如果未来采用其他已验证实现，须限制到当前用户授权的目标进程，使用必要的 QUERY/VM_READ 权限，逐项审查权限和读取范围；不得请求 ALL_ACCESS、写进程、终止微信或关闭安全软件。

## 5. 数据库形式、解密与连接

`SOURCE-CONFIRMED`（S7、S8）：上游走 WCDB 原生兼容 API，不是 Node 普通 SQLite 直接打开明文数据库。调用顺序为加载依赖、`InitProtection(resourcePath)`、`wcdb_init()`、`wcdb_open_account(sessionDbPath,key,&handle)`，随后 JSON ABI 查询。字符串由 `wcdb_free_string` 释放；需要句柄/游标的生命周期管理。没有在可见 TS 里先批量解密到普通 SQLite 的实现。

**未知且不能补猜**：page size、salt、KDF 迭代次数、HMAC、cipher compatibility、Key 是否按库变化、压缩字典、WAL 的解密/一致性处理，以及 `InitProtection` 的副作用。官方 Tencent WCDB 是可用的候选数据库框架，但不能证明上游随包 WCDB.dll 与官方某版本等同，或者普通 rusqlite+SQLCipher 能立即替代。

本机以 `rb` 各读取 session/contact 数据库头部 16 字节，均不是 `SQLite format 3\0`。该结果与加密库相符，但单凭文件头不识别密码算法。没有尝试用普通 SQLite 打开真实文件，没有 checkpoint、VACUUM、重建索引、修改 PRAGMA 或创建 sidecar。

### 严格只读的额外差距

S7 的 `installRecallJournal`、`ensureRecallJournalTable`、`createRecallJournalTrigger` 向消息库传递 `CREATE TABLE` / `CREATE TRIGGER ... BEFORE DELETE ... INSERT`。S1:302-361 显示这属于显式启用的撤回保护，默认配置关闭；不能说普通导出必定执行这些 SQL。可以确认：该封装**整体**不是只读接口。

`wcdb_open_account` 的 ABI 没有 read-only flags，native 实现缺失；仅限制自己发 SELECT 不能排除其内部建库、恢复、WAL checkpoint 或共享内存写入。发布前必须取得可审计后端，证明源库/源 sidecar 禁止写打开；只读失败要报错，不能回退读写。

直接复制运行中的 `.db` 并忽略 `-wal` 不构成可靠快照。`immutable=1` 也不能不加验证地用在仍变化的源库。未来必须落实可验证的只读 WAL 事务或一致快照方案；本次没有创建聊天库副本。

## 6. 涉及哪些 DB 文件

| 文件/模式（相对账号 db_storage） | TraceMemo 证据 | 本机目录实测 / V1 用途 |
| --- | --- | --- |
| `session/session.db`，兼容查找 `session.db` | S7:3098；连接入口 | 存在；会话目录 |
| `contact/contact.db` | TS 通过 kind=contact 查询 `contact` 表，具体路径解析在 DLL | 存在；联系人资料 |
| `message/message_*.db` | S7 使用 native table stats 获得实际 dbPath，而非固定单文件 | `message_0.db`、`message_1.db`、`message_2.db`；普通消息 |
| `message/biz_message.db` / `biz_message_N.db` | S7:1811 的显式枚举及 S13 | 本机 `_0`、`_1`；公众号历史 |
| `message/media_*.db` | 语音数据通过 native API，TS 不暴露具体表 | 本机 `_0`、`_1`；V1 不解码媒体 |
| `message/message_resource.db`、`hardlink/hardlink.db` | native 的媒体定位接口，确切表映射未公开 | 均存在；可选媒体元数据候选 |
| `emoticon/emoticon.db`（另有 emotion 路径兼容） | S7:3104 | 存在；可选表情元数据 |
| `head_image/head_image.db` | 头像接口存在，但物理表在 native 内 | 存在；V1 不下载/显示头像 |
| 各库的 `-wal` / `-shm` | native 如何处理 `UNVERIFIED` | 上述核心库均有 sidecar，不能遗漏 WAL |

其他实际存在的库包括 contact_fts、message_fts、favorite、favorite_fts、general、bizchat、chatbot_message、weclaw、sns、solitaire、third_app_icon。它们的存在不意味着应把朋友圈、收藏、机器人等纳入 V1；未读取这些库的内容。不要将 `message_fts.db` 误识别为普通消息 shard。

## 7. 消息、联系人、群聊的 DB / table 来源

| 对象 | 能从可见源码确认的部分 | 未确认部分 |
| --- | --- | --- |
| 会话 | `session.db` -> `wcdb_get_sessions` -> normalizeSession；结果包含 username 等 | native 使用的真实 session 表名、过滤规则 |
| 联系人 | kind=contact；SQL 直接读取 `contact`，字段 `username`、`nick_name`、`remark`，另有身份/头像字段兼容 | 本机表结构和列版本 |
| 群聊 | 会话 username 以 `@chatroom` 结尾；消息仍走消息 shard；群昵称由独立 native API 返回 | 群成员关系表名、编码和数据库内连接关系 |
| 群成员 | `wcdb_get_group_members` + `wcdb_get_group_nicknames`，联系人表补微信昵称/备注 | 不能把接口 JSON 字段当作实际表字段 |
| 消息 | native stats 返回 `table_name`、`db_path`；公众号回退显式构造 `Msg_` + MD5(username) 并查询 sqlite_master | 普通 shard 的完整表映射和 sender 数值 ID 解析由 native 隐藏 |

S7 的 `getChatTables()` 返回 `Chat_<md5>`，这是适配旧业务的**逻辑名称**；不能据此断言 V4 物理消息表名为 `Chat_*`。同样，不能把别的项目的 `Name2Id`、`SessionTable` 等名称未经核对写成本机已确认 schema。

可见消息字段候选：`local_id`、`server_id`、`create_time`、`local_type`、`message_content`、`compress_content`、`sender_username`、`computed_is_send`、`is_send`，并兼容其他别名与 `WCDB_CT_*`。部分是 native 计算结果，不能直接假定每个磁盘表拥有这些列。

发送者优先使用 native 返回 username，再回退到正文的群发送者前缀；自身身份与 send 标志决定 mesDes。上游 mesDes 的最终输出语义是 **0=自己，1=对方**，不是把 is_send 原值直接序列化。群昵称、微信昵称和通讯录备注需要分开缓存。对身份有歧义的消息应保留原值并标记问题，不能静默标错 `is_self`。

## 8. shard 与跨库完整性

`SOURCE-CONFIRMED`（S7:1177、1668、1771；S13）：有多消息库，同一会话可能分布于多个 shard，local_id 可重复。上游有原生 cursor 漏旧 shard 的兼容路径：用 table stats 找库，再逐表按时间查询，与 cursor 结果合并。

必须区分“有分页接口”和“完整 streaming”：现有 cursor 每批 1000 行，但 TS 把批次追加到 `allRows`；表扫描无 limit 时每个 store 使用 `LIMIT 5000`，且可容忍部分 shard 失败。因此不能照搬这个路径后宣称百万消息不漏、不截断、不增加内存。

V1 方案：每个账号一个 client，统一枚举所有实际消息 stores；选定会话只映射到对应 stores；`COUNT` 与输出使用同一组参数和已确定的源视图；分页按稳定键 `(create_time, local_id, rowid)`，分片标识参与唯一性，跨 shard/会话用有界归并获得总排序。server_id 作为字符串保留，避免 JavaScript 精度问题。不能按单个 local_id 全局去重，也不能用随消息数增长的 HashSet 宣称恒定内存。若库中同一 server_id 出现多个物理副本，需在一致快照上定义并验证去重；COUNT 必须计相同逻辑消息。

## 9. 各消息类型如何表示

`SOURCE-CONFIRMED`（S7 的 decodeMessageContent、S9、S10）：类型可能是宽整数，服务层用低 32 位做主类型；原值必须完整保留。内容可为文本、BLOB、hex/base64 包装；Zstd 魔数 `0xfd2fb528` 被 fzstd 解压。native 自身是否已处理 WCDB 字典压缩仍未知。

| canonical type | 上游识别依据 | V1 应保留的信息 |
| --- | --- | --- |
| text | 主类型 1 | UTF-8 正文；中文、emoji、换行、前后空白均保真 |
| image | 3，`img` XML；也有 packed_info 补充定位 | 已有 MD5、datName、尺寸/metadata；不下载、不解密 |
| voice | 34；正文空也有效，媒体可能在别处 | 语音标识、已有时长/metadata；不因正文空丢弃 |
| video | 43，`videomsg` XML | md5/newmd5/rawmd5、长度、时长、尺寸 |
| file | 主类型 49，appmsg 子类型 6 / 74 | 标题、已有文件名/路径/hash/大小；发送中需保留状态 |
| quote | 49 + 子类型 57 或 refermsg | 自身标题/正文、引用内容、引用发送者/类型/已有 ID；缺失字段置空 |
| system | 10000 / 10002；revokemsg、sysmsgtemplate 等 | 可读文本及原 XML；只解释撤回通知，不安装撤回保护 |
| location | 48，location XML | 坐标、地点名、label；不调用地图服务 |
| link | 49 的分享/链接子类型，例如 5 | 标题、描述、URL 元数据；不请求 URL |
| sticker | 47，emoji/sticker XML，或 49 的相应载荷 | MD5、已有链接/metadata；不联网获取内容 |
| unknown | 未覆盖类型、格式错误或无法可靠解释 | 完整 raw_type、raw_xml 或原二进制的可逆编码、解析状态 |

上游还识别名片 42、通话 50、小程序、红包、转账、合并转发。V1 不必新增平台功能，但必须将暂不支持的结构作为 unknown 保留，而不是丢弃或编造文本。

develop 的系统消息修复很关键：新版正文在 template，占位符引用 link_list；hidden=1 的按钮文案不应冒充整条消息。解析应处理 CDATA、XML 实体和嵌套 refermsg；Rust 实现选择不会加载外部实体/访问网络的 XML 解析器，并保留失败原文。上游 regex/trim/不可读字符清理属于展示逻辑，不能无条件迁到保真导出。

媒体路径只记录已验证来源字段或简单本地关联，缺失就置空。不要将 MD5 强行拼成“原始路径”；不复制媒体、转码、OCR、语音识别。链接保留为数据，不跟随访问。

## 10. 哪些源码可以直接复用

**目前没有确认可以直接复制进产品的 TraceMemo 核心文件。** 技术上可能独立抽取目录发现、账号匹配、消息解析、FFI 签名，但缺少覆盖授权，不能先复制再补 notice。DLL 下载可用不等于允许再分发。

如果取得所需授权，最小候选闭包是 S2/S4/S5 的目录/身份规则、S6 数据库 Key 专用路径、S7 的只读查询子集、S10 的解析子集及有明确来源的 native 后端。resource-paths 只保留固定打包目录；不引入 Electron、Koffi、服务器、监控、AI、发送、缓存平台。还必须先解决 Hook、read-only 和 native 内部副作用，授权本身不能替代技术审计。

WeFlow 兼容表现在旧 `Chat_*` 逻辑接口、WechatDb 的 mesLocalID/mesDes/msgContent 等字段以及 native 函数族。README 声称行为兼容不能推出“当前 DLL 就是某个历史 WeFlow 二进制”，本次没有可靠出处链证明这种同一性。

## 11. 哪些部分适合 Rust 实现

在第 13 节的授权/隔离边界成立后，可独立实现：有限目录发现、账号选择与校验、规范模型、SQL 参数化过滤、联系人缓存、消息归一化、JSONL/JSON streaming、取消与 worker 消息通道、egui 窗口。

**不适合凭当前资料直接重写的部分**：数据库 Key 的真实算法、WCDB 的 cipher 参数和 shard 发现内部实现。用空函数、假数据或“用户自行提供解密库”替代它们，都不满足自动读取本机微信的 V1 验收。

### Phase 0 架构决策简报（当时的设计；最终实现见文末）

新项目，无既有产品架构。产品只用 Rust stable；GUI 用 egui/eframe，core 不依赖 GUI。只建立实际需要的模块：`core/wechat`、`core/parser`、`core/model`、`core/export`、`ui`，不提前建立插件框架、trait 单实现层或多进程服务器。

| 组件 | 唯一职责 / 边界 | 验证 |
| --- | --- | --- |
| WechatClient | 持有一个账号的已验证只读后端；detect/open/list/count/有界读取 | 首先用本机真实库验证，不打印正文/Key |
| parser/model | 内部 row -> canonical Message；身份与时间规则唯一实现处 | 人造样本、边界数据及受控本机对照 |
| exporter | Filter + message stream -> JSONL 或 JSON array；文件事务与取消 | 每行独立解码、失败/取消、计数相等 |
| ui | 只生成 ExportConfig、发送 worker 请求和展示结果 | 导出期间响应、取消、状态与进度 |

若未来使用获许可 C ABI，只允许一个 Rust adapter 负责：加载固定路径 DLL、初始化一次、native handle/cursor RAII、UTF-8 与路径契约、i64 时间/ID 类型核对、成对释放 native 字符串、错误码脱敏映射。当前 ABI 游标时间为 int32，是需明确测试的范围限制；不可悄悄窄化 Rust i64。该 FFI 是必要的数据库边界，不增加 Node/Python runtime。当前未选择或链接任何原生依赖。

顶层构建入口计划为 Cargo；固定 Cargo.lock、Windows x64 target、离线运行测试；依赖确认后先完成 Phase 1，再逐阶段加入 serde/serde_json、错误类型和 eframe。数据库依赖须由实证决定，不能提前把 rusqlite 的普通 SQLite 当作兼容后端。依赖安装需另按用户的环境规则执行；本次未安装任何包。

### canonical 与导出契约

- 保留用户要求的 id、conversation_id/name、sender_id/name、is_self、timestamp、datetime、type、content、raw_type、media、quoted。增加可逆 raw_payload/raw_xml 和解析状态；未知值不能伪造。
- timestamp 使用 Unix 秒 i64；datetime 用明确偏移的 RFC3339；ID 用字符串。每次导出冻结所用本地时区和查询参数。
- 日期采用本地日历：`start 00:00 <= t < (end + 1 calendar day) 00:00`。2026-01-01 至 2026-09-19 的右边界为 2026-09-20 00:00。按日历构造次日午夜，不把所有日期当作固定 86400 秒；遇到时区歧义/无效午夜必须明确处理。
- 近 7/30/90 天默认包含今天在内的 N 个自然日；今年从当地 1 月 1 日至今天。自定义开始晚于结束报输入错误；合法但没有匹配记录则成功输出空结果。
- 会话过滤先映射到实际 stores，时间条件与 COUNT 都下推 SQL，使用参数绑定。表名不能绑定时，仅接纳已枚举且验证的标识符并正确引用。
- 流水线为只读 row/batch -> normalize -> serialize -> BufWriter；不创建 Vec<Message> 全量快照。JSON 用流式数组写入，不用全量 JSON Value。
- 全局排序由时间加明确的稳定 tie-breaker 决定；COUNT 与去重规则保持一致。预览后源库若变化，必须使用同一有效快照或重新计算并提示，不能声称精确计数仍然一致。
- 一个 worker 拥有数据库连接、prepared statements 和联系人缓存；GUI 通过有界 channel 传 ExportConfig 和节流进度，取消用 AtomicBool。若 backend 单次调用不可取消，必须给出有界批次/超时，不能在 GUI join 阻塞。
- 输出位于用户选定的独立本地目录，拒绝写入微信数据树及网络路径。先用 create_new 写唯一 `.partial`，flush/sync 成功后发布；取消/磁盘满不覆盖已有文件，半成品不能冒充成功。Windows 同名发布必须有不覆盖保证。
- 普通日志只记录错误分类、脱敏阶段和数量，不记录 Key、正文或 native 返回的原始敏感错误串；不新增网络、遥测、自动更新和媒体下载。

## 12. TraceMemo 与相关依赖许可证

`SOURCE-CONFIRMED`：develop 完整树仅找到 wechat_chatter 的局部 LICENSE/NOTICE；main 另有已移除连接器的 MIT LICENSE。README 和 package.json 未给出主体 license 声明；关键 TS 文件无覆盖授权头。wechat_chatter NOTICE 明确说明其许可证不适用于仓库其他部分。未找到 wx_key.dll / wcdb_api.dll 对应源码及许可证。

因此复用决策是“未取得许可，不复制或分发”，而不是擅自指定 MIT/GPL。GitHub 官方说明区分公开可见/可 fork 与获准修改再分发：[Licensing a repository](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository)。这是工程准入判断；本调查不替代具体授权文本。

Tencent [WCDB LICENSE](https://github.com/Tencent/wcdb/blob/master/LICENSE) 确认主体 BSD-3-Clause，并列第三方许可。它不能替 TraceMemo 专用封装背书；若采用官方源码构建，要按固定版本收集实际依赖许可。Koffi/fs-extra/fzstd/SDL2/MSVC runtime 的随包具体版本和完整分发条款尚未逐一锁定；它们没有作为本项目依赖引入。发布前需要对实际依赖闭包重新审计，而不是继承上游整个 package 清单。

详见仓库根目录 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)。

### 已追踪的替代来源

- [WeFlow 当前官方主分支](https://github.com/hicccc77/WeFlow/tree/318d5ac4a968eaef9a5cbd7c663151b6ba2efcf0)：当前页面只有说明和图片，README 表明移除了提 Key/解密能力；没有可接续当前实现的源码闭包。不能据此判断历史版本从未有过许可，也没有把非官方镜像当作已核实上游。
- README 引用的 [sjzar/chatlog](https://github.com/sjzar/chatlog)：当前官方仓库只保留代码移除说明，不提供本次所需源码。
- [WechatMessageExplorer](https://github.com/svcvit/WechatMessageExplorer)：所引用项目是 Mac 版，不能当作 Windows 4.1.13.65 已验证后端。
- 有限补充检索发现 [WeChatDataAnalysis/key_v4.py](https://github.com/LifeArchiveProject/WeChatDataAnalysis/blob/main/key_v4.py) 和 [privatetalk](https://github.com/crisxuan/privatetalk)。前者呈现另一条内存扫描/可选附加 Key 处理路线，但本次未建立可复用授权与当前版本验证；后者 README 声明 CC BY-NC-SA 4.0，原生移植说明仍引用外部 native 源码目录。二者均未通过本项目复用准入；未运行其提 Key、修改或发送功能。此为候选筛查，不声称已经穷尽所有社区实现。

## 13. 不适合复制时的 clean-room 方案

不能把读过源码后的逐行 Rust 翻译称为 clean-room，也不能以“独立兼容层”名称绕过授权要求。可执行的分离方案如下，当前只是方案：

1. 先获得可审计且授权清楚的 native 实现，或固定一个允许复用的替代上游版本；逐文件验证来源。若直接使用获许可实现，按其许可证保留 copyright、license、修改记录，而无需虚称 clean-room。
2. 若确需 clean-room，由分析角色仅输出必要的接口行为、文件格式事实、错误和自制测试向量；不交付上游源码、反编译实现、原测试代码或表达性注释。密钥/密码参数必须有独立可靠证据，不能用猜测填表。
3. 由未接触受限制实现的独立实现者从这些规格编写 Rust；保留来源接触记录。本次分析者已经阅读 TraceMemo 源码，因此不能自行宣称满足严格的人员隔离。
4. 用自己生成的空库/样本和用户授权本机数据进行兼容测试，先验证读 Key/页校验/只读打开/WAL/联系人/文本，再进行大量消息与 GUI。真实正文与 Key 不进入报告、fixture、提交或普通日志。
5. 在真实读取成功之前不推进产品 GUI 和后续功能验收。若找不到满足要求的后端，保留 Phase 1 阻塞；不靠黑盒 DLL 或手工解密输入假装达到自动读取标准。

需要解除的具体门槛：`wx_key.dll` 与 `wcdb_api.dll` 的来源、对应源码/构建版本、复制修改及再分发授权；对实际执行路径的不写进程证明；数据库只读打开和 WAL 一致性证明。另一条可接受路线是已经通过同等审核的社区实现。本机目录路径已由用户提供，不再是阻塞项。

## Phase 0 结束时的记录（历史，后续完成状态见文末）

已执行：Rust/Cargo 可解析且版本均为 1.98.0；通过定向 Get-Process 查到 Weixin 4.1.13.65；在用户指定 D:\xwechat_files 发现一个账号候选、三个普通消息 shard、两个公众号 shard；仅按只读文件句柄检查两个数据库头。未扫描用户配置/凭据，未读聊天正文、Key 或进程内存，未写原库。运行中的微信仍可能自行修改其数据库，因此不能凭这次检测宣称文件哈希保持不变。

上游 getting-started 推荐的 Windows 版本为 4.1.9.57；本机 4.1.13.65 的数据读取兼容性是 `UNVERIFIED`，不能把 README 的“Windows 4.x”推广成所有补丁版实测通过。

| 阶段 / 验收 | 状态 | 证据 / 未完成原因 |
| --- | --- | --- |
| Phase 0 源码、native 边界、许可调查 | 完成 | 本文、固定提交、静态 PE 检查；缺失内容均明确列出 |
| Phase 1 detect 的环境预检 | 已做有限实测 | 进程/版本/目录/文件头；不是 Rust Core 完成 |
| 自动数据库 Key、只读连接、真实联系人/文本 | BLOCKED / UNVERIFIED | native 来源、许可及只读行为未满足要求 |
| Phase 2–4 模型、流式导出、过滤和 COUNT | NOT_RUN | 遵守先打通核心链路的开发顺序 |
| Phase 5–6 GUI 与复杂类型 | NOT_RUN | 未创建 GUI 壳 |
| Phase 7 测试 | NOT_RUN | 仅阅读上游 mock 测试，没有运行或声称本机通过 |
| Phase 8 release / EXE | NOT_RUN | 没有产物，不宣称 V1 完成 |

后续最小验收矩阵（均待执行）：

| 检查组 | 必须覆盖 |
| --- | --- |
| 范围与 COUNT | 全部、单会话、多会话、全选/清空、无消息会话；COUNT 与实际输出逐项相等 |
| 日期 | 近 7/30/90 天、今年、自定义、空结果、反向区间；开始午夜包含、结束日最后一秒包含、次日午夜排除 |
| 数据保真 | 中文、emoji、空白、换行；群成员、自己/他人、相同秒排序、重复 local_id 跨 shard、不漏旧库 |
| 消息解析 | 文本及全部列出的媒体类型；引用、CDATA、XML 实体、模板系统消息、损坏 XML、未知类型、空正文语音；原始载荷可恢复 |
| 性能 | 10 万/100 万生成数据；记录峰值工作集、吞吐、取消延迟，验证内存不随消息总量增长；不把生成数据冒充本机实测 |
| 文件可靠性 | JSONL 每行独立解码、流式 JSON 数组整体有效；输出无权限/磁盘满/取消/同名文件/断开时不损坏已有输出 |
| 只读与本地 | 源库和 WAL/SHM 禁止写入测试、只读 ACL 场景、进程权限审核、断网运行、无日志正文或 Key |
| GUI 与发行 | Windows x64 实际启动；480px 单窗口、日期选择、会话筛选、worker 进度与取消；无 Node/Python/WebView 依赖 |

## 原生二进制溯源

仅静态读取以下 develop 文件，未加载执行；它们只在忽略的研究目录中。没有据此授予再分发许可。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `resources/key/win32/x64/wx_key.dll` | 195072 | `9a2f2c6a09a3219afa4748a5e7e513d99c607173c3112c5afcc888996b98248c` |
| `resources/wcdb/win32/x64/wcdb_api.dll` | 382976 | `b8417ccbc9b306b53957b2c87d9600c6bacb5d51344d9996867056357f08fa40` |
| `resources/wcdb/win32/x64/WCDB.dll` | 9992704 | `1d5c7e92add00d986b98823340b4d5dd06dfb3a8c400dcae3caab4e447c9e3dc` |

`wcdb_api.dll` 的 PE 导入确认依赖 WCDB.dll 与 MSVC runtime；其导出确认有 open_account、exec_query、message_cursor 等函数。WCDB.dll 有 cipher/Zstd 符号以及 WS2_32 导入；这不是“调用过网络”的证据，只说明不能仅凭 TypeScript 未联网就认证整个黑盒依赖完全无网络副作用。

复查命令（在项目根执行；工具不执行被分析 DLL）：

```powershell
git -C .reference/TraceMemo rev-parse HEAD
git -C .reference/TraceMemo ls-tree -r --name-only HEAD
rg -n 'InitializeHook|PollKeyData|CleanupHook' .reference/TraceMemo/src/main/key-service-win.ts
rg -n 'CREATE TABLE|CREATE TRIGGER|LIMIT|allRows.push' .reference/TraceMemo/src/main/wcdb4-client.ts
& 'D:\mingw64\mingw64\bin\objdump.exe' -p .reference/TraceMemo/resources/key/win32/x64/wx_key.dll
Get-FileHash -Algorithm SHA256 .reference/TraceMemo/resources/key/win32/x64/wx_key.dll
```

开发期间联网仅用于用户要求的公开源码调查；产品的零联网要求仍保持不变。没有发送任何本机聊天、账号 profile 或数据库数据到外部服务。

## Phase 0 后续增补：可审计的备用实现

在完成上述分析后，继续按用户的第四优先级调查替代项目，找到 [wechatauto-replica](https://github.com/fanyuantaier/wechatauto-replica/tree/492a8fb70b95865613d6d8d9740323233dbfa197)，固定提交 `492a8fb70b95865613d6d8d9740323233dbfa197`（2026-09-19，1.2.2.5）。本地 LICENSE 为完整 Apache-2.0；pyproject.toml 同样声明 Apache-2.0。没有 NOTICE 文件。

`SOURCE-CONFIRMED`：其 `wechatauto/db.py` 的 `_collect_key_candidates` 用 QUERY_INFORMATION | VM_READ 打开目标进程，定位 WCDB Config.Cipher 对象，解码候选 raw encryption key，并通过每库首页 HMAC 认证。不需 Hook、写内存、注销或重启微信。只移植这些函数及相关页解密规则，不采用 cfg master-key fallback、UI 自动化、发送、媒体下载、日志正文及持久化 Key。

这条源码提供 4096 字节页、80 字节保留区、AES-CBC、HMAC-SHA512 和 raw-key 的具体规则；这些是该替代源码的证据，**不是从 TraceMemo 猜出的参数**。先用本机页认证及真实查询验证，成功后才能标记兼容。

架构决策更新：产品核心可以全部为 Rust，使用已安装的 RustCrypto 原语和 Windows bindings。源 DB/WAL 只用只读文件句柄；工作区内制作经一致性检查的临时副本，逐页认证/解密后以只读 SQLite 查询。WAL 必须按 SQLite 官方格式检查 salt、连续 checksum 和最后提交帧；不会移植上游按页类型猜测、忽略失败、容忍缺最近消息的回退。实现来源、修改范围与 Apache 许可证随项目记录。该路线属于有许可的移植，不称为 clean-room。

## 实现后的真实验证与差异修正

`RUNTIME-VERIFIED`：本机微信 4.1.13.65、用户提供的 D:\xwechat_files。Config.Cipher 只读扫描候选通过每库首页 HMAC；8 个必要库逐页认证、WAL 合并及 SQLite quick_check 通过。646 个会话、498 个非空会话，最后一次完整读取 392,468 条消息；COUNT、输出数量与 canonical ID 去重检查一致。单群 1,647 条 JSONL/JSON 均成功解码，测试输出删除。

实际 schema 补充：`session.db.SessionTable`；`contact.db.contact/stranger/chat_room/chatroom_member`；各 `message_N.db` / `biz_message_N.db` 的 `Msg_<MD5(username)>`；同库 `Name2Id(rowid,user_name)`；资源库的 `ChatName2Id` 和 `SenderName2Id`。消息列含 `local_id/server_id/local_type/sort_seq/real_sender_id/create_time/message_content/compress_content/source/packed_info_data`。

关键修正：备用 Python 源码把 `real_sender_id` 映射到资源库 `SenderName2Id`，该行为不能忠实照搬到本机。实际私聊对照确认编号属于各消息库自己的 `Name2Id`；已在 Rust 中逐分片缓存映射，并用不同分片编号的回归测试防止串号。自己身份从所选目录与真实用户名唯一匹配确定，不假设编号 2 是自己。资源库用户名仅辅助账号和会话识别。

最终 core/UI 分离；流式 JSONL 和 JSON、SQL 范围/COUNT、跨分片去重、后台 worker/取消、日期选择器和多会话选择已实现。EXE 静态链接 CRT，原生图标和版本资源已嵌入，未引入 TraceMemo DLL、Node、Python 或 WebView。

合成基准为 100,002 / 1,000,002 条，核心进程峰值工作集分别 14.79 / 18.30 MiB；百万条导出约 3.939 秒。不是实际聊天性能承诺。测试、未支持类型、未测系统和 GUI 固定开销详见 [validation.md](validation.md)。

Phase 1–8 的产品实现及本机范围验证已完成；TraceMemo 黑盒 DLL 的原始许可/行为缺口并未因此被宣称解决。产品选择有许可的可审计替代路线，同时保留未知载荷和明确的版本边界。
