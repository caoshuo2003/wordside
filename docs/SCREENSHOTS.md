# 界面截图

文档里的图片是 0.3.0 卡片的真实截图：真实的 WordSide 程序显示示例词条 `record`（`apps/wordside/dev/sample-run.json`），背后是一篇自写的英文短文。图片按 1.5 倍（屏幕 150% 缩放）截取，没有后期添加控件或文字。

| 文件 | 内容 |
|---|---|
| `images/demo.gif` | README 动图：阅读页 → 选中单词 → 卡片出现，循环播放 |
| `images/preview-light.png` | 浅色：选中 `record`，卡片在选区下方 |
| `images/preview-dark.png` | 深色：同一场景 |
| `images/icon.png` | 软件图标，由 `apps/wordside/assets/wordside.ico` 导出 |

## 怎么重新截

1. 用一个单独的 Edge 配置目录全屏打开阅读页（`--user-data-dir` 指向 `.cache` 下的临时目录，`--kiosk`），页面加载后用脚本自己选中单词，不向浏览器发送任何输入。
2. 用开发参数启动卡片：`wordside.exe --payload apps/wordside/dev/sample-run.json --light --stay`（深色用 `--dark`）；查词框会显示示例词，和真实查词一样。把卡片窗口移到选区下方。
3. 截图前把阅读页和卡片暂时置顶，只截这两者，截完恢复窗口顺序；截图进程要 DPI 感知，按物理像素截取。
4. 动图：分别截“未选中”“已选中”“卡片”三张，缩小到 850×670，加淡入淡出，用 `gifenc` 编码成 GIF（工具放在 `.cache`，不进仓库）。

更新界面后同步图片；使用示例词条和自写文字，不含私人信息。涉及真实桌面截图时，先征得当前操作者同意，并确认画面里没有其他窗口。
