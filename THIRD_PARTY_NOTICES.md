# 第三方来源与复用状态

更新日期：2026-09-20。本项目移植下列获许可的最小数据库读取算法，其余参考项目不作为产品依赖。锁定 Rust 依赖的声明和完整许可证见 `licenses/dependencies.txt`；包内缺失的 monorepo 许可证从 crate 对应提交补全，来源索引为 `licenses/upstream/sources.json`。默认内嵌字体的 OFL/UFL/MIT 条款也包含在汇编中。运行时加载的 Windows 字体不随包再分发。

## wechatauto-replica 的最小 Rust 移植

- 上游：https://github.com/fanyuantaier/wechatauto-replica
- 提交：`492a8fb70b95865613d6d8d9740323233dbfa197`，版本 1.2.2.5。
- 作者：上游 pyproject.toml 标注 `wechatauto-replica`；所用源码没有单独版权行，不臆造版权持有人。
- 许可证：Apache License 2.0，完整原文保存于 `licenses/wechatauto-Apache-2.0.txt`；未发现上游 NOTICE 文件。
- 源文件：`wechatauto/db.py`；范围：Config.Cipher 常量、`_collect_key_candidates`、`_find_bytes`、`_split_key`、`_verify_enc_key`、`_decrypt_page` 的读取/认证/页布局逻辑。
- 目标：`src/core/wechat/key.rs`、`src/core/wechat/cipher.rs`。
- 修改日期：2026-09-19；改为 Rust、限定进程权限、分块扫描与取消、Key 只留内存并清除、逐页认证；WAL 校验与提交处理另按 SQLite 公开格式实现。不移植发送、UIA、cfg master-key、Key 落盘或其他上层功能。

## 其他调查来源

| 来源 | 已核对范围 | 许可证结论 | 当前处理 |
| --- | --- | --- | --- |
| [TraceMemo develop](https://github.com/Wxw-Gu/TraceMemo/tree/a9f264107b2d6c57aca4f1488c397fc26cb07e62) | README、package.json、相关 TS、完整文件树、Windows 原生 DLL | 未找到覆盖主体代码、`wx_key.dll`、`wcdb_api.dll` 的明确许可证；不能从 public 状态推断可复制或分发 | 仅在忽略目录 `.reference/TraceMemo` 中研究原样文件；未作为产品复用 |
| [TraceMemo main](https://github.com/Wxw-Gu/TraceMemo/tree/276fb72bc9bb8b4757ba269e9da3fc01b09e81f7) | 文件树、关键文件差异、局部许可证 | `services/wechat-connector/LICENSE` 为 MIT，Copyright (c) 2026 fastclaw-ai；仅适用于该组件，不覆盖微信数据库访问层 | 不引入该连接器 |
| [wechat_chatter notice](https://github.com/Wxw-Gu/TraceMemo/blob/a9f264107b2d6c57aca4f1488c397fc26cb07e62/docs/third-party/wechat-chatter/NOTICE.md) | NOTICE 和相邻 LICENSE | GPL-3.0；Copyright (C) 2026 yincongcyincong；说明明确不用于声明 TraceMemo 其他部分许可证 | 属于发送功能，完全排除 |
| [WeFlow](https://github.com/hicccc77/WeFlow/tree/318d5ac4a968eaef9a5cbd7c663151b6ba2efcf0) | 当前官方主分支、README | 当前分支不提供所需底层源码；不能推断历史版本许可证或把历史授权套给其他项目 | 未复制历史镜像或二进制 |
| [Tencent WCDB](https://github.com/Tencent/wcdb/blob/master/LICENSE) | 官方 LICENSE | WCDB 主体 BSD-3-Clause，另含第三方条款；该授权不自动覆盖 TraceMemo 包装 DLL 及未核对来源的构建产物 | 仅候选，尚未下载源码构建或链接 |
| [privatetalk](https://github.com/crisxuan/privatetalk/blob/main/LICENSE) | 项目 README、原生组件移植说明 | README 声明 CC BY-NC-SA 4.0；尚未完成原生组件逐文件授权核对 | 不作为已获许可的替代组件 |

未来任何实际复用必须在此增加：精确提交和源文件、原版权声明、完整许可证文件位置、目标文件、复制/修改/翻译范围，以及随二进制发布的 notice。Rust 翻译并不自动消除原授权义务。未核对的 DLL 不进入发布目录。

研究快照中的原文件保留上游内容，没有改写版权或许可声明。研究报告引用的是来源和行为说明；发行包另附 `licenses/` 中的实际依赖许可证。
