# WordSide 市场调研

调研日期：2026-11-02。这是一次桌面调研，来源是 GitHub 数据、产品官网、应用商店和论坛讨论，**没有做用户访谈，也没有市场规模数据**。论坛反馈样本很小，只能当线索。“没找到”只代表这次搜索没发现，不代表市场上一定没有。

**最终决定：** 维护者看过本报告后，选择了不在下表中的方向：参考 [frun](https://github.com/Nongfsq/frun) 做极简、极致轻量的原生查词工具（见[产品开发文档](PRODUCT-DEVELOPMENT.md)）。下面的“语境词典”留作以后可以叠加的方向。

## 结论

1. **“划词查词 / 划词翻译”本身已经是拥挤的市场。** 免费开源的工具很多，WordSide 现在的功能不足以让人换用。
2. **句子翻译不是差异点。** 几乎所有竞品都有，且接入的模型比较接近。建议移出 0.3.0 的重点。
3. **比较明显的空位是：** 用权威词典的内容，告诉用户“这个词在这句话里是哪个意思”，并把查词时的句子留下来。相关的产品有，但这次没找到把这些组合起来、面向 Windows 中文读者的产品。
4. **最大的风险不在竞争，而在词典来源：** 依赖抓取 Cambridge 网页，既可能被人机验证拦下，授权也没有解决。

## 竞品

| 产品 | 类型 | 平台 | 现状 | 主要特点 | 对 WordSide 的启示 |
|---|---|---|---|---|---|
| [欧路词典](https://www.eudic.net/v4/en/home/buy) | 商业词典 | 全平台 | 官网列电脑版买断 168 元，另有 AI 会员 | 词库丰富、生词本、同步、AI | 词库和生态比不过，不要正面竞争 |
| [Pot](https://github.com/pot-app/pot-desktop) | 开源翻译 | Win/Mac/Linux | 约 1.9 万星，**仓库已归档**，最后发布 2025-05 | Tauri、多翻译源、OCR、插件 | 用户需要新去处；有用户嫌窗口位置和大小不会适配 |
| [STranslate](https://github.com/STranslate/STranslate) | 开源翻译 | Windows | 约 8500 星，持续更新 | WPF、翻译和 OCR、响应快 | Windows 上最直接的翻译竞品 |
| [NextAI Translator](https://github.com/nextai-translator/nextai-translator) | 开源 AI 翻译 | 跨平台、插件 | 约 2.5 万星，持续更新 | 大模型划词翻译 | 大模型翻译已是常见能力 |
| [沙拉查词](https://github.com/crimx/ext-saladict) | 开源扩展 | 浏览器 | 约 1.3 万星，持续更新 | 多词典聚合、生词本、PDF | 只覆盖浏览器 |
| [Easydict](https://github.com/tisfeng/Easydict) | 开源词典翻译 | macOS | 约 1.5 万星，持续更新 | 简洁，开箱即用 | 不是 Windows，但证明“简洁”有人要 |
| [WordWeb](https://wordweb.info) | 商业词典 | Windows | 活跃，Pro 版 10.62 | 任意程序一键查词；联网时提供 AI 语境释义和翻译 | **最接近的竞品。** 官网未说明是否支持中文，也未提及保存句子 |
| [SelectAndTranslate](https://wingetgui.com/apps/YanaYuan-SelectAndTranslate) | 免费工具 | Windows | 小众，信息来自开发者和软件目录 | GPT-4o 划词翻译；开发者称会自动记录词和所在句子，可导出到 Anki | 说明“留句子”已有人做；重点是翻译和背词 |
| [Context](https://apps.apple.com/qa/app/context-accurate-word-lookup/id6477352612) | 商业应用 | iPhone、Mac | 最后更新 2024-05 | 拍照或导入图片，解释词在上下文中的意思 | 语境查词有人做，但不是 Windows，也不是划词 |
| [WordNote](https://chromewebstore.google.com/detail/chiibaoahgbiihmhfpdajmkapcngjobm)、[VocabAI](https://chromewebstore.google.com/detail/bmaphbnbbnaobnpiedcjkhlehbfjonko) | 扩展 | 浏览器 | 小众 | 划词后保存句子，AI 生成释义 | 只覆盖浏览器，释义由 AI 生成 |

星数和更新状态来自 2026-11-02 的 GitHub 数据。

## 用户反馈线索

来自 [V2EX 讨论 1](https://www.v2ex.com/t/1143953)、[讨论 2](https://v2ex.com/t/910296) 和 [Pot 的 Issue](https://github.com/pot-app/pot-desktop/issues)，都是个别用户的反馈：

| 主题 | 线索 |
|---|---|
| 收费和限额 | 求替代品的起因就是原插件开始限制每月划词次数 |
| 稳定性 | 有用户称 Easydict 导致 VS Code 复制失效；Pot 有“经常无法唤起翻译窗口”的 Issue |
| 窗口体验 | 位置、大小不适配；窗口不在主屏；深色模式闪白 |
| 大模型延迟 | 非流式响应时，长文本要等好一会 |
| PDF 取词 | [选区错位](https://forums.zotero.org/discussion/comment/420351/)、[取出乱码](https://bbs.wps.cn/topic/43232)，[划词翻译文档](https://hcfy.ai/en/docs/guides/pdf)也承认 PDF 有难解决的问题 |

## 大模型和词典

[Lew 等人 2024 年的实验](https://academic.oup.com/ijl/article/37/3/322/7706426)用 ChatGPT 3.5 对比两部词典：ChatGPT 在写作类任务上更好；在阅读理解上胜过英英词典，但**不如双语词典**。[相关综述](https://elex.link/ojs/index.php/elex/article/view/35)也提醒生成内容可能有虚构信息。样本是特定任务和旧模型，结论不能直接外推。

这支持一个思路：**不让 AI 写释义，只让它从权威词典的义项里挑选。** 这样既有语境，内容也能核对。这是根据研究做的推断，还没有验证过。

## 差异化方向

| 方向 | 做什么 | 优点 | 风险 | 判断 |
|---|---|---|---|---|
| **A. 语境词典** | 读取选中词所在的句子，在词典义项中标出这句话用的是哪一个；自动留下句子和来源；再次遇到时提示“你查过它” | 解决“一个词 31 条释义看不完”；比纯 AI 可信；不增加操作负担 | 依赖读取句子的能力和 AI 选义的准确率；句子要发给 AI | **推荐** |
| B. 程序员专用 | 拆分 `getUserById`、`snake_case`，针对终端、代码和文档优化 | WordSide 已在终端上下过功夫，范围明确 | 用户群窄，是否愿意换工具未知 | 可作为 A 的首批目标用户 |
| C. 完全本地 | 不联网，本机词库和本地模型 | 隐私卖点明确，不受拦截影响 | 中文释义质量和包体积是硬问题 | 先不做；ECDICT 可作兜底 |
| D. 更好的翻译器 | 多翻译源、OCR | 需求最明确 | 竞品最多，没有优势 | **不建议** |

### 方向 A 的具体形态

```text
terminal            英 /ˈtɜː.mɪ.nəl/
在这句话里 ▸ 3. 终端（用来输入命令的程序）
“Open a terminal and run the command.”  ← 来自 VS Code
你上周也查过它

其他释义
1. 航站楼；总站
2. （疾病）晚期的
```

- **AI 只选编号，不写释义。** 显示的内容都来自词典，可以直接核对。
- **没有 AI 也能用。** 不配置或超时，就和现在一样显示完整释义。
- **句子只在用户开启后才发送。** 这是产品的隐私底线。
- **历史要改：** 需要保存句子和来源应用，比之前“只存词”（D-4）范围更大，要明确告知，并提供关闭和清空。

## 动手前要验证

这些都没验证过，建议先花几天做完，再决定是否投入：

| 验证 | 怎么做 | 通过标准（建议） |
|---|---|---|
| 能否读到句子 | 用现有 UIA 代码，在 Chrome、Edge、记事本、VS Code、PDF、终端各试一次 | 主要阅读场景大多能读到；读不到时能退回普通查词 |
| AI 选义准不准 | 准备 50 个多义词句子，用真实词典义项测试 | 准确率 90% 以上 |
| 够不够快 | 记录词典和 AI 各自的耗时 | 选义结果 2 秒内出现，且不阻塞释义显示 |
| 用户要不要 | 用现有设计稿或演示视频，找 5–10 位经常读英文的人聊 | 多数人愿意换用或付费，而不只是说“不错” |

## 必须先解决的风险

- **词典来源：** 方向 A 高度依赖词典义项，而现在靠抓取 Cambridge 网页。本机已被人机验证拦下。[Cambridge 授权页面](https://dictionary.cambridge.org/license)这次打不开，授权条款没有核实。做大之前应当申请正式授权，或换来源。[Merriam-Webster API](https://dictionaryapi.com/info/terms-of-service) 的免费版不允许商业使用，商业版要单独谈。[ECDICT](https://github.com/skywind3000/ECDICT) 是 MIT 许可，可作兜底，但义项粒度和 Cambridge 不同，不一定适合做选义，也要核实数据本身的来源。
- **AI 成本：** 继续由用户自备密钥，门槛高，会挡住很多普通用户。如果要开箱即用，就要自建服务并承担费用，这是产品和商业决定。
- **自有代码没有开源许可：** 影响别人贡献和使用。
