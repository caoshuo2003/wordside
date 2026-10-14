# 发布流程

首次 GitHub 版本为 **v0.1.0**。源码与成品分开：Git 保存可重建代码，GitHub Release 保存程序、安装包、便携包和许可。

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

便携包包含主程序、同版许可、使用和验证说明；不会打包 Node、用户偏好或词条缓存。用 `tools/check-release.ps1` 检查大小和版本，核对 ZIP 清单。真实桌面与安装未测时明确写出，不把构建成功写成安装通过。

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

GitHub CLI 已登录且对私有仓库有权限时，可运行：

```sh
gh release create v0.1.1 --repo caoshuo2003/wordside --verify-tag --prerelease   --title "WordSide 0.1.1" --notes-file docs/releases/v0.1.1.md   release/WordSide_0.1.1.exe release/WordSide_0.1.1_x64-setup.exe   release/WordSide_0.1.1-portable.zip release/THIRD-PARTY-LICENSES_0.1.1.txt
```

核对私有性、main/tag 对应提交、Release 附件与 CI 实际结果。Windows 未签名安装包和真实安装验收状态应如实说明；不要自动启用发布流水线或更改仓库可见性。
