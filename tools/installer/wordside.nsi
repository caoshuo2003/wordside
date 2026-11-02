; WordSide installer: one exe for the current user, no administrator rights.
; Built by tools/build-installer.ps1; see docs/RELEASING.md.
;
; It uses 0.2.0's install folder and uninstall entry, so an upgrade replaces 0.2.0 in
; place: 0.2.0's own uninstaller runs silently first (it keeps settings unless asked).
; Settings live in %APPDATA%\local.wordside.dictionary.api-preview and are never touched.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "makensis /DVERSION=x.y.z /DEXE=... /DLICENSES=... /DICON=... /DOUT=..."
!endif

!define PRODUCT "WordSide"
!define DISPLAY "WordSide · 词伴"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\WordSide"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Name "${DISPLAY} ${VERSION}"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\WordSide"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ManifestDPIAware true
BrandingText "${DISPLAY} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "WordSide"
VIAddVersionKey /LANG=2052 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=2052 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=2052 "FileDescription" "WordSide ${VERSION} 安装程序"
VIAddVersionKey /LANG=2052 "LegalCopyright" "WordSide"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "WordSide 是一个安静、轻巧的划词词典。$\r$\n$\r$\n安装只放到当前用户，不需要管理员权限。已经安装的 0.2.0 会先被替换，原来的设置都会保留。"
!define MUI_FINISHPAGE_TEXT "WordSide 运行后在任务栏右下角的托盘待命，不会弹出窗口。$\r$\n$\r$\n左键点托盘图标可以输入单词查询；选中单词后按 Ctrl+Alt+A 查词；托盘右键可开启“自动划词”。"
!define MUI_FINISHPAGE_RUN "$INSTDIR\WordSide.exe"
!define MUI_FINISHPAGE_RUN_TEXT "现在运行 WordSide"
; The second finish-page checkbox, borrowed for starting with Windows.
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "开机时自动启动"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION StartWithWindows

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

; Any WordSide started from the install folder (0.2.0 or this one) holds its files open.
!macro CloseWordSide
  nsExec::Exec 'taskkill /F /IM WordSide.exe'
  Pop $0
  Sleep 300
!macroend

Function StartWithWindows
  WriteRegStr HKCU "${RUN_KEY}" "WordSide" '"$INSTDIR\WordSide.exe"'
FunctionEnd

; 0.2.0 (Tauri) registered the same uninstall entry. Run its uninstaller quietly and
; wait, so its WebView files go before ours arrive. Our own entry is just overwritten.
Function RemoveOldVersion
  ReadRegStr $0 HKCU "${UNINSTALL_KEY}" "DisplayVersion"
  ReadRegStr $1 HKCU "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $0 == ""
  ${OrIf} $1 == ""
    Return
  ${EndIf}
  StrCpy $2 $0 2
  ${If} $2 != "0."
    Return
  ${EndIf}
  StrCpy $2 $0 3
  ${If} $2 != "0.1"
  ${AndIf} $2 != "0.2"
    Return
  ${EndIf}
  ; Tauri writes the location with quotes; strip them.
  StrCpy $3 $1 1
  ${If} $3 == '"'
    StrCpy $1 $1 -1 1
  ${EndIf}
  ${If} ${FileExists} "$1\uninstall.exe"
    DetailPrint "正在移除旧版 WordSide $0…"
    ExecWait '"$1\uninstall.exe" /S _?=$1' $3
    Delete "$1\uninstall.exe"
    RMDir "$1"
  ${EndIf}
FunctionEnd

Section "WordSide"
  !insertmacro CloseWordSide
  Call RemoveOldVersion
  SetOutPath "$INSTDIR"
  File "/oname=WordSide.exe" "${EXE}"
  File "/oname=THIRD-PARTY-LICENSES.txt" "${LICENSES}"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\WordSide.lnk" "$INSTDIR\WordSide.exe" "" "$INSTDIR\WordSide.exe" 0

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${DISPLAY}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "WordSide"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/caoshuo2003/wordside"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" '"$INSTDIR\WordSide.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" 4600
SectionEnd

Section "Uninstall"
  !insertmacro CloseWordSide
  Delete "$INSTDIR\WordSide.exe"
  Delete "$INSTDIR\THIRD-PARTY-LICENSES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\WordSide.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "WordSide"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
