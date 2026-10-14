# 上游来源与改版范围

来源：[Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api) 公开 main 分支，获取日期 **2026-10-13**。该仓库继承自 [chenelias/cambridge-dictionary-api](https://github.com/chenelias/cambridge-dictionary-api)。保留原 MIT 许可证与 **EliasChen** 署名；未记录的上游提交编号不作补造。

## 保留与移植

原 `data.js` 的词性、音标/音频、释义/译文、例句选择器移入 `dictionary/cambridge.js`，保持原文本提取和数组顺序。原路由、四种词库与成功 JSON 字段兼容，原依赖和 pnpm 锁定版本保留。

原解析器全文保存在 `tests/upstream/data.js`，只用于合成页面的离线对照，不用于产品请求。Rust 解析器位于 `desktop/src-tauri/src/dictionary/`，移植相同字段和行为；跨语言夹具来自原始 oracle，而非真实网页归档。

## 本机改版

- 查询默认入口 `lookup.js`，导入时不启动服务；模块与 CLI 可直接调用，本机 HTTP 路由为可选独立入口。
- 共享 keep-alive、完整 URL 缓存键、容量上限、并发合并；只缓存完整成功结果。
- HTML 截断、结构异常与网络错误分别归类，不把连接失败当作缺词。
- 明确缺词后提取 Cambridge 原生 `spellcheck` 候选，严格限定词库、原查询与候选链接；不自动纠错，不增加正常命中的候选请求。
- 合法省略 def-body 的英英释义保留；不削弱全文、定义数量与译文/例句归属检查。
- Wiktionary 请求、词形模块和 provider 已移除，`verbs` 固定为空数组，仅兼容原 JSON。

当前模块版本 **1.2.1-local.1**，独立于 WordSide 应用版本 **0.1.0**。桌面直接使用 Rust，不打包该 JS 服务或 Node 运行时。`index.html` 保留原模块预览页面，不是 WordSide 桌面入口。

产品级归属与许可见 [ATTRIBUTION](../../docs/ATTRIBUTION.md) 和 [NOTICE](../../NOTICE.md)。
