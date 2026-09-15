[CmdletBinding()]
param([string]$OutputDirectory = 'dist')
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$output = if ([IO.Path]::IsPathRooted($OutputDirectory)) { [IO.Path]::GetFullPath($OutputDirectory) } else { [IO.Path]::GetFullPath((Join-Path $repoRoot $OutputDirectory)) }
New-Item -ItemType Directory -Path $output -Force | Out-Null
$archive = Join-Path $output 'dji4g-panel-windows-x64-portable.zip'
if (Test-Path -LiteralPath $archive) { throw 'Portable archive already exists; use a fresh output directory.' }
$staging = Join-Path $output ('portable-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $staging | Out-Null
foreach ($name in @('dji4g-panel.exe','dji4g-helper.exe')) {
    $binary = Join-Path $repoRoot ('target/x86_64-pc-windows-msvc/release/' + $name)
    if (!(Test-Path -LiteralPath $binary)) { throw "Missing release binary: $name" }
    Copy-Item -LiteralPath $binary -Destination $staging
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'docs/使用说明.txt') -Destination $staging
Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE-MIT'),(Join-Path $repoRoot 'LICENSE-APACHE') -Destination $staging
$manifest = Get-ChildItem -LiteralPath $staging -File | ForEach-Object { [pscustomobject]@{name=$_.Name;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash} }
$manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $staging 'portable-manifest.json') -Encoding utf8
$payload = Get-ChildItem -LiteralPath $staging -File
Compress-Archive -LiteralPath $payload.FullName -DestinationPath $archive
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
    foreach ($file in $payload) {
        $entry = $zip.GetEntry($file.Name)
        if (!$entry) { throw "Archive missing $($file.Name)" }
        $stream = $entry.Open()
        try {
            $hasher = [Security.Cryptography.SHA256]::Create()
            try { $hash = [Convert]::ToHexString($hasher.ComputeHash($stream)) } finally { $hasher.Dispose() }
        } finally { $stream.Dispose() }
        if ($hash -ne (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash) { throw "Archive hash mismatch: $($file.Name)" }
    }
} finally { $zip.Dispose() }
$archiveHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
"$archiveHash  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath ($archive + '.sha256') -Encoding ascii
[pscustomobject]@{status='verified';archive=$archive;sha256=$archiveHash} | ConvertTo-Json -Compress
