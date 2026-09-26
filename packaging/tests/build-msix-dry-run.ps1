[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$builder = Join-Path $repoRoot 'packaging\scripts\build-msix.ps1'
$generator = Join-Path $repoRoot 'packaging\scripts\generate-msix-assets.ps1'

$plan = & $builder -DryRun | ConvertFrom-Json
if ($plan.status -ne 'dry-run') { throw 'packaging:test_dry_run_status' }
if ($plan.target -ne 'x86_64-pc-windows-msvc') { throw 'packaging:test_dry_run_target' }
if ($plan.signed) { throw 'packaging:test_dry_run_must_be_unsigned' }
if ($plan.artifact -notlike '*unsigned-development-only.msix') {
    throw 'packaging:test_unsigned_artifact_name'
}

$workspaceVersion = (Select-String -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -Pattern '^version = "([0-9]+\.[0-9]+\.[0-9]+)"$').Matches[0].Groups[1].Value
$expectedMsixVersion = "$workspaceVersion.0"
if ($plan.artifact -ne "Dji4GPanel-$expectedMsixVersion-unsigned-development-only.msix") { throw 'packaging:test_artifact_version_mismatch' }
[xml]$packageXml = Get-Content -LiteralPath $plan.manifest -Raw
if ($packageXml.Package.Identity.Version -ne $expectedMsixVersion) { throw 'packaging:test_manifest_version_mismatch' }

function Get-PngDimension([string] $path) {
    $bytes = [IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 24) { throw 'packaging:test_asset_too_short' }
    if ($bytes[0] -ne 137 -or $bytes[1] -ne 80 -or $bytes[2] -ne 78 -or $bytes[3] -ne 71 -or
        $bytes[4] -ne 13 -or $bytes[5] -ne 10 -or $bytes[6] -ne 26 -or $bytes[7] -ne 10) {
        throw 'packaging:test_asset_not_png'
    }
    @{
        Width = ($bytes[16] -shl 24) -bor ($bytes[17] -shl 16) -bor ($bytes[18] -shl 8) -bor $bytes[19]
        Height = ($bytes[20] -shl 24) -bor ($bytes[21] -shl 16) -bor ($bytes[22] -shl 8) -bor $bytes[23]
    }
}

$temp = Join-Path ([IO.Path]::GetTempPath()) ('Dji4GPanel-assets-test-' + [guid]::NewGuid().ToString('N'))
try {
    & $generator -OutputDirectory $temp | Out-Null
    foreach ($asset in @{
        'StoreLogo.png' = 50
        'Square44x44Logo.png' = 44
        'Square150x150Logo.png' = 150
    }.GetEnumerator()) {
        $path = Join-Path $temp $asset.Key
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "packaging:test_generated_asset_missing:$($asset.Key)"
        }
        $dimension = Get-PngDimension $path
        if ($dimension.Width -ne $asset.Value -or $dimension.Height -ne $asset.Value) {
            throw "packaging:test_generated_asset_dimensions:$($asset.Key)"
        }
    }
}
finally {
    if (Test-Path -LiteralPath $temp -PathType Container) {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
}

'{"status":"passed","test":"build-msix-dry-run"}'
