# 发布流程

源码与成品分开：Git 保存可重建代码，GitHub Release 保存程序、安装包、便携包和许可。发布的是 `apps/wordside` 的单个 exe 和它的安装版。

## 1 · 准备

从最新 main 建 `chore/release-<版本>` 分支：

1. 改 `apps/wordside/Cargo.toml` 与 `Cargo.lock` 中 `wordside` 的版本（exe 文件属性里的版本由 `build.rs` 从这里取）。
2. CHANGELOG 的“未发布”改成 `<版本> — <日期>`，新增 `docs/releases/v<版本>.md`，使用指南和常见问题里的下载链接、文件名改成新版本。

修复用 0.3.x，新功能用 0.4.0；只改文档不改应用版本。独立 Cambridge 模块有自己的版本，不跟随应用改号。

## 2 · 构建

按[开发指南](DEVELOPMENT.md#检查与构建)运行测试、生成第三方许可、正式编译，并运行 `check-wordside.ps1 -Runtime`。成品放进忽略的 `release/v<版本>/`，保留旧版：

```text
WordSide_<版本>.exe
WordSide_<版本>_x64-setup.exe          tools/build-installer.ps1 -Exe <exe> -Licenses <许可> -OutDir release/v<版本>
WordSide_<版本>-portable.zip           exe、许可和“使用说明.txt”
THIRD-PARTY-LICENSES_<版本>.txt
```

安装版需要 NSIS 的 `makensis`（例如 `scoop install extras/nsis`）。它只装给当前用户，沿用 0.2.0 的安装目录和卸载项，会先静默卸载已安装的 0.2.0。核对 exe 的文件属性显示新版本。

## 3 · 合入与打标签

提交后推送分支、开 PR。发布 PR 如果只改版本号和文档，代码已在之前的 PR 上通过检查，不用等它的 CI。合入后：

```sh
git switch main
git pull --ff-only
git tag -a v0.3.1 -m "WordSide 0.3.1"
git push origin v0.3.1
```

以上以 v0.3.1 为例。已发布的标签不覆盖、不移动，也不推送本机所有历史标签（0.3.0 曾因当天重新发布移动过一次，是唯一的例外）。

## 4 · 发布并核对

```sh
gh release create v0.3.1 --repo caoshuo2003/wordside \
  --verify-tag --latest \
  --title "WordSide · 词伴 0.3.1" --notes-file docs/releases/v0.3.1.md \
  release/v0.3.1/WordSide_0.3.1_x64-setup.exe release/v0.3.1/WordSide_0.3.1.exe \
  release/v0.3.1/WordSide_0.3.1-portable.zip release/v0.3.1/THIRD-PARTY-LICENSES_0.3.1.txt
```

发布前确认目标仓库、版本标签和 Release 附件一致，标签指向的代码已在 PR 上通过 CI；发布后把附件下载回来，和本地成品比对 SHA256。正式版使用 `--latest`；提前试用的版本使用 `--prerelease --latest=false`，转正式版时若程序和版本号不变，用 `gh release edit v<版本> --prerelease=false --latest` 沿用同一标签与附件。发布说明面向使用者，不重复粘贴构建日志与测试清单。

README 负责介绍和下载，使用指南负责操作，FAQ 负责排查，Release 说明本版变化与已知限制，开发指南负责环境和构建。只更新受本次改动影响的内容，保持链接和版本一致。
