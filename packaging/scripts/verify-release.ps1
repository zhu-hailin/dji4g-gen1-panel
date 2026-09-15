<#
.SYNOPSIS
    Verifies an unsigned development release candidate produced by build-msix.ps1.

.DESCRIPTION
    The verifier fails closed and detects the four classes of release defect required by the
    acceptance plan:
      1. a missing artifact or manifest entry,
      2. a SHA-256 that does not match the recorded release manifest,
      3. a package identity (Name/Publisher/Version/ProcessorArchitecture) that does not match the
         expected constants, and
      4. an unexpected or debug/spike binary inside the package.

    An .msix is an OPC/ZIP container, so the package contents are introspected with
    System.IO.Compression and do NOT require the Windows SDK / makeappx. The panel and helper
    executables are extracted from the package and re-hashed against the manifest, which proves the
    packed binaries are exactly the ones that were recorded.

    This script never signs, never installs a certificate, and never claims the candidate is
    signed or releasable. It only validates an unsigned development candidate.

.PARAMETER DistDirectory
    Directory holding the candidate. Defaults to '<repo>\dist'.

.PARAMETER Manifest
    Path to release-manifest.json. Defaults to '<DistDirectory>\release-manifest.json'.

.OUTPUTS
    A compressed JSON report on stdout. Exits non-zero if any check is Failed.
#>
[CmdletBinding()]
param(
    [string]$DistDirectory,
    [string]$Manifest
)

$ErrorActionPreference = 'Stop'

function Get-RepositoryRoot {
    $root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
    if (-not (Test-Path -LiteralPath (Join-Path $root 'Cargo.toml') -PathType Leaf)) {
        throw 'verify:repository_root_invalid'
    }
    return $root
}

function Normalize-EntryName([string]$name) {
    return ($name -replace '\\', '/').Trim('/')
}

function Get-Sha256Hex([byte[]]$bytes) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally { $sha.Dispose() }
}

$expectedIdentity = [ordered]@{
    name = 'Dji4GPanel'
    publisher = 'CN=Dji4GPanel Development'
    version = '0.1.0.0'
    processor_architecture = 'x64'
}

$requiredEntries = @(
    '[Content_Types].xml',
    'AppxManifest.xml',
    'AppxBlockMap.xml',
    'dji4g-panel.exe',
    'dji4g-helper.exe',
    'Assets/StoreLogo.png',
    'Assets/Square44x44Logo.png',
    'Assets/Square150x150Logo.png'
)

# Patterns that must never appear in a release candidate.
$forbiddenPatterns = @('*.pdb', '*.ilk', '*.exp', '*.lib', 'hotspot_spike*', '*debug*', '*.msi')

$repoRoot = Get-RepositoryRoot
$dist = if ([string]::IsNullOrWhiteSpace($DistDirectory)) { Join-Path $repoRoot 'dist' } else { [IO.Path]::GetFullPath($DistDirectory) }
$manifestPath = if ([string]::IsNullOrWhiteSpace($Manifest)) { Join-Path $dist 'release-manifest.json' } else { [IO.Path]::GetFullPath($Manifest) }

$failures = New-Object System.Collections.Generic.List[string]
$blocked = New-Object System.Collections.Generic.List[string]
$passed = New-Object System.Collections.Generic.List[string]

if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    $failures.Add('verify:manifest_missing')
    [ordered]@{ status = 'failed'; failures = $failures; blocked = $blocked; passed = $passed } | ConvertTo-Json -Compress
    exit 1
}

$rawManifest = Get-Content -LiteralPath $manifestPath -Raw
$rawManifest = $rawManifest.TrimStart([char]0xFEFF)
$releaseManifest = $rawManifest | ConvertFrom-Json

# The candidate must be an unsigned development artifact unless it was explicitly signed.
if ($releaseManifest.signed) {
    $blocked.Add('verify:signed_candidate_out_of_scope')
}

$artifactName = $releaseManifest.artifact
$artifactPath = Join-Path $dist $artifactName
if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
    $failures.Add("verify:artifact_missing:$artifactName")
    [ordered]@{ status = 'failed'; failures = $failures; blocked = $blocked; passed = $passed } | ConvertTo-Json -Compress
    exit 1
}

