# 开发指南

[架构](ARCHITECTURE.md) · [贡献与分支](../CONTRIBUTING.md) · [发布流程](RELEASING.md) · [产品开发文档](PRODUCT-DEVELOPMENT.md)

## 准备环境

| 工具 | 用途 |
|---|---|
| Rust stable，Windows MSVC 目标 | 编译 |
| Visual Studio C++ Build Tools + Windows SDK | 链接；SDK 的 `rc.exe` 把图标和版本信息嵌进 exe |
| Node（版本见 `.node-version`） | 生成第三方许可；重新生成解析测试数据 |
| pnpm 9.15.9 | 只在重新生成解析测试数据时用，安装上游模块的锁定依赖 |
| NSIS | 只在打安装版时用 |

Node 和 pnpm 是开发工具，不进发布包。程序只有一个 Cargo 工程 `apps/wordside/`；0.2.0（Tauri + TypeScript）已从仓库删除，需要时看 `v0.2.0` 标签。

## 检查与构建

在 PowerShell 7 中：

```powershell
cargo fmt --manifest-path apps/wordside/Cargo.toml -- --check
cargo clippy --manifest-path apps/wordside/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path apps/wordside/Cargo.toml
node tools/build-licenses.mjs
cargo build --manifest-path apps/wordside/Cargo.toml --release
.\tools\check-wordside.ps1 -Exe apps/wordside/target/release/wordside.exe -Runtime
```

- `cargo test` 不操作桌面、不连网，也不读写真实设置或系统剪贴板；有一个计时测试在机器很忙时偶尔失败，CI 会单独重试它。
- 取词层里会开真实窗口的夹具标了 `#[ignore]`，在不被打扰的桌面上手动跑。它们会弹出并很快关掉几个小窗口，不碰系统剪贴板（剪贴板测试用私有窗口站）：

  ```powershell
  cargo test --manifest-path apps/wordside/Cargo.toml -- --ignored --exact `
    capture::windows_text::tests::plain_rename_edit_keeps_only_selected_word `
    capture::windows_text::tests::ordinary_edit_unicode_filter_password_and_long_offsets `
    capture::windows_text::tests::unresponsive_native_edit_times_out_then_recovers `
    capture::windows_text::tests::native_text_controls_selection_owner_password_and_filtering `
    capture::probe::tests::hung_helper_is_reaped_and_next_real_selection_recovers `
    capture::windows_copy::isolated_tests::native_backup_round_trip_uses_private_window_station
  ```

  不要直接跑 `--ignored`：`helper_process`、`fixture_process`、`private_clipboard_process` 是给上面这些测试启动的子进程入口，单独运行会一直等输入。
- 发音播放要真实的声音设备，也标了 `#[ignore]`：音量设为 0，听不到声音，检查每次播放从第一个样本放到最后、快速切换时新的一段从头开始。

  ```powershell
  cargo test --manifest-path apps/wordside/Cargo.toml -- --ignored --exact audio::tests::every_sample_plays_from_the_first
  ```
- 第三方许可要在正式编译前生成，否则 exe 里只有一行提示，`check-wordside.ps1` 会判为失败。
- `check-wordside.ps1 -Runtime` 用临时设置目录启动程序，测托盘待命时的内存和空闲 CPU；不加 `-Runtime` 只做 CI 也能做的静态检查。
- 安装版：`.\tools\build-installer.ps1 -Exe <exe> -Licenses apps\wordside\generated\THIRD-PARTY-LICENSES.txt -OutDir <目录>`，需要 NSIS（`scoop install extras/nsis`）。安装和卸载可以加 `/S` 静默运行。
- 编译时 `build.rs` 用 Windows SDK 的 `rc.exe` 嵌入图标；没有 SDK 也能编译，只是 exe 没有图标。改了 `src/icon.rs` 的图形后用 `wordside.exe --export-icon apps\wordside\assets\wordside.ico` 重新导出。

过滤和跳过不算通过用例。真实选词、焦点、DPI、托盘与安装单独验收，在 PR 里注明实际覆盖与未验项。

开发时的启动参数（会直接显示卡片，也不参与单实例）：

| 参数 | 作用 |
|---|---|
| `--payload <json>` | 显示保存的词条，如 `apps/wordside/dev/sample-run.json` |
| `--parse <html>` | 解析保存的网页并显示 |
| `--first-paint` | 打印启动到首次绘制的毫秒数后退出 |
| `--stay` | 失去焦点时不自动隐藏 |
| `--light`、`--dark`、`--opaque`、`--gpu` | 强制外观、关闭玻璃、改用 GPU 渲染 |
| `--dpi <n>` | 按指定 DPI 绘制，如 `--dpi 96` 在高分屏上看 1080p（100%）的效果 |
| `--sound <file>` | 所有读音都播放这个本地文件，用于在访问不到 Cambridge 的电脑上测发音 |

正常启动（不带参数）只在托盘待命，读写真实设置。

## 解析对照与诊断

原始 Nongfsq 解析器在 `modules/cambridge-dictionary-api/` 按原文保留，只用来对照；`apps/wordside` 的 Rust 解析器用它生成的合成夹具核对完整 JSON。解析契约变化后，重新生成夹具并跑测试：

```powershell
npm exec --yes --package=pnpm@9.15.9 -- pnpm --dir modules/cambridge-dictionary-api install --frozen-lockfile
node tools/generate-native-fixtures.cjs
cargo test --manifest-path apps/wordside/Cargo.toml
```

夹具写到 `apps/wordside/tests/fixtures/`。真实网页内容与私人查询输出不写入源码库。

程序提供不打开窗口的诊断入口：

```powershell
wordside.exe --wordside-dictionary-probe en-cn:terminal en:runtime
wordside.exe --wordside-dictionary-parse C:\temp\sample-page.html
wordside.exe --wordside-dictionary-licenses
```

探针不使用卡片的缓存；后端计时不等于从选词到弹窗的完整耗时。在线查询不作为 CI 必需测试。

### 剪贴板备份诊断

仅在用户同意访问当前剪贴板时运行：

```powershell
.\tools\diagnose-clipboard-formats.ps1
.\tools\diagnose-clipboard-backup.ps1 -Executable apps\wordside\target\release\wordside.exe
```

第一项只枚举格式编号和名称；第二项读取并临时复制当前数据到内部辅助进程内存，退出即释放，只输出状态、步骤、格式编号、错误码和耗时。两者都不发送按键、不清空或写入剪贴板，也不启动产品界面。取词故障优先查看设置目录的 `capture-status.json`：它只覆盖保存最近一次失败的阶段与状态，不含选词内容。

## GitHub CI

`.github/workflows/wordside.yml` 只在目标为 main 的 PR 上、改到这些文件时执行：`apps/wordside/`、`tools/build-licenses.mjs`、`tools/check-wordside.ps1`、`.node-version` 和这个 workflow 本身。内容是格式、clippy、测试、第三方许可、正式编译和 `check-wordside.ps1`，依赖和编译结果有缓存，约 5 分钟。

- 只改文档（`*.md`、`docs/`）或其他文件时不执行。
- 合并到 main 后不再重跑：PR 上已经检查过同样的内容。
- 需要时可以在 Actions 里手动运行。

CI 覆盖源码与自动化检查；真实桌面、安装和发布各自单独执行并记录。远端检查状态以 GitHub Actions 的实际结果为准。

## 界面素材

截图使用当前 UI 和示例数据，说明与维护方法见 [SCREENSHOTS.md](SCREENSHOTS.md)。更新样式后再更新相关图片，避免文档与实际产品漂移。
