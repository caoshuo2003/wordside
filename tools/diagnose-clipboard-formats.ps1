# Read-only clipboard metadata. Never reads clipboard data, injects input or creates windows.
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class WordSideClipboardFormatDiagnostic {
    [DllImport("user32.dll", SetLastError=true)] public static extern bool OpenClipboard(IntPtr window);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool CloseClipboard();
    [DllImport("user32.dll", SetLastError=true)] public static extern uint EnumClipboardFormats(uint format);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern int GetClipboardFormatNameW(uint format, StringBuilder name, int capacity);
}
'@
$opened = $false
try {
    for ($attempt = 0; $attempt -lt 10; $attempt++) {
        if ([WordSideClipboardFormatDiagnostic]::OpenClipboard([IntPtr]::Zero)) { $opened = $true; break }
        Start-Sleep -Milliseconds 8
    }
    if (!$opened) { throw "Clipboard busy; error $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
    $formats = [Collections.Generic.List[object]]::new()
    [uint32]$format = 0
    while ($true) {
        $format = [WordSideClipboardFormatDiagnostic]::EnumClipboardFormats($format)
        if ($format -eq 0) {
            $nativeError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            if ($nativeError -ne 0) { throw "Format enumeration error $nativeError" }
            break
        }
        if ($formats.Count -ge 128) { throw 'Format count limit exceeded' }
        $name = [Text.StringBuilder]::new(256)
        $length = [WordSideClipboardFormatDiagnostic]::GetClipboardFormatNameW($format, $name, $name.Capacity)
        $formats.Add([pscustomobject]@{ id=$format; name=if ($length -gt 0) {$name.ToString()} else {'standard'} })
    }
    [pscustomobject]@{ readOnly=$true; contentsRead=$false; formats=$formats.ToArray() } | ConvertTo-Json -Depth 4
} finally {
    if ($opened) { [void][WordSideClipboardFormatDiagnostic]::CloseClipboard() }
}
