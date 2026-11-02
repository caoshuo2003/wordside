# Builds WordSide_<version>_x64-setup.exe from a release exe with NSIS (makensis on PATH,
# e.g. `scoop install extras/nsis`). The version comes from apps/wordside/Cargo.toml.
param(
    [Parameter(Mandatory = $true)][string] $Exe,
    [Parameter(Mandatory = $true)][string] $Licenses,
    [Parameter(Mandatory = $true)][string] $OutDir
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$version = (Select-String -Path (Join-Path $project 'apps\wordside\Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$makensis = (Get-Command makensis -ErrorAction Stop).Source
New-Item -ItemType Directory -Force $OutDir | Out-Null
$out = Join-Path (Resolve-Path $OutDir) "WordSide_${version}_x64-setup.exe"
& $makensis /V2 /INPUTCHARSET UTF8 `
    "/DVERSION=$version" `
    "/DEXE=$((Resolve-Path $Exe).Path)" `
    "/DLICENSES=$((Resolve-Path $Licenses).Path)" `
    "/DICON=$(Join-Path $project 'apps\wordside\assets\wordside.ico')" `
    "/DOUT=$out" `
    (Join-Path $PSScriptRoot 'installer\wordside.nsi')
if ($LASTEXITCODE -ne 0) { throw "makensis failed ($LASTEXITCODE)" }
Get-Item $out
