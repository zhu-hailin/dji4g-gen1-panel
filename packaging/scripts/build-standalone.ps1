[CmdletBinding()]
param([Parameter(Mandatory)][string]$PayloadDirectory, [Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference='Stop'
$repoRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$payload=(Resolve-Path -LiteralPath $PayloadDirectory).Path
if (Test-Path -LiteralPath $Destination) { throw 'Output exists; choose a new file name.' }
$manifest=Get-Content -LiteralPath (Join-Path $payload 'portable-manifest.json') -Raw | ConvertFrom-Json
foreach ($entry in $manifest) {
 if ((Get-FileHash -LiteralPath (Join-Path $payload $entry.name)).Hash -ne $entry.sha256) { throw "Payload changed: $($entry.name)" }
}
foreach ($name in @('dji4g-panel.exe','dji4g-helper.exe','dji4g-driver-setup.exe','drivers/qcser.inf')) {
 if (!(Test-Path -LiteralPath (Join-Path $payload $name) -PathType Leaf)) { throw "Missing: $name" }
}
$previousDir=$env:DJI4G_STANDALONE_DIR
$previousId=$env:DJI4G_STANDALONE_ID
try {
 $env:DJI4G_STANDALONE_DIR=$payload
 $env:DJI4G_STANDALONE_ID=(Get-FileHash -LiteralPath (Join-Path $payload 'portable-manifest.json')).Hash
 Push-Location $repoRoot
 try {
  $destinationPath=[IO.Path]::GetFullPath($Destination)
  & cargo rustc -p dji4g-panel --bin dji4g-portable --release --offline -- -C target-feature=+crt-static "--emit=link=$destinationPath"
  if ($LASTEXITCODE -ne 0) { throw 'Standalone build failed' }
 } finally { Pop-Location }
 [pscustomobject]@{File=[IO.Path]::GetFullPath($Destination);Sha256=(Get-FileHash -LiteralPath $Destination).Hash;Bytes=(Get-Item -LiteralPath $Destination).Length} | ConvertTo-Json
} finally {
 $env:DJI4G_STANDALONE_DIR=$previousDir
 $env:DJI4G_STANDALONE_ID=$previousId
}
