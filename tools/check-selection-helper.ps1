param([string]$Executable='')
$ErrorActionPreference='Stop'
if(!$Executable){
    $project=Split-Path $PSScriptRoot -Parent
    $version=(Get-Content (Join-Path $project 'desktop\package.json') -Raw | ConvertFrom-Json).version
    $Executable=Join-Path $project "release\WordSide_${version}.exe"
}
$Executable=(Resolve-Path $Executable).Path
# This internal entry returns before Tauri, the tray and single-instance setup.
$start=[System.Diagnostics.ProcessStartInfo]::new()
$start.FileName=$Executable
$start.Arguments='--wordside-selection-helper'
$start.UseShellExecute=$false
$start.CreateNoWindow=$true
$start.RedirectStandardInput=$true
$start.RedirectStandardOutput=$true
$start.RedirectStandardError=$true
$process=[System.Diagnostics.Process]::new()
$process.StartInfo=$start
$started=$false
try{
    $started=$process.Start()
    if(!$started){throw 'Internal helper failed to start'}
    foreach($serial in @(1,2)){
        # Zero source is deliberately refused, with no user window or focus access.
        $request=@{serial=$serial;text=@{pid=0;window=0;focus=0;origin='Focused'}} | ConvertTo-Json -Compress
        $process.StandardInput.WriteLine($request)
        $process.StandardInput.Flush()
        $reading=$process.StandardOutput.ReadLineAsync()
        if(!$reading.Wait(4000)){throw 'Helper protocol timed out'}
        $line=$reading.Result
        $prefix='WORDSIDEPROBE '
        if(!$line -or !$line.StartsWith($prefix)){throw 'Missing helper protocol reply'}
        $reply=$line.Substring($prefix.Length) | ConvertFrom-Json
        if($reply.serial -ne $serial -or $reply.selection.kind -ne 'source_mismatch'){throw 'Invalid helper refusal or serial'}
    }
    $process.Refresh()
    if($process.MainWindowHandle -ne [IntPtr]::Zero){throw 'Internal helper must have no visible main window'}
    $process.StandardInput.Close()
    if(!$process.WaitForExit(4000)){throw 'Helper did not exit after its private pipe closed'}
    if($process.ExitCode -ne 0){throw 'Helper exit failed'}
    [pscustomobject]@{status='passed';protocolReplies=2;source='deliberately invalid, no user selection';visibleMainWindow=$false;cleanExit=$true;mode='internal helper only; product GUI not started'} | ConvertTo-Json
}finally{
    if($started -and !$process.HasExited){$process.Kill();[void]$process.WaitForExit(4000)}
    $process.Dispose()
}
