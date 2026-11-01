# WordSide 代理工作说明

这里是适用于所有人的规则，随仓库提交。本机情况（如网络、本地工具）写在被忽略的 `AGENTS.local.md`，没有这个文件也不影响使用。

产品和计划以仓库文档为准：

- 开发计划与问题清单：[docs/PRODUCT-DEVELOPMENT.md](docs/PRODUCT-DEVELOPMENT.md)
- 架构：[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 环境与测试：[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)
- 分支与提交：[CONTRIBUTING.md](CONTRIBUTING.md)；发布：[docs/RELEASING.md](docs/RELEASING.md)

## 项目概况

Windows x64 划词词典，Tauri 2 + Rust + TypeScript（无前端框架）。

| 位置 | 内容 |
|---|---|
| `desktop/src-tauri/src/capture/` | Windows 取词，风险最高 |
| `desktop/src-tauri/src/dictionary/` | Cambridge 请求与解析 |
| `desktop/src-tauri/src/main.rs` | 应用层，待拆分 |
| `desktop/src/`、`shared/` | 界面和 JSON 校验 |
| `modules/cambridge-dictionary-api/` | 上游 JS 解析器，只用于行为对照，不进发布包 |

## 常用命令

```powershell
npm ci
npm exec --yes --package=pnpm@9.15.9 -- pnpm --dir modules/cambridge-dictionary-api install --frozen-lockfile
npm test
npm run check
npm run build
.\tools\windows-cargo.ps1 -Action test
.\tools\windows-cargo.ps1 -Action test -TestFilter 'capture::copy_policy::'
```

取词相关的完整过滤器清单以 `.github/workflows/ci.yml` 为准。

## 必须遵守

- **不要运行宽泛的 `capture::` 过滤器。** 它包含操作真实窗口的实机夹具。
- **不绕过 Cambridge 的人机验证。** 不伪装指纹，不自动完成验证，不逐个换代理出口。
- **不改应用标识** `local.wordside.dictionary.api-preview`，否则用户设置会丢失。
- **不放宽** CSP、`open_source` 白名单和隐私边界（词条不落盘，诊断文件不含单词和剪贴板内容）。
- **保留** Nongfsq / EliasChen 的署名和 MIT 许可。
- **解析器变化** 要验证完整字段和顺序，不能静默丢内容。
- **不提交** 依赖、编译缓存、Release 二进制、真实网页、选词内容或凭据。

## 改动方式

- `main` 不直接提交。每项改动一条短期分支，走 PR，CI 通过后 squash 合并。
- `refactor:` 不改行为；移动代码和修改逻辑分开提交；重构中发现的 bug 开 `fix/` 分支单独修。
- 测试数量不能减少；PR 如实写明跑过什么、没测什么，模拟结果不能算实机通过。
- 取词层不做整体拆分，只在修具体问题时一次动一个模块。
- 推送、创建 PR、合并、打标签、发布前都要先得到维护者同意。

## 排查提示

`CAMBRIDGE_BLOCKED` 可能只是当前网络被 Cloudflare 要求人机验证，不一定是代码回归。用 `--wordside-dictionary-parse` 和测试夹具可以把解析问题和网站访问问题分开。
