# 项目维护与贡献

WordSide · 词伴的源码已在 [caoshuo2003/wordside](https://github.com/caoshuo2003/wordside) 公开。欢迎通过 Issue 报告问题、讨论想法；代码许可见 [NOTICE](NOTICE.md)。

**main 保存可发布代码；每项改动使用一条短期分支，通过 PR 检查和合并。**

## 一次改动的流程

1. 用 Issue 写清触发场景、想改变的行为与完成条件。
2. 从最新 main 建分支，专注解决一项可验收问题。
3. 实现并运行受影响检查，更新必要文档与 CHANGELOG 的 Unreleased。
4. 提交后 push 任务分支，创建 PR，说明问题、结果与实际验证。
5. 查看 CI 与 diff，解决失败后合并；可以 squash，让 main 保持清晰。
6. 合并后删除不再需要的远端分支，同步本机 main。

先确认工作树中的已有改动归属，不覆盖或重置其他任务。

```sh
git switch main
git pull --ff-only
git switch -c docs/improve-guide
# 修改相关文档并检查链接
git add README.md docs/USER-GUIDE.md
git diff --cached
git commit -m "docs: 完善首次使用教程"
git push -u origin docs/improve-guide
# 在 GitHub 创建 PR，检查 CI 后合并
```

`--ff-only` 失败时先看历史并处理分歧，不用强制重置绕过。任务分支日常维护不使用强推。

## 分支与提交

| 工作 | 分支例子 | 提交前缀 |
|---|---|---|
| 功能 | `feat/selection-history` | `feat:` |
| 修复 | `fix/popup-dismiss` | `fix:` |
| 工程 | `chore/build-portability` | `chore:` |
| 文档 | `docs/getting-started` | `docs:` |
| 测试 | `test/windows-acceptance` | `test:` |
| 未确定实验 | `exp/lookup-latency` | 说明实验目的 |

实验先记录方法与结论，有价值后再接入确定功能；不要把未完成实验直接合进 main。提交说明可中文或英文，应该能看懂变化。

## 验证与内容边界

JS/TS 修改执行 `npm test` / `npm run check` 的受影响范围；Windows 后端修改执行相应原生测试。安装、真实取词、焦点和 DPI 单独验证，不能用模拟冒称实机通过。检查命令、结果与未测项填写在 PR。

只提交软件源码、锁文件、测试、资源、文档、项目 CI 和许可证。不提交依赖、编译缓存、Release 二进制、浏览器配置、真实网页、选词、设置、凭据或本机任务记录。

保留 Nongfsq / EliasChen 来源与 MIT 许可；解析器变化须验证完整字段和顺序，不静默丢弃内容。

## 版本与发布

当前版本为 **0.1.1 正式版**。修复可用 0.1.2，新功能可用 0.2.0；普通文档修改保持应用版本不变。

发布从已检查的 main 创建不可移动的版本标签，流程见 [RELEASING.md](docs/RELEASING.md)。成品放 GitHub Release，不进入源码提交。main 的保护策略按当前账号可用能力配置；无须为单人起步强制要求第二个审查者。
