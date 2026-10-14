# 许可与来源

## WordSide 自有代码

WordSide 当前作为私有项目维护，自有代码尚未选择对外开源许可。仓库可访问性不自动授予使用、再分发或开源授权；未来公开前将单独确定许可。

## 参考与移植

Cambridge 解析部分基于 [Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api) 修改，该项目继承自 [chenelias/cambridge-dictionary-api](https://github.com/chenelias/cambridge-dictionary-api)。原 MIT 许可证中的 `Copyright (c) EliasChen` 保留于 [LICENSE.txt](modules/cambridge-dictionary-api/LICENSE.txt)。

本机 JS 改版位于 `modules/cambridge-dictionary-api/`；桌面 Rust 解析器移植了其中的选择器、字段结构、文本提取与顺序。新增连接管理、校验、候选、选区读取和桌面交互的范围见[来源说明](docs/ATTRIBUTION.md)及 [UPSTREAM.md](modules/cambridge-dictionary-api/UPSTREAM.md)。本声明不将 MIT 许可扩展为整个 WordSide 自有工程的许可。

## 原生依赖

构建工具从锁定的 Windows 原生依赖图收集第三方许可，保守包含构建依赖；补充文件与来源位于 [third-party](third-party/README.md)。程序、安装包和便携包提供同版 `THIRD-PARTY-LICENSES.txt`，包含 Nongfsq / EliasChen MIT 声明。桌面发布包不包含 Node 或 JS 服务依赖。

## 词典内容

释义、例句和音频来自 [Cambridge Dictionary](https://dictionary.cambridge.org/)，相关内容权利属于其权利人。WordSide 是独立的软件项目，与 Cambridge 无官方关联；第三方源码许可不授予词典内容版权。
