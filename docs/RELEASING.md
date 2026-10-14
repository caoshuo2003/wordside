# 发布流程

首次 GitHub 正式版本为 **v0.1.0**。源码与成品分开：Git 保存可重建代码，GitHub Release 保存程序、安装包、便携包和许可。

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

把成品整理到忽略的 `release/`，保留旧版：

```text
WordSide_<版本>.exe
WordSide_<版本>_x64-setup.exe
WordSide_<版本>-portable.zip
THIRD-PARTY-LICENSES_<版本>.txt
```

便携包包含主程序、同版许可和使用说明。用 `tools/check-release.ps1` 检查版本与大小，核对 ZIP 清单；发布说明应写清用户可用的功能、运行要求、已知限制和实际验证范围。

## 3 · 合入与打标签

PR 的实际 CI 通过后合入 main，确认工作树干净并创建 annotated tag：

```sh
git switch main
git pull --ff-only
git tag -a v0.1.1 -m "WordSide 0.1.1"
git push origin v0.1.1
```

例子使用下一次修复版本；已有标签不覆盖、不移动，也不推送本机所有历史标签。

## 4 · 发布并核对

GitHub CLI 已登录且具有仓库发布权限时，可运行：

```sh
gh release create v0.1.1 --repo caoshuo2003/wordside \
  --verify-tag --latest \
  --title "WordSide · 词伴 0.1.1" --notes-file docs/releases/v0.1.1.md \
  release/WordSide_0.1.1.exe release/WordSide_0.1.1_x64-setup.exe \
  release/WordSide_0.1.1-portable.zip release/THIRD-PARTY-LICENSES_0.1.1.txt
```

发布前确认目标仓库、版本标签、Release 附件和实际 CI 结果一致。正式版使用 `--latest` 作为默认下载版本；提前试用的版本使用 `--prerelease --latest=false`。发布说明如实记录已完成的检查与已知限制。

已有预发布版转正式版时，若程序和版本号保持一致，可沿用同一标签与附件：

```sh
gh release edit v0.1.0 --repo caoshuo2003/wordside \
  --prerelease=false --latest --notes-file docs/releases/v0.1.0.md
```

README 负责介绍和下载，使用指南负责操作，FAQ 负责排查，Release 说明本版变化与已知限制，开发指南负责环境和构建。只更新受本次改动影响的内容，保持链接和版本一致。
