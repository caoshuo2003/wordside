# 项目来源与改动

## 从 Nongfsq 项目开始

WordSide 使用 **[Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api)** 作为 Cambridge 查询与解析的基础。来源是其公开 `main` 分支，获取日期为 **2026-10-13**。本仓库不将获取日期冒充已锁定的上游提交编号。

Nongfsq 仓库继承自 **[chenelias/cambridge-dictionary-api](https://github.com/chenelias/cambridge-dictionary-api)**，原作者署名 **EliasChen**。原 MIT 许可证完整保留，详见[原许可证](../modules/cambridge-dictionary-api/LICENSE.txt)。

这层关系同时写入首页、模块记录与分发许可。感谢原项目提供简洁的 Cambridge 网页选择器、释义/例句顺序与 JSON 格式。

## 哪些内容沿用，哪些由 WordSide 增加

| 部分 | 来源与处理 |
|---|---|
| 词性、音标、音频、释义、译文、例句选择器 | 从 Nongfsq 原解析器沿用并移入本机 JS 模块 |
| 成功 JSON 字段与顺序 | 保持兼容；`verbs: []` 仅保留字段，无词形功能 |
| 原始解析器对照 | 原文件保存在 `tests/upstream/data.js`，用于离线测试，不进入桌面运行路径 |
| 桌面解析器 | 将 JS 解析行为移植到 Rust；合成夹具对照完整字段 |
| 查询可靠性 | 连接复用、并发合并、限额、取消、超时、完整性校验与错误分类 |
| 缺词建议 | 确认缺词后读取 Cambridge 原生候选，用户点击确认，不自动纠错 |
| Windows 取词 | UIA、原生文本控件、源窗口检查、隔离读取与有限兼容记录 |
| 产品界面 | Tauri / TypeScript 浮窗、固定/缩放、托盘、设置、配色与用户提示 |

WordSide 直接访问 Cambridge 网页，不使用原项目已部署的 API 服务。JS 模块作为可独立运行的来源改版和行为对照保留，桌面运行不携带 Node。

## 已移除的原功能

原项目的 Wiktionary 词形请求不在 WordSide 当前产品中执行。接口保留空 `verbs` 字段是兼容措施，不代表预留或已实现该功能；原文件中相关代码仅作为来源原文保留。

## 许可与内容边界

- 原项目的 MIT 文本、EliasChen 署名和来源链接保持不变。
- 原生依赖按各自许可分发，补充声明见 [third-party](../third-party/README.md)。
- WordSide 自有代码尚未选择对外开源许可，见 [NOTICE](../NOTICE.md)。
- Cambridge 词典内容与音频不因 MIT 源码许可而变成自由再分发的数据；项目无官方关联。

更细的模块调整见 [UPSTREAM.md](../modules/cambridge-dictionary-api/UPSTREAM.md)。
