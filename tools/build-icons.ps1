#Requires -Version 5.1
<#
.SYNOPSIS
  Builds every icon and brand image Halcyon ships, from the masters in assets/brand.

.DESCRIPTION
  `npm run icon` runs this. It replaces tools/make-icon.cjs, which *drew* the Phase 0 icon — a
  blue rounded square — and would have painted over the real brand every time it ran.

  Inputs (assets/brand, from the designer's export of 2026-09-17):
    store/halcyon-icon-accent.svg   the app icon master: a white envelope on #EC3013, full bleed
    ico/halcyon-{N}.png             hand-drawn small sizes; the stroke thickens as the icon shrinks
    msix/*.png                      the Windows package set, already named with MSIX qualifiers

  Outputs, all committed:
    src-tauri/icons/                the bundle icons; icon.ico is the exe, window and taskbar
    src-tauri/icons/tray/           the notification-area icon at each scale, embedded by tray.rs
    src-tauri/msix/Assets/          the Store package's tiles and app-list icons
    src-tauri/installer/*.bmp       the NSIS installer's header and welcome-page images
    public/favicon.svg              the browser build's tab icon

  Two deliberate departures from a plain "resize the master":
    * icon.ico, 32x32.png and 64x64.png are built from the hand-drawn sizes, not scaled down.
      The brand sets the stroke at 8% of the width at 256px and 12% at 16px so a small icon
      keeps the same visual weight; a downscale of the 256 would thin it to a hairline.
    * msix/SplashScreen.* is not packaged. AppxManifest.xml declares no splash screen, on
      purpose: Windows never shows one for a full-trust desktop app, and declaring one made the
      App Certification Kit fail. See the comment there.
#>

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root      = Split-Path -Parent $PSScriptRoot
$brand     = Join-Path $root 'assets\brand'
$icons     = Join-Path $root 'src-tauri\icons'
$msix      = Join-Path $root 'src-tauri\msix\Assets'
$installer = Join-Path $root 'src-tauri\installer'
$public    = Join-Path $root 'public'

$master = Join-Path $brand 'store\halcyon-icon-accent.svg'

# The brand's accent, and the neutral plate it pairs with on light surfaces.
$accent  = [Drawing.ColorTranslator]::FromHtml('#EC3013')
$neutral = [Drawing.ColorTranslator]::FromHtml('#F3F2F2')

function Get-PngSize([string]$path) {
  # Width and height straight from IHDR, so a mislabelled file is caught rather than packed.
  $bytes = [IO.File]::ReadAllBytes($path)
  $signature = [BitConverter]::ToString($bytes, 0, 8)
  if ($signature -ne '89-50-4E-47-0D-0A-1A-0A') { throw "$path is not a PNG" }
  # Each byte widened to [int] before the shift. PowerShell shifts a [byte] as a byte, so
  # 1 -shl 8 came out as 0 and a 256px icon measured 0x0 — the first run failed on exactly that.
  $width  = ([int]$bytes[16] -shl 24) -bor ([int]$bytes[17] -shl 16) -bor ([int]$bytes[18] -shl 8) -bor [int]$bytes[19]
  $height = ([int]$bytes[20] -shl 24) -bor ([int]$bytes[21] -shl 16) -bor ([int]$bytes[22] -shl 8) -bor [int]$bytes[23]
  return @($width, $height)
}

function Assert-Square([string]$path, [int]$size) {
  $w, $h = Get-PngSize $path
  if ($w -ne $size -or $h -ne $size) { throw "$path is ${w}x${h}, expected ${size}x${size}" }
}

# --------------------------------------------------------------------------- 1. inputs
foreach ($required in @($master, (Join-Path $brand 'ico'), (Join-Path $brand 'msix'))) {
  if (-not (Test-Path $required)) { throw "missing brand input: $required" }
}

# ------------------------------------------------------- 2. bundle PNGs from the master
# `tauri icon` renders the SVG. It also writes Android, iOS and macOS sets, which this
# Windows-only app does not ship, so it runs into a scratch folder and only the desktop
# files are taken.
$scratch = Join-Path ([IO.Path]::GetTempPath()) ("halcyon-icons-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
  # Silenced entirely: it narrates every Android and iOS file it writes, on stderr, for
  # platforms this app does not ship. Under 'Stop', PowerShell 5.1 turns redirected native
  # stderr into terminating errors, so the preference is relaxed for this one call and the exit
  # code is what decides.
  Push-Location $root
  $preference = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    & npx tauri icon $master -o $scratch *> $null
  } finally {
    $ErrorActionPreference = $preference
    Pop-Location
  }
  if ($LASTEXITCODE -ne 0) { throw "tauri icon failed with exit code $LASTEXITCODE" }

  $desktop = @(
    '128x128.png', '128x128@2x.png', 'icon.png',
    'Square30x30Logo.png', 'Square44x44Logo.png', 'Square71x71Logo.png', 'Square89x89Logo.png',
    'Square107x107Logo.png', 'Square142x142Logo.png', 'Square150x150Logo.png',
    'Square284x284Logo.png', 'Square310x310Logo.png', 'StoreLogo.png'
  )
  foreach ($name in $desktop) {
    Copy-Item (Join-Path $scratch $name) (Join-Path $icons $name) -Force
  }
} finally {
  Remove-Item $scratch -Recurse -Force -ErrorAction SilentlyContinue
}

# The hand-drawn sizes win wherever one exists.
foreach ($size in 32, 64) {
  $source = Join-Path $brand "ico\halcyon-$size.png"
  Assert-Square $source $size
  Copy-Item $source (Join-Path $icons "${size}x${size}.png") -Force
}

# The Phase 0 generator's output, which nothing reads any more.
Remove-Item (Join-Path $icons 'icon-source.png') -ErrorAction SilentlyContinue

# ------------------------------------------------------------------- 3. icon.ico
# PNG-compressed entries, as `tauri icon` writes them and as the resource compiler and NSIS
# already accept. 32 first: it is the entry a few older shell paths take without asking.
$icoSizes = 32, 16, 20, 24, 40, 48, 64, 256
$images = foreach ($size in $icoSizes) {
  $path = Join-Path $brand "ico\halcyon-$size.png"
  Assert-Square $path $size
  [pscustomobject]@{ Size = $size; Bytes = [IO.File]::ReadAllBytes($path) }
}

$stream = New-Object IO.MemoryStream
$writer = New-Object IO.BinaryWriter($stream)
$writer.Write([uint16]0)                 # reserved
$writer.Write([uint16]1)                 # type: icon
$writer.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($image in $images) {
  $edge = if ($image.Size -ge 256) { 0 } else { $image.Size }   # 0 means 256
  $writer.Write([byte]$edge)             # width
  $writer.Write([byte]$edge)             # height
  $writer.Write([byte]0)                 # palette size
  $writer.Write([byte]0)                 # reserved
  $writer.Write([uint16]1)               # colour planes
  $writer.Write([uint16]32)              # bits per pixel
  $writer.Write([uint32]$image.Bytes.Length)
  $writer.Write([uint32]$offset)
  $offset += $image.Bytes.Length
}
foreach ($image in $images) { $writer.Write($image.Bytes) }
$writer.Flush()
[IO.File]::WriteAllBytes((Join-Path $icons 'icon.ico'), $stream.ToArray())
$writer.Dispose()

# The tray's own copies. Tauri makes icon.ico's *first* entry the window icon, and the tray would
# otherwise get that 32px image shrunk to 16-24px by the shell, losing the heavier stroke the
# small sizes are drawn with. platform/tray.rs embeds these and picks one for the display scale.
$tray = Join-Path $icons 'tray'
New-Item -ItemType Directory -Force -Path $tray | Out-Null
foreach ($size in 16, 20, 24, 32, 40, 48) {
  $source = Join-Path $brand "ico\halcyon-$size.png"
  Assert-Square $source $size
  Copy-Item $source (Join-Path $tray "halcyon-$size.png") -Force
}

# ---------------------------------------------------------------- 4. MSIX assets
# The export's file names already carry the scale-, targetsize- and altform- qualifiers MSIX
# resolves, so they are copied as they are. The folder is emptied first: a stale asset left
# behind is still packaged, and still chosen for whatever qualifier it matches.
New-Item -ItemType Directory -Force -Path $msix | Out-Null
Get-ChildItem $msix -File | Remove-Item -Force
Get-ChildItem (Join-Path $brand 'msix') -Filter '*.png' |
  Where-Object { $_.Name -notlike 'SplashScreen*' } |
  ForEach-Object { Copy-Item $_.FullName (Join-Path $msix $_.Name) }

# ------------------------------------------------------------ 5. installer images
# NSIS takes 24-bit BMPs at fixed sizes: the header at 150x57, the welcome and finish pages'
# sidebar at 164x314. The layouts are the ones these files already had — the icon on a light
# ground, a rule of the accent along the sidebar's foot — with the new icon and accent in
# them. The icons are drawn at their own pixel sizes, from hand-drawn files, not resampled.
function New-InstallerBitmap([int]$width, [int]$height, [scriptblock]$paint, [string]$path) {
  $bitmap = New-Object Drawing.Bitmap($width, $height, [Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $graphics = [Drawing.Graphics]::FromImage($bitmap)
  try {
    $graphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
    $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
    $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::Half
    & $paint $graphics
  } finally { $graphics.Dispose() }
  $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Bmp)
  $bitmap.Dispose()
}

function Draw-Image([Drawing.Graphics]$graphics, [string]$path, [int]$x, [int]$y) {
  $image = [Drawing.Image]::FromFile($path)
  try { $graphics.DrawImage($image, $x, $y, $image.Width, $image.Height) } finally { $image.Dispose() }
}

$icon88 = Join-Path $brand 'msix\Square44x44Logo.scale-200.png'   # the 88px app-list icon
$icon40 = Join-Path $brand 'ico\halcyon-40.png'
Assert-Square $icon88 88
Assert-Square $icon40 40

New-InstallerBitmap 164 314 {
  param($g)
  $g.Clear($neutral)
  Draw-Image $g $icon88 38 52
  $rule = New-Object Drawing.SolidBrush($accent)
  $g.FillRectangle($rule, 0, 311, 164, 3)
  $rule.Dispose()
} (Join-Path $installer 'sidebar.bmp')

New-InstallerBitmap 150 57 {
  param($g)
  $g.Clear([Drawing.Color]::White)
  Draw-Image $g $icon40 102 9
} (Join-Path $installer 'header.bmp')

# --------------------------------------------------------------------- 6. favicon
New-Item -ItemType Directory -Force -Path $public | Out-Null
Copy-Item $master (Join-Path $public 'favicon.svg') -Force

Write-Host "Icons built from $brand"
