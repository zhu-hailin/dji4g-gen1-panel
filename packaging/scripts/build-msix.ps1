[CmdletBinding()]
param(
    [switch]$DryRun,
    [string]$OutputDirectory,
    [switch]$Sign,
    [string]$CertificateThumbprint
)

$ErrorActionPreference = 'Stop'

function Get-RepositoryRoot {
    $root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
    if (-not (Test-Path -LiteralPath (Join-Path $root 'Cargo.toml') -PathType Leaf)) {
        throw 'packaging:repository_root_invalid'
    }
    return $root
}

function Find-KitTool([string] $toolName) {
    $kitsRoot = $env:KitsRoot10
    if ([string]::IsNullOrWhiteSpace($kitsRoot)) {
        $kitsRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
    }
    if (-not (Test-Path -LiteralPath $kitsRoot -PathType Container)) {
        if ($DryRun) { return $null }
        throw 'packaging:windows_sdk_not_found'
    }

    $tool = Get-ChildItem -LiteralPath (Join-Path $kitsRoot 'bin') -Filter $toolName -File -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Name -eq 'x64' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if ($null -eq $tool -and -not $DryRun) {
        throw "packaging:sdk_tool_missing:$toolName"
    }
    if ($null -eq $tool) { return $null }
    return $tool.FullName
}

function Assert-GeneratedPath([string] $path, [string] $root) {
    $fullPath = [IO.Path]::GetFullPath($path)
    $fullRoot = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
    if (-not $fullPath.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'packaging:generated_path_outside_root'
    }
}

function Get-PngDimension([string] $path) {
    $bytes = [IO.File]::ReadAllBytes($path)
    $signature = [byte[]](137, 80, 78, 71, 13, 10, 26, 10)
    $signatureMatches = $bytes.Length -ge 24
    if ($signatureMatches) {
        for ($index = 0; $index -lt $signature.Length; $index++) {
            if ($bytes[$index] -ne $signature[$index]) {
                $signatureMatches = $false
                break
            }
        }
    }
    if (-not $signatureMatches) {
        throw 'packaging:asset_not_png'
    }
    $width = ($bytes[16] -shl 24) -bor ($bytes[17] -shl 16) -bor ($bytes[18] -shl 8) -bor $bytes[19]
    $height = ($bytes[20] -shl 24) -bor ($bytes[21] -shl 16) -bor ($bytes[22] -shl 8) -bor $bytes[23]
    return @{ Width = $width; Height = $height }
}

function Assert-MsixAssets([string] $root) {
    $expected = @{
        'StoreLogo.png' = 50
        'Square44x44Logo.png' = 44
        'Square150x150Logo.png' = 150
    }
    foreach ($name in $expected.Keys) {
        $path = Join-Path $root $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "packaging:asset_missing:$name"
        }
        $dimension = Get-PngDimension $path
        if ($dimension.Width -ne $expected[$name] -or $dimension.Height -ne $expected[$name]) {
            throw "packaging:asset_dimensions_invalid:$name"
        }
    }
}

$repoRoot = Get-RepositoryRoot
$manifest = Join-Path $repoRoot 'packaging\msix\Package.appxmanifest'
$panelExe = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\dji4g-panel.exe'
$helperExe = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\dji4g-helper.exe'
$makeAppx = Find-KitTool 'makeappx.exe'
$signTool = Find-KitTool 'signtool.exe'
$target = 'x86_64-pc-windows-msvc'
$artifactName = 'Dji4GPanel-0.1.0.0-unsigned-development-only.msix'
$assetRoot = Join-Path $repoRoot 'packaging\msix\Assets'

if ($Sign -and [string]::IsNullOrWhiteSpace($CertificateThumbprint)) {
    throw 'packaging:certificate_thumbprint_required'
}
if ($Sign -and $DryRun) {
    throw 'packaging:dry_run_cannot_sign'
}

$outputRoot = if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    Join-Path $repoRoot 'dist'
} else {
    [IO.Path]::GetFullPath($OutputDirectory)
}

$plan = [ordered]@{
    status = if ($DryRun) { 'dry-run' } else { 'build' }
    target = $target
    manifest = $manifest
    panel_exe = $panelExe
    helper_exe = $helperExe
    makeappx = $makeAppx
    signtool = $signTool
    artifact = $artifactName
    output_dir = $outputRoot
    signed = [bool]$Sign
}

if ($DryRun) {
    $plan | ConvertTo-Json -Compress
    return
}

if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    throw 'packaging:manifest_missing'
}
Assert-MsixAssets $assetRoot
if ($null -eq $makeAppx) {
    throw 'packaging:makeappx_missing'
}

