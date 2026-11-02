# 开发指南

[架构](ARCHITECTURE.md) · [贡献与分支](../CONTRIBUTING.md) · [发布流程](RELEASING.md) · [产品开发文档](PRODUCT-DEVELOPMENT.md)

## 准备环境

| 工具 | 用途 |
|---|---|
| Node `.node-version` 中的 24.18.0 | TypeScript、前端构建及 JS 对照测试 |
| pnpm 9.15.9 | 独立 Cambridge 模块的锁定依赖 |
| Rust stable，Windows MSVC 目标 | 原生后端 |
| Visual Studio C++ Build Tools + Windows SDK | Windows 编译、链接 |
| Microsoft Edge WebView2 | 桌面界面运行 |

Node 与 pnpm 是开发工具，不进入桌面发布包。依赖按锁文件安装，不依赖其他工作区或旧源码。

## 安装与快速检查

首次获取项目并运行检查：

```sh
git clone https://github.com/caoshuo2003/wordside.git
cd wordside
npm ci
npm exec --yes --package=pnpm@9.15.9 -- pnpm --dir modules/cambridge-dictionary-api install --frozen-lockfile
npm test
npm run check
npm run build
```

- `npm test`：独立 API、JSON、候选、选词清理与用户提示分类；HTTP 测试只使用临时本机端口。
- `npm run check`：TypeScript 类型检查。
- `npm run build`：生成当前 UI 到忽略的 `desktop/dist/`。

Linux 可运行 JS/TS 检查与浏览器预览。预览使用示例词条，不代表 Windows 原生取词或桌面兼容性。

## Windows 原生检查与构建

在 PowerShell 7 中运行：

```powershell
.\tools\windows-cargo.ps1 -Action check
.\tools\windows-cargo.ps1 -Action test
.\tools\windows-cargo.ps1 -Action bundle
```

脚本从 PATH 查找 Node / Rust，用 vswhere 查找 MSVC；也可从 x64 Developer PowerShell 启动。先生成锁定依赖的第三方许可与 UI，再执行 Cargo / Tauri。

可用 `-NodeExecutable` 指定 Node，`-TargetDir` 指定独立编译缓存；仅调整本次构建进程环境。`-DevToolsRoot` 是可选自定义工具链布局，不是普通克隆的必需条件。

默认成品：

```text
desktop/src-tauri/target/release/wordside.exe
desktop/src-tauri/target/release/bundle/nsis/WordSide_0.2.0_x64-setup.exe
```

若指定 CARGO_TARGET_DIR，输出位于对应缓存目录。生成许可在 `desktop/src-tauri/generated/`，Tauri schema 在 `gen/`，均不提交源码。

默认原生 test 检查 HTTP、解析、取消、缓存、设置与纯逻辑；跳过 `capture::` / `dismissal::`。可以用 `-TestFilter 'capture::shortcut::'` 单独检查纯快捷键解析；这项同样不操作桌面。取词事务相关检查可分别指定以下过滤器，均不注入输入、不打开测试窗口、不读写系统剪贴板：

```text
selection_input::tests::
capture::tests::
capture::copy_policy::
capture::timing::
capture::windows_copy::tests::
capture::windows_copy::ole::tests::
capture::input::tests::
capture::windows_text::semantics_tests::
capture::diagnostics::tests::
capture::hotkey::tests::
capture::windows::tests::
capture::compatibility::tests::
capture::selection::tests::
capture::probe::protocol_tests::
```

其中 `windows_copy` 使用可丢弃的 Win32 内存数据，OLE 测试使用进程内数据对象，不连接系统剪贴板；`protocol_tests` 使用私有管道和合成子进程。不要用宽泛的 `capture::` 过滤器代替上述检查，它还包含会操作窗口的实机夹具。

过滤和跳过不算通过用例。真实选词、焦点、DPI、托盘与安装应单独验收，在 PR 或本地任务记录中注明实际覆盖与未验项。

## 0.3.0 原生卡片（apps/wordside）

独立的 Cargo 工程，不需要 Node（只有生成第三方许可时要用）。在 PowerShell 7 中：

