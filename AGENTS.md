# WordSide 代理工作说明

这里是适用于所有人的规则，随仓库提交。本机情况（如网络、本地工具）写在被忽略的 `AGENTS.local.md`，没有这个文件也不影响使用。

产品和计划以仓库文档为准：

- 开发计划与问题清单：[docs/PRODUCT-DEVELOPMENT.md](docs/PRODUCT-DEVELOPMENT.md)
- 架构：[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 环境与测试：[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)
- 分支与提交：[CONTRIBUTING.md](CONTRIBUTING.md)；发布：[docs/RELEASING.md](docs/RELEASING.md)

## 项目概况

Windows x64 极简划词词典：纯 Rust + Direct2D，单个 exe。代码全部在 `apps/wordside/`；0.2.0（Tauri）已在 0.3.1 之后从仓库删除，需要时看 `v0.2.0` 标签。

| 位置 | 内容 |
|---|---|
| `apps/wordside/src/capture/` | Windows 取词，从 0.2.0 整体搬来，风险最高：判断逻辑和时序不随手改 |
| `apps/wordside/src/dictionary/`、`lookup.rs` | 读取单词 API，只搬不改 |
| `apps/wordside/src/app.rs`、`render.rs`、`card.rs` | 窗口、绘制、卡片状态 |
| `modules/cambridge-dictionary-api/` | 上游 JS 解析器，用于对照和生成测试数据（`tools/generate-native-fixtures.cjs`），不进发布包 |

## 常用命令

```powershell
cargo fmt --manifest-path apps/wordside/Cargo.toml
cargo clippy --manifest-path apps/wordside/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path apps/wordside/Cargo.toml
node tools/build-licenses.mjs
cargo build --manifest-path apps/wordside/Cargo.toml --release
.\tools\check-wordside.ps1 -Exe apps/wordside/target/release/wordside.exe -Runtime
```

`cargo test` 不操作桌面、不连网，也不碰系统剪贴板。

## 必须遵守

- **不要直接跑 `cargo test -- --ignored`。** 被忽略的是操作真实窗口、剪贴板和声音设备的实机测试，按[开发指南](docs/DEVELOPMENT.md)逐个指定运行。
- **不绕过 Cambridge 的人机验证。** 不伪装指纹，不自动完成验证，不逐个换代理出口。
- **不改应用标识** `local.wordside.dictionary.api-preview`，否则用户设置会丢失。
- **读取单词 API 只搬不改。** 除路径和模块引用外不改逻辑，输出的 JSON 不变。
- **守住性能预算。** 不引入 WebView、前端框架或内置字体；超出产品开发文档 3.1 节的预算要说明原因。
- **不放宽** 外部链接白名单和隐私边界：词条缓存只在内存；不保存查询记录；诊断文件不含单词和剪贴板内容。
- **保留** Nongfsq / EliasChen 的署名和 MIT 许可。
- **解析器变化** 要验证完整字段和顺序，不能静默丢内容。
- **不提交** 依赖、编译缓存、Release 二进制、真实网页、选词内容或凭据。

## 改动方式

- `main` 不直接提交。按里程碑开短期分支和 PR，squash 合并，不为每个小改动单独开 PR。
- CI 按改了哪些文件执行，规则见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md#github-ci)：改了代码的 PR 等对应检查通过再合并；只改文档的 PR 不跑 CI；合并到 main 后不再重跑。不要为了“保险”手动重跑已经通过的同一份代码。
- 文档和计划攒在本地，到一个阶段再随下一个 PR 一起推送。发布走 PR；只改版本号和文档的发布 PR，代码已在之前的 PR 检查过，维护者可以不等 CI 合并。
- `refactor:` 不改行为；移动代码和修改逻辑分开提交；重构中发现的 bug 开 `fix/` 分支单独修。
- 测试数量不能减少；PR 如实写明跑过什么、没测什么，模拟结果不能算实机通过。
- 改取词的判断逻辑或时序，一个 PR 只动一处，并附实机测试结果。
- 推送、创建 PR、合并、打标签、发布前都要先得到维护者同意。

## 排查提示

`CAMBRIDGE_BLOCKED` 可能只是当前网络被 Cloudflare 要求人机验证，不一定是代码回归。用 `--wordside-dictionary-parse` 和测试夹具可以把解析问题和网站访问问题分开。
