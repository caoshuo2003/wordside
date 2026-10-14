param([int]$Seconds=12,[int]$Interval=3)
$ErrorActionPreference='Stop'
$all=Get-CimInstance -Query 'SELECT ProcessId,ParentProcessId,Name,ExecutablePath FROM Win32_Process'
$matching=@($all | Where-Object {
 if($_.Name -notmatch '^WordSide.*\.exe$' -or !$_.ExecutablePath){return $false}
 try {return [System.Diagnostics.FileVersionInfo]::GetVersionInfo($_.ExecutablePath).ProductName -eq 'WordSide'} catch {return $false}
})
$matchedIds=@($matching.ProcessId)
$roots=@($matching | Where-Object {$_.ParentProcessId -notin $matchedIds})
if(!$roots.Count){[pscustomobject]@{status='not_running';product='WordSide';note='No identified WordSide process; no application was started or controlled.'} | ConvertTo-Json;exit 0}
$logical=(Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
$observations=@();$previous=@{};$previousTime=$null;$watch=[Diagnostics.Stopwatch]::StartNew()
do {
 $processes=Get-CimInstance -Query 'SELECT ProcessId,ParentProcessId,Name FROM Win32_Process'
 $ids=[System.Collections.Generic.HashSet[int]]::new();foreach($root in $roots){[void]$ids.Add([int]$root.ProcessId)}
 do {$added=$false;foreach($process in $processes){if($ids.Contains([int]$process.ParentProcessId) -and $ids.Add([int]$process.ProcessId)){$added=$true}}} while($added)
 $time=$watch.Elapsed.TotalSeconds;$current=@{};$working=[long]0;$private=[long]0;$cpuDelta=0.0;$names=@();$nativePrivate=[long]0;$webviewPrivate=[long]0
 foreach($processId in $ids){try {$process=Get-Process -Id $processId -ErrorAction Stop;$cpu=$process.TotalProcessorTime.TotalSeconds;$current[$processId]=$cpu;$working+=$process.WorkingSet64;$private+=$process.PrivateMemorySize64;$names+=$process.ProcessName;if($process.ProcessName -like 'msedgewebview2*'){$webviewPrivate+=$process.PrivateMemorySize64}else{$nativePrivate+=$process.PrivateMemorySize64};if($previous.ContainsKey($processId)){$cpuDelta+=[Math]::Max(0,$cpu-$previous[$processId])}} catch {}}
 $core=if($null -ne $previousTime){100*$cpuDelta/($time-$previousTime)}else{$null}
 $observations += [pscustomobject]@{elapsedSeconds=[Math]::Round($time,3);processes=$current.Count;processNames=@($names | Sort-Object -Unique);workingSetMiB=[Math]::Round($working/1MB,2);privateCommitMiB=[Math]::Round($private/1MB,2);nativePrivateCommitMiB=[Math]::Round($nativePrivate/1MB,2);webviewPrivateCommitMiB=[Math]::Round($webviewPrivate/1MB,2);cpuPercentOneCore=if($null -ne $core){[Math]::Round($core,3)}else{$null};cpuPercentMachine=if($null -ne $core){[Math]::Round($core/$logical,3)}else{$null}}
 $previous=$current;$previousTime=$time
 if($watch.Elapsed.TotalSeconds -ge $Seconds){break};Start-Sleep -Seconds $Interval
} while($true)
[pscustomobject]@{status='sampled';mode='Read-only existing WordSide and descendant processes; activity uncontrolled; working set may count shared pages more than once';logicalProcessors=$logical;roots=@($roots | ForEach-Object {[pscustomobject]@{id=$_.ProcessId;name=$_.Name;version=[Diagnostics.FileVersionInfo]::GetVersionInfo($_.ExecutablePath).ProductVersion}});observations=$observations} | ConvertTo-Json -Depth 7
