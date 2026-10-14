# 许可与来源

## WordSide 自有代码

WordSide · 词伴的源码已公开，自有代码尚未选择开源许可证，也尚未提供通用的复制、修改和再分发授权。下述第三方代码继续遵循各自许可。

## 参考与移植

Cambridge 解析部分基于 [Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api) 修改，该项目继承自 [chenelias/cambridge-dictionary-api](https://github.com/chenelias/cambridge-dictionary-api)。原 MIT 许可证中的 `Copyright (c) EliasChen` 保留于 [LICENSE.txt](modules/cambridge-dictionary-api/LICENSE.txt)。

仓库保留 JS 改版与原解析器对照，桌面将相关解析行为移植到 Rust。原 MIT 许可与作者署名随相关代码保留；新增实现与来源关系见[项目来源](docs/ATTRIBUTION.md)及 [UPSTREAM.md](modules/cambridge-dictionary-api/UPSTREAM.md)。

## 第三方依赖

发布页与软件分发包提供同版第三方许可，包含 Nongfsq / EliasChen 的 MIT 声明。原生依赖的许可与补充来源见 [third-party](third-party/README.md)。

## 词典内容

释义、例句和音频来自 [Cambridge Dictionary](https://dictionary.cambridge.org/)，相关内容权利属于其权利人。WordSide 是独立项目，与 Cambridge 无官方关联。词典内容的使用授权与解析器源码的 MIT 许可分别适用，相关说明可参阅 [Cambridge 数据授权页面](https://dictionary.cambridge.org/license)。