# 1. Whole-package hash must match the manifest.
$actualMsixHash = (Get-FileHash -LiteralPath $artifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
$expectedMsixHash = ([string]$releaseManifest.files.$artifactName).ToLowerInvariant()
if ($actualMsixHash -ne $expectedMsixHash) {
    $failures.Add("verify:hash_mismatch:$artifactName")
}
else {
    $passed.Add("verify:hash_ok:$artifactName")
}

# 2. Package identity recorded in the manifest must equal the expected constants.
foreach ($field in $expectedIdentity.Keys) {
    $recorded = [string]$releaseManifest.package_identity.$field
    if ($recorded -ne $expectedIdentity[$field]) {
        $failures.Add("verify:identity_mismatch:$field")
    }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($artifactPath)
try {
    $entryNames = @($zip.Entries | ForEach-Object { Normalize-EntryName $_.FullName })

    # 3. No unexpected entries; no debug/spike binaries.
    foreach ($name in $entryNames) {
        $leaf = [IO.Path]::GetFileName($name)
        $isForbidden = $false
        foreach ($pattern in $forbiddenPatterns) {
            if ($leaf -like $pattern) { $isForbidden = $true; break }
        }
        if ($isForbidden) {
            $failures.Add("verify:debug_binary:$name")
            continue
        }
        $isAllowed = ($requiredEntries -contains $name) -or ($name -eq 'AppxSignature.pdx')
        if (-not $isAllowed) {
            $failures.Add("verify:unexpected_entry:$name")
        }
    }

    # 4. Every required entry must be present.
    foreach ($required in $requiredEntries) {
        if ($entryNames -notcontains $required) {
            $failures.Add("verify:missing_entry:$required")
        }
    }

    # 5. The packed AppxManifest.xml identity must equal the expected constants.
    $manifestEntry = $zip.Entries | Where-Object { (Normalize-EntryName $_.FullName) -eq 'AppxManifest.xml' } | Select-Object -First 1
    if ($null -eq $manifestEntry) {
        $failures.Add('verify:missing_entry:AppxManifest.xml')
    }
    else {
        $reader = New-Object System.IO.StreamReader($manifestEntry.Open())
        try { $manifestXmlText = $reader.ReadToEnd() } finally { $reader.Dispose() }
        [xml]$manifestXml = $manifestXmlText
        $identity = $manifestXml.Package.Identity
        $actual = [ordered]@{
            name = [string]$identity.Name
            publisher = [string]$identity.Publisher
            version = [string]$identity.Version
            processor_architecture = [string]$identity.ProcessorArchitecture
        }
        foreach ($field in $expectedIdentity.Keys) {
            if ($actual[$field] -ne $expectedIdentity[$field]) {
                $failures.Add("verify:package_identity_mismatch:$field")
            }
        }
        if ($actual.name -eq $expectedIdentity.name) { $passed.Add('verify:package_identity_ok') }
    }

    # 6. The packed executables must hash to the values recorded in the manifest.
    foreach ($exe in @('dji4g-panel.exe', 'dji4g-helper.exe')) {
        $entry = $zip.Entries | Where-Object { (Normalize-EntryName $_.FullName) -eq $exe } | Select-Object -First 1
        if ($null -eq $entry) {
            $failures.Add("verify:missing_entry:$exe")
            continue
        }
        $ms = New-Object System.IO.MemoryStream
        try {
            $stream = $entry.Open()
            try { $stream.CopyTo($ms) } finally { $stream.Dispose() }
            $actualExeHash = Get-Sha256Hex $ms.ToArray()
        }
        finally { $ms.Dispose() }
        $expectedExeHash = ([string]$releaseManifest.files.$exe).ToLowerInvariant()
        if ([string]::IsNullOrWhiteSpace($expectedExeHash)) {
            $failures.Add("verify:manifest_hash_absent:$exe")
        }
        elseif ($actualExeHash -ne $expectedExeHash) {
            $failures.Add("verify:hash_mismatch:$exe")
        }
        else {
            $passed.Add("verify:hash_ok:$exe")
        }
    }
}
finally {
    $zip.Dispose()
}

$status = if ($failures.Count -gt 0) { 'failed' } else { 'passed' }
[ordered]@{
    status = $status
    artifact = $artifactPath
    sha256 = $actualMsixHash
    failures = $failures
    blocked = $blocked
    passed = $passed
} | ConvertTo-Json -Compress

if ($failures.Count -gt 0) { exit 1 }
exit 0
