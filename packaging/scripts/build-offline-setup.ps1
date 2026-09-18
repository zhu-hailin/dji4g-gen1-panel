[CmdletBinding()]
param([Parameter(Mandatory)][string]$PayloadZip, [Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference='Stop'
$repoRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$payload=(Resolve-Path -LiteralPath $PayloadZip).Path
$output=[IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $output -Force | Out-Null
$destination=Join-Path $output '大疆4G面板完整安装包.exe'
if (Test-Path -LiteralPath $destination) { throw 'Output already exists; choose a fresh directory' }
$previousZip=$env:DJI4G_OFFLINE_ZIP
$previousHash=$env:DJI4G_OFFLINE_SHA256
try {
 $env:DJI4G_OFFLINE_ZIP=$payload
 $env:DJI4G_OFFLINE_SHA256=(Get-FileHash -LiteralPath $payload).Hash
 Push-Location $repoRoot
 try { & cargo build -p dji4g-panel --bin dji4g-setup --release --offline; if ($LASTEXITCODE -ne 0) { throw 'Setup build failed' } } finally { Pop-Location }
 Copy-Item -LiteralPath (Join-Path $repoRoot 'target/x86_64-pc-windows-msvc/release/dji4g-setup.exe') -Destination $destination
 [pscustomobject]@{file=$destination;sha256=(Get-FileHash -LiteralPath $destination).Hash;payloadSha256=$env:DJI4G_OFFLINE_SHA256} | ConvertTo-Json | Tee-Object -FilePath (Join-Path $output 'setup-manifest.json')
} finally { $env:DJI4G_OFFLINE_ZIP=$previousZip; $env:DJI4G_OFFLINE_SHA256=$previousHash }