Push-Location $repoRoot
try {
    & cargo build --locked --release --target $target -p dji4g-panel -p dji4g-helper
}
finally {
    Pop-Location
}
if ($LASTEXITCODE -ne 0) { throw 'packaging:cargo_build_failed' }
if (-not (Test-Path -LiteralPath $panelExe -PathType Leaf)) {
    throw 'packaging:panel_exe_missing'
}
if (-not (Test-Path -LiteralPath $helperExe -PathType Leaf)) {
    throw 'packaging:helper_exe_missing'
}

New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$output = Join-Path $outputRoot $artifactName
if (Test-Path -LiteralPath $output) {
    throw 'packaging:output_exists'
}

$stagingRoot = Join-Path ([IO.Path]::GetTempPath()) 'Dji4GPanel-msix-staging'
New-Item -ItemType Directory -Path $stagingRoot -Force | Out-Null
$staging = Join-Path $stagingRoot ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $staging -Force | Out-Null

try {
    $assets = Join-Path $staging 'Assets'
    New-Item -ItemType Directory -Path $assets -Force | Out-Null
    Copy-Item -LiteralPath $panelExe -Destination (Join-Path $staging 'dji4g-panel.exe')
    Copy-Item -LiteralPath $helperExe -Destination (Join-Path $staging 'dji4g-helper.exe')
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $staging 'AppxManifest.xml')
    foreach ($asset in @('StoreLogo.png', 'Square44x44Logo.png', 'Square150x150Logo.png')) {
        $source = Join-Path $assetRoot $asset
        Copy-Item -LiteralPath $source -Destination (Join-Path $assets $asset)
    }

    & $makeAppx pack /v /h SHA256 /d $staging /p $output /no
    if ($LASTEXITCODE -ne 0) { throw 'packaging:makeappx_failed' }

    if ($Sign) {
        $certificate = Get-ChildItem -LiteralPath "Cert:\CurrentUser\My\$CertificateThumbprint" -ErrorAction Stop
        if (-not $certificate.HasPrivateKey) { throw 'packaging:certificate_private_key_missing' }
        if ($certificate.Subject -ne 'CN=Dji4GPanel Development') { throw 'packaging:certificate_publisher_mismatch' }
        if ($null -eq $signTool) { throw 'packaging:signtool_missing' }
        & $signTool sign /fd SHA256 /sha1 $CertificateThumbprint $output
        if ($LASTEXITCODE -ne 0) { throw 'packaging:signtool_sign_failed' }
        & $signTool verify /pa /v $output
        if ($LASTEXITCODE -ne 0) { throw 'packaging:signtool_verify_failed' }
    }

    $msixHash = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
    $panelHash = (Get-FileHash -LiteralPath $panelExe -Algorithm SHA256).Hash.ToLowerInvariant()
    $helperHash = (Get-FileHash -LiteralPath $helperExe -Algorithm SHA256).Hash.ToLowerInvariant()

    $commit = $null
    try {
        $commit = (& git -C $repoRoot rev-parse HEAD 2>$null)
        if ($LASTEXITCODE -ne 0) { $commit = $null }
    }
    catch { $commit = $null }

    $sha256Path = Join-Path $outputRoot ($artifactName + '.sha256')
    Set-Content -LiteralPath $sha256Path -Value ("{0}  {1}" -f $msixHash, $artifactName) -Encoding ascii

    $manifestPath = Join-Path $outputRoot 'release-manifest.json'
    $releaseManifest = [ordered]@{
        artifact = $artifactName
        unsigned_development_only = (-not [bool]$Sign)
        signed = [bool]$Sign
        generated_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        commit = $commit
        package_identity = [ordered]@{
            name = 'Dji4GPanel'
            publisher = 'CN=Dji4GPanel Development'
            version = '0.1.0.0'
            processor_architecture = 'x64'
        }
        files = [ordered]@{
            $artifactName = $msixHash
            'dji4g-panel.exe' = $panelHash
            'dji4g-helper.exe' = $helperHash
        }
    }
    $manifestJson = $releaseManifest | ConvertTo-Json -Depth 6
    [System.IO.File]::WriteAllText($manifestPath, $manifestJson, (New-Object System.Text.UTF8Encoding($false)))

    [ordered]@{
        status = 'created'
        artifact = $output
        manifest = $manifestPath
        sha256 = $msixHash
        signed = [bool]$Sign
    } | ConvertTo-Json -Compress
}
finally {
    Assert-GeneratedPath $staging $stagingRoot
    if (Test-Path -LiteralPath $staging -PathType Container) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
}