```powershell
cargo fmt --manifest-path apps/wordside/Cargo.toml -- --check
cargo clippy --manifest-path apps/wordside/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path apps/wordside/Cargo.toml
node tools/build-licenses.mjs --crate apps/wordside
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
- 第三方许可要在正式编译前生成，否则 exe 里只有一行提示，`check-wordside.ps1` 会判为失败。
- `check-wordside.ps1 -Runtime` 用临时设置目录启动程序，测托盘待命时的内存和空闲 CPU；不加 `-Runtime` 只做 CI 也能做的静态检查。
- 编译时 `build.rs` 用 Windows SDK 的 `rc.exe` 嵌入图标；没有 SDK 也能编译，只是 exe 没有图标。改了 `src/icon.rs` 的图形后用 `wordside.exe --export-icon apps\wordside\assets\wordside.ico` 重新导出。

开发时的启动参数（会直接显示卡片，也不参与单实例）：

| 参数 | 作用 |
|---|---|
| `--payload <json>` | 显示保存的词条，如 `apps/wordside/dev/sample-run.json` |
| `--parse <html>` | 解析保存的网页并显示 |
| `--first-paint` | 打印启动到首次绘制的毫秒数后退出 |
| `--stay` | 失去焦点时不自动隐藏 |
| `--light`、`--dark`、`--opaque`、`--gpu` | 强制外观、关闭玻璃、改用 GPU 渲染 |
| `--sound <file>` | 所有读音都播放这个本地文件，用于在访问不到 Cambridge 的电脑上测发音 |

正常启动（不带参数）只在托盘待命，读写真实设置。

## 解析对照与诊断

原始 Nongfsq 解析器在 `modules/cambridge-dictionary-api/tests/upstream/` 按原文保留；桌面 Rust 使用合成夹具对照完整 JSON。

解析契约变化后执行：

```sh
node tools/generate-native-fixtures.cjs
npm test
```

再运行受影响的 Windows 原生测试。真实网页内容与私人查询输出不写入源码库。

主程序提供初始化桌面前返回的诊断入口：

```powershell
.\release\v0.2.0\WordSide_0.2.0.exe --wordside-dictionary-probe en-cn:terminal en:runtime
.\release\v0.2.0\WordSide_0.2.0.exe --wordside-dictionary-parse C:\temp\sample-page.html
.\release\v0.2.0\WordSide_0.2.0.exe --wordside-dictionary-licenses
```

探针不使用桌面缓存；后端计时不等于从选词到弹窗的完整耗时。在线查询不作为 CI 必需测试。Release 文件需先自行下载或按发布步骤生成。

### 剪贴板备份诊断

仅在用户同意访问当前剪贴板时运行：

```powershell
.\tools\diagnose-clipboard-formats.ps1
.\tools\diagnose-clipboard-backup.ps1 -Executable .\release\v0.2.0\WordSide_0.2.0.exe
```

第一项只枚举格式编号和名称；第二项读取并临时复制当前数据到内部辅助进程内存，退出即释放，只输出状态、步骤、格式编号、错误码和耗时。两者都不发送按键、不清空或写入剪贴板，也不启动产品界面。取词故障优先查看设置目录的 `capture-status.json`：它只覆盖保存最近一次失败的阶段与状态，不含选词内容。上述只读诊断工具用于进一步排查特定状态。

## GitHub CI

`.github/workflows/ci.yml` 在 main push、目标为 main 的 PR 与手动触发时执行：

- Linux：锁文件安装、JS 测试、TypeScript 与 UI 构建。
- Windows：同样检查，另外执行原生逻辑、纯快捷键解析及上述取词事务检查。
- Windows（apps/wordside）：格式、clippy、测试、生成第三方许可、正式编译、`check-wordside.ps1` 发布自检；依赖和编译结果有缓存。

CI 覆盖源码与自动化检查；真实桌面、安装和发布各自单独执行并记录。远端检查状态以 GitHub Actions 的实际结果为准。

## 界面素材

截图使用当前 UI 和示例数据，说明与维护方法见 [SCREENSHOTS.md](SCREENSHOTS.md)。更新样式后再更新相关图片，避免文档与实际产品漂移。
