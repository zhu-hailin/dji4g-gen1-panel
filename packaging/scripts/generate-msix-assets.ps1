[CmdletBinding()]
param(
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot '..\msix\Assets'
}

function New-GeometryIcon {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$Size
    )

    if ($Size -lt 16) { throw 'packaging:icon_size_too_small' }
    Add-Type -AssemblyName System.Drawing
    $bitmap = [Drawing.Bitmap]::new($Size, $Size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $background = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(32, 33, 36))
    $accent = [Drawing.Pen]::new(
        [Drawing.Color]::FromArgb(91, 192, 190),
        [Math]::Max(2, [Math]::Round($Size * 0.1))
    )
    $graphics.Clear([Drawing.Color]::FromArgb(32, 33, 36))
    $graphics.FillEllipse($background, $Size * 0.1, $Size * 0.1, $Size * 0.8, $Size * 0.8)

    # Draw a simple 4G mark from lines and an arc. It intentionally uses no font, logo,
    # network download, or third-party artwork, so every output is deterministic and reusable.
    $left = $Size * 0.25
    $middle = $Size * 0.49
    $top = $Size * 0.25
    $baseline = $Size * 0.75
    $graphics.DrawLine($accent, $left, $baseline, $middle, $top)
    $graphics.DrawLine($accent, $middle, $top, $middle, $baseline)
    $graphics.DrawLine($accent, $left, $Size * 0.57, $Size * 0.58, $Size * 0.57)
    $arcRect = [Drawing.RectangleF]::new($Size * 0.48, $Size * 0.25, $Size * 0.3, $Size * 0.5)
    $graphics.DrawArc($accent, $arcRect, 45, 280)
    $graphics.DrawLine($accent, $Size * 0.63, $Size * 0.58, $Size * 0.78, $Size * 0.58)

    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    $accent.Dispose()
    $background.Dispose()
    $graphics.Dispose()
    $bitmap.Dispose()
}

# The brand logo ships with the repository (apps/panel/assets/brand).  When it is present the
# MSIX squares are scaled from it (transparent background preserved); the deterministic geometric
# mark stays as the fallback so packaging never depends on a network fetch.
$script:BrandLogoPath = Join-Path $PSScriptRoot '..\..\apps\panel\assets\brand\logo.png'

function New-BrandIcon {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$Size
    )
    if (-not (Test-Path $script:BrandLogoPath)) {
        New-GeometryIcon -Path $Path -Size $Size
        return
    }
    Add-Type -AssemblyName System.Drawing
    $logo = [Drawing.Image]::FromFile($script:BrandLogoPath)
    $bitmap = [Drawing.Bitmap]::new($Size, $Size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $inset = [Math]::Round($Size * 0.08)
    $graphics.DrawImage($logo, $inset, $inset, $Size - 2 * $inset, $Size - 2 * $inset)
    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    $logo.Dispose()
    $graphics.Dispose()
    $bitmap.Dispose()
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
New-BrandIcon -Path (Join-Path $OutputDirectory 'StoreLogo.png') -Size 50
New-BrandIcon -Path (Join-Path $OutputDirectory 'Square44x44Logo.png') -Size 44
New-BrandIcon -Path (Join-Path $OutputDirectory 'Square150x150Logo.png') -Size 150

Get-ChildItem -LiteralPath $OutputDirectory -Filter '*.png' -File |
    Select-Object Name, Length |
    ConvertTo-Json -Compress
