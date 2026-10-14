# 项目来源与改动

## 从 Nongfsq 项目开始

WordSide 使用 **[Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api)** 作为 Cambridge 查询与解析的基础。获取来源为公开 `main` 分支，记录日期为 **2026-10-13**。

Nongfsq 仓库继承自 **[chenelias/cambridge-dictionary-api](https://github.com/chenelias/cambridge-dictionary-api)**，原作者署名 **EliasChen**。原 MIT 许可证完整保留，详见[原许可证](../modules/cambridge-dictionary-api/LICENSE.txt)。感谢原项目提供的网页解析实现与字段结构。

## 沿用与新增

| 部分 | 来源与处理 |
|---|---|
| 词性、音标、音频、释义、译文、例句 | 沿用 Nongfsq 原解析器的选择器与内容顺序 |
| 数据格式 | 保持 JSON 字段与顺序兼容，`verbs: []` 作为兼容字段保留 |
| 原解析器对照 | 原文件保存在 `tests/upstream/data.js`，用于离线对照测试 |
| 桌面查询 | 将解析行为移植到 Rust，由桌面程序直接请求 Cambridge 网页 |
| 查询可靠性 | 增加连接管理、缓存、取消、超时与结果校验 |
| 拼写建议 | 缺词后读取 Cambridge 提供的候选，由用户点击确认 |
| Windows 取词 | 增加选区读取、原生文本控件适配与临时兼容记录 |
| 产品界面 | 增加释义浮窗、固定与缩放、托盘、设置及配色 |

JS 改版与原文件在仓库中保留，作为来源记录和行为对照。详细模块变更见 [UPSTREAM.md](../modules/cambridge-dictionary-api/UPSTREAM.md)。

## 许可与词典内容

原 MIT 文本、EliasChen 署名和来源链接随相关代码保留，其他依赖遵循各自许可。WordSide 自有代码的许可状态见 [NOTICE](../NOTICE.md)，原生依赖说明见 [third-party](../third-party/README.md)。

Cambridge 词典内容与音频由其权利人持有，数据使用授权与源码许可分别适用。WordSide 是独立项目，与 Cambridge 无官方关联。
