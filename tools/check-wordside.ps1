# Checks a built apps/wordside exe against the release rules in docs/PRODUCT-DEVELOPMENT.md 3.1.
# Always: size, the real third-party notices, the diagnostic entries.
# With -Runtime (needs a desktop session): memory and idle CPU while it waits in the tray,
# using a temporary settings folder so the user's settings are never touched.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [switch] $Runtime
)
$ErrorActionPreference = 'Stop'
$failures = @()
$Exe = (Resolve-Path $Exe).Path
$work = Join-Path ([IO.Path]::GetTempPath()) "wordside-check-$PID"
New-Item -ItemType Directory -Force $work | Out-Null

# The exe is a windowed program: wait for it and send its output to a file.
function Invoke-Entry([string[]] $Arguments, [string] $Name) {
    $out = Join-Path $work "$Name.txt"
    $quoted = $Arguments | ForEach-Object { '"' + $_ + '"' }
    Start-Process $Exe -ArgumentList $quoted -Wait -NoNewWindow -RedirectStandardOutput $out
    Get-Content $out -Raw
}

$size = (Get-Item $Exe).Length / 1MB
"size: {0:N2} MB (limit 5)" -f $size
if ($size -gt 5) { $failures += 'exe is larger than 5 MB' }

$licenses = Invoke-Entry @('--wordside-dictionary-licenses') 'licenses'
"licenses: {0:N0} bytes" -f $licenses.Length
if (-not $licenses.StartsWith('WordSide ') -or $licenses.Length -lt 10000) {
    $failures += 'third-party notices are missing; run node tools/build-licenses.mjs --crate apps/wordside before building'
}
if ($licenses -notmatch 'Nongfsq/cambridge-dictionary-api') { $failures += 'the parser authors are not credited' }

$page = Join-Path $work 'missing.html'
$parse = Invoke-Entry @('--wordside-dictionary-parse', $page) 'parse'
if ($parse -notmatch '"apiError":"无法读取对照文件"') { $failures += 'the parse entry does not answer' }

if ($Runtime) {
    $env:APPDATA = Join-Path $work 'appdata'
    New-Item -ItemType Directory -Force $env:APPDATA | Out-Null
    $process = Start-Process $Exe -PassThru
    try {
        Start-Sleep 4
        $process.Refresh()
        $workingSet = $process.WorkingSet64 / 1MB
        $private = $process.PrivateMemorySize64 / 1MB
        "waiting in the tray: {0:N1} MB working set (limit 2), {1:N1} MB private (limit 20)" -f $workingSet, $private
        if ($workingSet -gt 2) { $failures += 'working set in the tray is over 2 MB' }
        if ($private -gt 20) { $failures += 'private memory in the tray is over 20 MB' }
        $before = $process.TotalProcessorTime
        Start-Sleep 10
        $process.Refresh()
        $cpu = ($process.TotalProcessorTime - $before).TotalMilliseconds
        "idle CPU over 10 s: $cpu ms"
        if ($cpu -gt 50) { $failures += 'it uses CPU while idle' }
    } finally {
        if (-not $process.HasExited) { $process.Kill() }
    }
}

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
if ($failures) {
    $failures | ForEach-Object { Write-Error "FAIL: $_" -ErrorAction Continue }
    exit 1
}
'all checks passed'
