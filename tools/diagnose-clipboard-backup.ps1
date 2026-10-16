param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [ValidateRange(0,60)][int]$WaitForOleSeconds=0
)
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path $Executable).Path
if($WaitForOleSeconds -gt 0){
    # Metadata-only polling avoids losing the failing state while switching apps.
    # GetClipboardData is not called by this observer; backup runs at most once.
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WordSideOleFormatObserver {
    [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr owner);
    [DllImport("user32.dll")] static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern uint EnumClipboardFormats(uint format);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClipboardFormatNameW(uint format, System.Text.StringBuilder name, int count);
    public static bool HasOle() {
        if(!OpenClipboard(IntPtr.Zero)) return false;
        try {
            uint id=0; int count=0;
            while((id=EnumClipboardFormats(id))!=0 && count++<128) {
                if(id<0xc000) continue;
                var name=new System.Text.StringBuilder(256);
                GetClipboardFormatNameW(id,name,256);
                if(name.ToString().Equals("DataObject",StringComparison.OrdinalIgnoreCase) || name.ToString().Equals("Ole Private Data",StringComparison.OrdinalIgnoreCase)) return true;
            }
            return false;
        } finally {CloseClipboard();}
    }
}
'@
    $deadline=[DateTime]::UtcNow.AddSeconds($WaitForOleSeconds)
    while(-not [WordSideOleFormatObserver]::HasOle()){
        if([DateTime]::UtcNow -ge $deadline){
            [pscustomobject]@{mode='metadata-only wait';prepared=$false;stage='ole_not_seen';waitSeconds=$WaitForOleSeconds} | ConvertTo-Json
            return
        }
        Start-Sleep -Milliseconds 100
    }
}
# Runs only WordSide's internal backup branch, before Tauri or any visible UI.
# Backup reads and duplicates all supported formats into private process memory.
# It never requests Restore/Clipboard, never sends keys, and never clears/sets data.
# Output is limited to status, elapsed time and native failure stage/format/error.
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
$elapsed=[System.Diagnostics.Stopwatch]::StartNew()
try{
    $started=$process.Start()
    if(!$started){throw 'Internal helper failed to start'}
    $request=@{serial=1;text=@{pid=0;window=0;focus=0;origin='Backup'}} | ConvertTo-Json -Compress
    $process.StandardInput.WriteLine($request)
    $process.StandardInput.Flush()
    $reading=$process.StandardOutput.ReadLineAsync()
    if(!$reading.Wait(8000)){
        [pscustomobject]@{mode='read-only backup; no clipboard writes or input';prepared=$false;stage='helper_timeout';elapsedMs=$elapsed.ElapsedMilliseconds} | ConvertTo-Json
        return
    }
    $line=$reading.Result
    $prefix='WORDSIDEPROBE '
    if(!$line -or !$line.StartsWith($prefix)){throw 'Missing helper protocol reply'}
    $reply=$line.Substring($prefix.Length) | ConvertFrom-Json
    if($reply.serial -ne 1){throw 'Unexpected helper serial'}
    $summary=[ordered]@{mode='read-only backup; no clipboard writes or input';prepared=($null -ne $reply.clipboard_sequence);elapsedMs=$elapsed.ElapsedMilliseconds}
    if($null -ne $reply.clipboard_ole_formats){$summary.olePayloadFormats=$reply.clipboard_ole_formats}
    if($reply.clipboard_failure){$summary.failure=$reply.clipboard_failure}
    elseif($null -eq $reply.clipboard_sequence){$summary.stage=$reply.selection.kind}
    $process.StandardInput.Close()
    if(!$process.WaitForExit(2000)){throw 'Helper did not exit after its private pipe closed'}
    [pscustomobject]$summary | ConvertTo-Json -Depth 4
}finally{
    if($started -and !$process.HasExited){$process.Kill();[void]$process.WaitForExit(2000)}
    $process.Dispose()
}
