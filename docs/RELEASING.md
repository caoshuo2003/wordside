# 发布流程

首次 GitHub 正式版本为 **v0.1.0**。源码与成品分开：Git 保存可重建代码，GitHub Release 保存程序、安装包、便携包和许可。

## 0.3.0 起：原生程序 apps/wordside

0.3.0 起发布的是 `apps/wordside` 的单个 exe 和它的安装版。步骤：

1. 改 `apps/wordside/Cargo.toml` 与 `Cargo.lock` 中 `wordside` 的版本（exe 文件属性里的版本由 `build.rs` 从这里取），更新 CHANGELOG、用户文档和 `docs/releases/v<版本>.md`。
2. 按 [开发指南](DEVELOPMENT.md#030-原生卡片appswordside) 生成第三方许可、正式编译，并运行 `check-wordside.ps1`。
3. 用 `tools/build-installer.ps1 -Exe <exe> -Licenses <许可> -OutDir release/v<版本>` 生成安装版（需要 NSIS 的 `makensis`，例如 `scoop install extras/nsis`）。安装版只装给当前用户，沿用 0.2.0 的安装目录和卸载项，会先静默卸载已安装的 0.2.0。
4. 成品放进忽略的 `release/v<版本>/`：`WordSide_<版本>_x64-setup.exe`、`WordSide_<版本>.exe`、`THIRD-PARTY-LICENSES_<版本>.txt`，以及包含 exe、许可和使用说明的 `WordSide_<版本>-portable.zip`。
5. 合入、打标签、发布同下面第 3、4 节，附件换成上面四个文件。

0.3.0 是一次例外：同一天先发布了一版，随后修好剪贴板取词、1080p 文字和安装版，维护者决定把修好的这一版作为改版后第一个可用的 0.3.0，把 `v0.3.0` 标签移到新的提交并替换了附件。以后照常：已发布的标签不移动。

下面第 1、2 节是 0.2.0（`desktop/`，Tauri）的流程，退役前保留。

## 1 · 准备一个版本

从最新 main 建 `chore/release-<版本>` 分支，完成目标改动并更新：

- 根 / desktop package.json 与根 package-lock.json 的应用版本。
- Cargo.toml、Cargo.lock 中 `wordside` 的版本。
- tauri.conf.json、CHANGELOG、用户教程和 `docs/releases/v<版本>.md`。

独立 Cambridge 模块有自己的版本，不跟随应用盲目改号。

## 2 · 检查和构建

复用仍覆盖当前代码的证据，补查受影响部分；不为文档修改重复构建未变产品。

```powershell
.\tools\windows-cargo.ps1 -Action test
.\tools\windows-cargo.ps1 -Action bundle
```

把成品整理到忽略的 `release/v<版本>/`，保留旧版：

```text
WordSide_<版本>.exe
WordSide_<版本>_x64-setup.exe
WordSide_<版本>-portable.zip
THIRD-PARTY-LICENSES_<版本>.txt
```

便携包包含主程序、同版许可和使用说明。用 `tools/check-release.ps1 -ReleaseDirectory release/v<版本>` 检查版本与大小，核对 ZIP 清单；发布说明用简短列表说明本版修复与新增，必要时补充更新方式或已知限制；详细验证范围记录在 PR 或本地任务记录。

## 3 · 合入与打标签

发布 PR 合入 main 后，确认工作树干净并创建 annotated tag。发布 PR 如果只改版本号和文档，代码已在之前的 PR 上通过检查，不用等它的 CI；本地的 `cargo test` 和 `check-wordside.ps1 -Runtime` 照常要跑。

```sh
git switch main
git pull --ff-only
git tag -a v0.2.0 -m "WordSide 0.2.0"
git push origin v0.2.0
```

以上以 v0.2.0 为例；已有标签不覆盖、不移动，也不推送本机所有历史标签。

## 4 · 发布并核对

GitHub CLI 已登录且具有仓库发布权限时，可运行：

```sh
gh release create v0.2.0 --repo caoshuo2003/wordside \
  --verify-tag --latest \
  --title "WordSide · 词伴 0.2.0" --notes-file docs/releases/v0.2.0.md \
  release/v0.2.0/WordSide_0.2.0.exe release/v0.2.0/WordSide_0.2.0_x64-setup.exe \
  release/v0.2.0/WordSide_0.2.0-portable.zip release/v0.2.0/THIRD-PARTY-LICENSES_0.2.0.txt
```

发布前确认目标仓库、版本标签和 Release 附件一致，标签指向的代码已在 PR 上通过 CI；发布后把附件下载回来，和本地成品比对 SHA256。正式版使用 `--latest` 作为默认下载版本；提前试用的版本使用 `--prerelease --latest=false`。发布说明面向使用者，不重复粘贴构建日志与测试清单。

已有预发布版转正式版时，若程序和版本号保持一致，可沿用同一标签与附件：

```sh
gh release edit v0.1.0 --repo caoshuo2003/wordside \
  --prerelease=false --latest --notes-file docs/releases/v0.1.0.md
```

README 负责介绍和下载，使用指南负责操作，FAQ 负责排查，Release 说明本版变化与已知限制，开发指南负责环境和构建。只更新受本次改动影响的内容，保持链接和版本一致。
