param(
    [ValidateSet('check','test','build','release','bundle','probe')][string]$Action='check',
    [string]$DevToolsRoot=$env:WORDSIDE_DEVTOOLS,
    [string]$TargetDir='',
    [string]$NodeExecutable='node',
    [string]$ProbeWords='terminal,terminal,terminal',
    [string]$ProbeOutput='',
    [ValidateSet('en-cn','en')][string]$ProbeLanguage='en-cn',
    [string]$TestFilter='',
    [switch]$ProbeWorkflow
)
$ErrorActionPreference='Stop'
$project=Split-Path $PSScriptRoot -Parent
# The default uses the standard Rust/MSVC installation. An optional DevTools
# profile retains this workstation's existing D-drive toolchain and caches.
if ($DevToolsRoot) {
    $DevToolsRoot=(Resolve-Path $DevToolsRoot).Path
    $env:CARGO_HOME=Join-Path $DevToolsRoot 'rust\cargo'
    $env:RUSTUP_HOME=Join-Path $DevToolsRoot 'rust\rustup'
    $env:PATH="$env:CARGO_HOME\bin;$env:PATH"
    if (!$TargetDir) { $TargetDir=Join-Path $DevToolsRoot 'wordside-api-target' }
    $env:TAURI_BUNDLER_CACHE_DIR=Join-Path $DevToolsRoot 'tauri-cache'
    $env:TEMP=Join-Path $DevToolsRoot 'temp'
    $env:TMP=$env:TEMP
    New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
}
if ($TargetDir) { $env:CARGO_TARGET_DIR=$TargetDir }
if (!$env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS='4' }
$node=(Get-Command $NodeExecutable -ErrorAction Stop).Source
$cargo=(Get-Command cargo -ErrorAction Stop).Source
$vc=$null
if (!$env:VCToolsInstallDir) {
    if ($DevToolsRoot) {
        $candidate=Join-Path $DevToolsRoot 'VSBuildTools\VC\Auxiliary\Build\vcvars64.bat'
        if (Test-Path $candidate) { $vc=$candidate }
    }
    if (!$vc) {
        $vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        if (Test-Path $vswhere) {
            $found=& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
            if ($LASTEXITCODE -ne 0) { throw 'Unable to locate Visual Studio C++ tools.' }
            if ($found) { $vc=Join-Path $found 'VC\Auxiliary\Build\vcvars64.bat' }
        }
    }
    if (!$vc -or !(Test-Path $vc)) { throw 'Install Visual Studio C++ Build Tools + Windows SDK, or run from an x64 Developer PowerShell.' }
    $lines=& cmd.exe /d /c "`"$vc`" >nul && set"
    if ($LASTEXITCODE -ne 0) { throw 'MSVC environment setup failed.' }
    foreach ($line in $lines) {
        if ($line -match '^([^=]+)=(.*)$') {
            [Environment]::SetEnvironmentVariable($matches[1],$matches[2],'Process')
        }
    }
}
& $node "$PSScriptRoot\build-licenses.mjs"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $node "$PSScriptRoot\build-ui.mjs"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Push-Location "$project\desktop\src-tauri"
try {
    switch ($Action) {
        'bundle' { & "$project\node_modules\.bin\tauri.cmd" build --bundles nsis }
        'probe' {
            $env:WORDSIDE_PROBE_WORKFLOW=if ($ProbeWorkflow) {'1'} else {'0'}
            $env:WORDSIDE_PROBE_LANGUAGE=$ProbeLanguage
            if ($ProbeOutput) { $env:WORDSIDE_PROBE_DIR=$ProbeOutput }
            & $cargo run --locked --release --example connection_probe -- $ProbeWords.Split(',')
        }
        'release' { & $cargo build --locked --release }
        'test' {
            if ($TestFilter) { & $cargo test --locked --release $TestFilter -- --nocapture }
            else { & $cargo test --locked --release -- --skip capture:: --skip dismissal:: }
        }
        default { & $cargo $Action --locked }
    }
    $code=$LASTEXITCODE
} finally { Pop-Location }
exit $code
