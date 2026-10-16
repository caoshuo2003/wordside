<p align="center">
  <img src="desktop/ciban-icon.png" width="88" alt="WordSide · 词伴图标">
</p>
<h1 align="center">WordSide · 词伴</h1>
<p align="center">选中一个词，读懂它。<br>一个安静、紧凑的 Windows 划词词典。</p>
<p align="center"><strong>0.1.1 · Windows x64 · Cambridge Dictionary</strong></p>
<p align="center">
  <a href="https://github.com/caoshuo2003/wordside/releases/tag/v0.1.1">下载 WordSide</a> ·
  <a href="docs/USER-GUIDE.md">使用指南</a> ·
  <a href="docs/FAQ.md">常见问题</a> ·
  <a href="docs/PRIVACY.md">隐私与数据</a>
</p>

**WordSide · 词伴** 是一款 Windows 划词词典，帮你在阅读旁查看 Cambridge Dictionary 的释义、音标、例句与发音。开启自动划词后，在支持读取选区的应用里选中英文单词，即可弹出浮窗；也可以按快捷键主动查询。

![WordSide · 词伴日间与夜间界面](docs/images/overview.png)

*界面预览，使用示例词条。[图片说明](docs/SCREENSHOTS.md)*

**0.1.1 更新**：改进 PDF 选词、快捷键响应和首尾符号处理。[查看本版变化](docs/releases/v0.1.1.md)

## 下载与开始

从 [0.1.1 发布页](https://github.com/caoshuo2003/wordside/releases/tag/v0.1.1) 下载，选择一种方式即可：

| 方式 | 下载 | 使用方式 |
|---|---|---|
| 安装版 · 推荐 | [Windows x64 安装包](https://github.com/caoshuo2003/wordside/releases/download/v0.1.1/WordSide_0.1.1_x64-setup.exe) | 按提示安装，然后打开 WordSide |
| 便携版 | [ZIP 压缩包](https://github.com/caoshuo2003/wordside/releases/download/v0.1.1/WordSide_0.1.1-portable.zip) | 解压全部文件，运行其中的 WordSide |

需要 **Windows x64**、**Microsoft Edge WebView2** 和能访问 Cambridge 的网络。查词功能已内置，安装后即可开始。

1. 打开 WordSide，在“手动查词”中输入一个英文单词，点击“查询”。
2. 想选中就查，打开“自动划词”；想自己触发，选中单词后按 **Ctrl+Alt+A**。快捷键可以在设置里修改。
3. 把浮窗拖到顺手的位置，调整大小，再选一套喜欢的配色。

[完整使用指南](docs/USER-GUIDE.md) · [安装问题](docs/FAQ.md#安装与运行) · [取词与已知限制](docs/FAQ.md#取词与快捷键) · [隐私与数据](docs/PRIVACY.md) · [本版说明](docs/releases/v0.1.1.md)

## 查词时，你可以

| 体验 | 使用方式 |
|---|---|
| 边读边查 | 自动划词、快捷键和手动输入，自由选择 |
| 看释义、听发音 | 阅读英汉或英文释义、例句与音标，点击播放英式或美式发音 |
| 调整浮窗 | 拖动、缩放，自动记住大小；固定后在同一窗口继续查新词 |
| 少操心一点选词 | 自动清理首尾标点；没查到时，查看词典提供的拼写建议 |
| 换一种阅读氛围 | 薄荷绿、雾紫、浅蓝、暖灰，搭配日间、夜间或跟随系统 |
| 随时收起或退出 | 关闭主窗口时选择隐藏到托盘或退出，双击托盘重新打开 |

取词效果会随应用和文本区域而变化。读不到选区时，可以在 WordSide 中手动输入继续查词；Windows 桌面终端中的选词也遵循这一规则。连接或速度问题见[网络与速度](docs/FAQ.md#网络与速度)。

查询词会发送到 Cambridge；个人设置保存在本机，词条只在内存中短期缓存，退出后清空。详细说明见[隐私与本地数据](docs/PRIVACY.md)。

## 已知问题

**偶发无法使用**：如果软件突然无法正常使用，可在 Windows 右下角托盘右键 WordSide，选择“退出 WordSide”，再重新打开。已记录的一次异常通过此方式恢复；问题尚未再次复现，暂未修复。

## 参考项目与致谢

**WordSide 的 Cambridge 解析实现基于 [Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api) 修改与移植。** 该项目继承自 [chenelias/cambridge-dictionary-api](https://github.com/chenelias/cambridge-dictionary-api)，保留了原作者 **EliasChen** 的署名与 MIT 许可。感谢原项目提供的解析实现。

词典内容来自 [Cambridge Dictionary](https://dictionary.cambridge.org/)。WordSide 是独立项目，与 Cambridge 无官方关联。仓库已公开，自有代码的开源许可尚未选定；第三方组件遵循各自许可。详见[项目来源](docs/ATTRIBUTION.md)与[许可声明](NOTICE.md)。

## 反馈与参与

遇到问题或有新想法，欢迎[提交 Issue](https://github.com/caoshuo2003/wordside/issues)。描述软件版本、出现问题的应用和操作步骤，会更容易定位；截图请遮住私人信息。

[开发指南](docs/DEVELOPMENT.md) · [贡献与分支约定](CONTRIBUTING.md) · [路线图](docs/ROADMAP.md) · [更新记录](CHANGELOG.md)
