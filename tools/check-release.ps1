$ErrorActionPreference='Stop'
$project=Split-Path $PSScriptRoot -Parent
$release=Join-Path $project 'release'
$version=(Get-Content (Join-Path $project 'desktop\package.json') -Raw | ConvertFrom-Json).version
$config=Get-Content (Join-Path $project 'desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
if($config.identifier -ne 'local.wordside.dictionary.api-preview' -or $config.productName -ne 'WordSide'){throw 'WordSide identity mismatch'}
@("WordSide_${version}.exe","WordSide_${version}_x64-setup.exe") | ForEach-Object {
 $file=Get-Item (Join-Path $release $_)
 if($file.Length -le 0){throw 'Empty WordSide release file'}
 if($file.VersionInfo.ProductName -ne $config.productName){throw "WordSide product name mismatch: $($file.VersionInfo.ProductName)"}
 if(!$file.VersionInfo.ProductVersion.StartsWith($version)){throw "WordSide version mismatch: $($file.VersionInfo.ProductVersion)"}
 [pscustomobject]@{name=$file.Name;bytes=$file.Length;version=$file.VersionInfo.ProductVersion;identifier=$config.identifier;product=$file.VersionInfo.ProductName}
} | ConvertTo-Json
