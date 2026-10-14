# 开发指南

[架构](ARCHITECTURE.md) · [贡献与分支](../CONTRIBUTING.md) · [发布流程](RELEASING.md)

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

在仓库根执行：

```sh
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
desktop/src-tauri/target/release/bundle/nsis/WordSide_0.1.0_x64-setup.exe
```

若指定 CARGO_TARGET_DIR，输出位于对应缓存目录。生成许可在 `desktop/src-tauri/generated/`，Tauri schema 在 `gen/`，均不提交源码。

默认原生 test 检查 HTTP、解析、取消、缓存、设置与纯逻辑；跳过 `capture::` / `dismissal::`。可以用 `-TestFilter capture::shortcut::` 单独检查纯快捷键解析；这项同样不操作桌面。过滤和跳过不算通过用例。真实选词、焦点、DPI、托盘与安装应另列手动验收，见[发布说明](releases/v0.1.0.md)。

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
.\release\WordSide_0.1.0.exe --wordside-dictionary-probe en-cn:terminal en:runtime
.\release\WordSide_0.1.0.exe --wordside-dictionary-parse C:\temp\sample-page.html
.\release\WordSide_0.1.0.exe --wordside-dictionary-licenses
```

探针不使用桌面缓存；后端计时不等于从选词到弹窗的完整耗时。在线查询不作为 CI 必需测试。Release 文件需先自行下载或按发布步骤生成。

## GitHub CI

`.github/workflows/ci.yml` 在 main push、目标为 main 的 PR 与手动触发时执行：

- Linux：锁文件安装、JS 测试、TypeScript 与 UI 构建。
- Windows：同样检查，另外执行原生逻辑与纯快捷键解析测试。

CI 不启动用户桌面、不安装应用、不自动发布 Release。最终以 GitHub Actions 的实际 run 结果为准，不用本地检查代替远端成功。

## 界面素材

截图使用当前 UI 和示例数据，说明与维护方法见 [SCREENSHOTS.md](SCREENSHOTS.md)。更新样式后再更新相关图片，避免文档与实际产品漂移。
