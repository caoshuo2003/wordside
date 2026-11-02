<p align="center">
  <img src="docs/images/icon.png" width="64" alt="WordSide · 词伴图标">
</p>
<h1 align="center">WordSide · 词伴</h1>
<p align="center">选中一个词，读懂它。<br>一个安静、轻巧的 Windows 划词词典。</p>
<p align="center"><strong>0.3.0 · Windows x64 · Cambridge Dictionary</strong></p>
<p align="center">
  <a href="https://github.com/caoshuo2003/wordside/releases/tag/v0.3.0">下载 WordSide</a> ·
  <a href="docs/USER-GUIDE.md">使用指南</a> ·
  <a href="docs/FAQ.md">常见问题</a> ·
  <a href="docs/PRIVACY.md">隐私与数据</a>
</p>

**WordSide · 词伴** 是一款 Windows 划词词典，帮你在阅读时就近查看 Cambridge Dictionary 的释义、音标、例句与发音。开启自动划词后，选中英文单词，旁边就会弹出一张玻璃质感的小卡片；也可以按快捷键主动查询。

**0.3.0 是重写的新一代**：整个程序用 Rust 原生重写，只有一个约 4 MB 的 exe，不再需要 WebView2 和安装；平时只在托盘待命，几乎不占内存。界面换成一张拟态玻璃卡片。[查看本版变化](docs/releases/v0.3.0.md)

## 下载与开始

从 [0.3.0 发布页](https://github.com/caoshuo2003/wordside/releases/tag/v0.3.0) 下载：

| 方式 | 下载 | 使用方式 |
|---|---|---|
| 单个程序 · 推荐 | [WordSide_0.3.0.exe](https://github.com/caoshuo2003/wordside/releases/download/v0.3.0/WordSide_0.3.0.exe) | 放到任意文件夹，双击运行 |
| 便携包 | [ZIP 压缩包](https://github.com/caoshuo2003/wordside/releases/download/v0.3.0/WordSide_0.3.0-portable.zip) | 解压后运行其中的程序，包内附使用说明和第三方许可 |

需要 **Windows x64** 和能访问 Cambridge 的网络。程序没有数字签名，第一次运行时 Windows 可能提示“已保护你的电脑”，点“更多信息”→“仍要运行”。

1. 运行后 WordSide 在任务栏右下角的托盘待命，不会弹出窗口。左键点托盘图标，可以直接输入单词查询。
2. 想选中就查，在托盘右键菜单里勾选“自动划词”；想自己触发，选中单词后按 **Ctrl+Alt+A**。快捷键可以在卡片的设置里修改。
3. 从 0.2.0 升级：先在托盘右键退出 0.2.0；用过安装版的，可以在 Windows“已安装的应用”里卸载它。快捷键、自动划词和外观设置会沿用。

[完整使用指南](docs/USER-GUIDE.md) · [运行问题](docs/FAQ.md#安装与运行) · [取词与已知限制](docs/FAQ.md#取词与快捷键) · [隐私与数据](docs/PRIVACY.md) · [本版说明](docs/releases/v0.3.0.md)

## 功能一览

| 体验 | 使用方式 |
|---|---|
| 边读边查 | 自动划词、快捷键和手动输入，自由选择；卡片出现在选区旁边，不抢走原窗口的焦点 |
| 看释义、听发音 | 中文释义在前，英文解释和例句随后；点击播放英式或美式发音 |
| 按词性浏览 | 词性标签显示各自的释义条数；例句按需展开 |
| 键盘也能用 | ↑↓ 选择释义，→ 展开例句，Tab 切换词性，Esc 收起 |
| 少操心一点选词 | 自动清理首尾标点；没查到时，查看词典提供的拼写建议 |
| 安静地待着 | 只在托盘待命，点卡片外面就收起；需要时用图钉固定卡片 |
| 日夜都好看 | 浅色、深色或跟随系统；系统关闭透明效果时自动换成不透明卡片 |

取词效果会随应用和文本区域而变化。读不到选区时，可以在卡片里手动输入继续查词。连接或速度问题见[网络与速度](docs/FAQ.md#网络与速度)。

查询词会发送到 Cambridge；个人设置保存在本机，词条只在内存中短期缓存，退出后清空。详细说明见[隐私与本地数据](docs/PRIVACY.md)。

## 参考项目与致谢

**WordSide 的 Cambridge 解析实现基于 [Nongfsq/cambridge-dictionary-api](https://github.com/Nongfsq/cambridge-dictionary-api) 修改与移植。** 该项目继承自 [chenelias/cambridge-dictionary-api](https://github.com/chenelias/cambridge-dictionary-api)，保留了原作者 **EliasChen** 的署名与 MIT 许可。感谢原项目提供的解析实现。

词典内容来自 [Cambridge Dictionary](https://dictionary.cambridge.org/)。WordSide 是独立项目，与 Cambridge 无官方关联。仓库已公开，自有代码的开源许可尚未选定；第三方组件遵循各自许可。详见[项目来源](docs/ATTRIBUTION.md)与[许可声明](NOTICE.md)。

## 反馈与参与

遇到问题时，可以先在托盘右键 WordSide 选择“退出”，再重新打开。若问题仍然存在或能够复现，欢迎[提交 Issue](https://github.com/caoshuo2003/wordside/issues)，并附上 WordSide 版本、相关应用和操作步骤；截图请遮住私人信息。新想法和使用建议也欢迎在 Issue 中交流。

[开发指南](docs/DEVELOPMENT.md) · [贡献与分支约定](CONTRIBUTING.md) · [路线图](docs/ROADMAP.md) · [更新记录](CHANGELOG.md)
