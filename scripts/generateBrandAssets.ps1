param(
  [string]$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
  [string]$LogoHtmlPath = (Join-Path $PSScriptRoot "..\build\temporary-brand-source.html"),
  [string]$ChromePath = ""
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$buildDir = Join-Path $ProjectRoot "build"
$tempDir = Join-Path $ProjectRoot ".tmp-tests\brand-assets"
New-Item -ItemType Directory -Force -Path $buildDir, $tempDir | Out-Null

function Resolve-BrowserPath {
  if ($ChromePath -and (Test-Path -LiteralPath $ChromePath)) {
    return (Resolve-Path -LiteralPath $ChromePath).Path
  }

  $candidates = @(
    (Join-Path ${env:ProgramFiles} "Google\Chrome\Application\chrome.exe"),
    (Join-Path ${env:ProgramFiles(x86)} "Google\Chrome\Application\chrome.exe"),
    (Join-Path ${env:LocalAppData} "Google\Chrome\Application\chrome.exe"),
    (Join-Path ${env:ProgramFiles} "Microsoft\Edge\Application\msedge.exe"),
    (Join-Path ${env:ProgramFiles(x86)} "Microsoft\Edge\Application\msedge.exe")
  ) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }

  if ($candidates.Count -gt 0) {
    return (Resolve-Path -LiteralPath $candidates[0]).Path
  }

  throw "Chrome or Edge was not found. Pass -ChromePath with a Chromium browser executable."
}

function Test-LogoSource {
  param([string]$Path)

  if (!(Test-Path -LiteralPath $Path)) {
    throw "Logo source HTML was not found: $Path"
  }

  $source = Get-Content -Raw -LiteralPath $Path
  $requiredSnippets = @(
    ".app-icon-light",
    ".icon-orb-light",
    ".icon-glass-light",
    ".app-icon-dark",
    ".icon-orb-dark",
    ".icon-glass-dark",
    "linear-gradient(135deg, #F8FAFC 0%, #E2E8F0 100%)",
    "linear-gradient(135deg, #1E293B 0%, #0A0F1A 100%)",
    "linear-gradient(135deg, #007AFF, #4FACFE)",
    "linear-gradient(135deg, #3B82F6, #10B981)"
  )

  foreach ($snippet in $requiredSnippets) {
    if (!$source.Contains($snippet)) {
      throw "Logo source HTML is missing expected design token: $snippet"
    }
  }

  return $source
}

function Write-BrandIconHtml {
  param(
    [string]$Path,
    [string]$SourcePath,
    [ValidateSet("light", "dark")]
    [string]$Mode = "light"
  )

  $sourceForComment = [System.Net.WebUtility]::HtmlEncode($SourcePath)
  $isDark = $Mode -eq "dark"
  $containerClass = if ($isDark) { "app-icon-dark" } else { "app-icon-light" }
  $orbClass = if ($isDark) { "icon-orb-dark" } else { "icon-orb-light" }
  $glassClass = if ($isDark) { "icon-glass-dark" } else { "icon-glass-light" }
  $containerBackground = if ($isDark) { "linear-gradient(135deg, #1E293B 0%, #0A0F1A 100%)" } else { "linear-gradient(135deg, #F8FAFC 0%, #E2E8F0 100%)" }
  $containerShadow = if ($isDark) {
    "0 80px 200px -40px rgba(0, 0, 0, 0.8), inset 0 8px 8px rgba(255, 255, 255, 0.15), inset 0 0 0 4px rgba(255, 255, 255, 0.05), 0 0 0 4px rgba(0, 0, 0, 1)"
  } else {
    "0 80px 180px -48px rgba(0, 30, 60, 0.22), inset 0 8px 12px rgba(255, 255, 255, 1), inset 0 -8px 12px rgba(0, 0, 0, 0.04), 0 0 0 4px rgba(0, 0, 0, 0.04)"
  }
  $orbBackground = if ($isDark) { "linear-gradient(135deg, #3B82F6, #10B981)" } else { "linear-gradient(135deg, #007AFF, #4FACFE)" }
  $orbShadow = if ($isDark) { "0 0 160px rgba(59, 130, 246, 0.5)" } else { "0 0 140px rgba(0, 122, 255, 0.38)" }
  $orbOpacity = if ($isDark) { "0.85" } else { "1" }
  $glassBackground = if ($isDark) {
    "linear-gradient(135deg, rgba(255, 255, 255, 0.1) 0%, rgba(255, 255, 255, 0.02) 100%)"
  } else {
    "linear-gradient(135deg, rgba(255, 255, 255, 0.72) 0%, rgba(255, 255, 255, 0.32) 100%)"
  }
  $glassBorder = if ($isDark) { "4px solid rgba(255, 255, 255, 0.15)" } else { "4px solid rgba(255, 255, 255, 0.88)" }
  $glassShadow = if ($isDark) {
    "0 40px 120px -20px rgba(0, 0, 0, 0.5), inset 0 4px 4px rgba(255, 255, 255, 0.2)"
  } else {
    "0 34px 96px -28px rgba(0, 30, 60, 0.16), inset 0 6px 8px rgba(255, 255, 255, 0.94)"
  }

  @"
<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <!--
    Source: $sourceForComment
    TEMPORARY ONLY: this generated placeholder supports the Fylura identity
    migration while logo design is in progress. It is not an approved logo.
    The source file is a local token fixture, not a design reference. Mode: $Mode.
  -->
  <style>
    html, body {
      margin: 0;
      width: 1024px;
      height: 1024px;
      background: transparent;
      overflow: hidden;
    }

    .$containerClass {
      width: 1024px;
      height: 1024px;
      border-radius: 224px;
      background: $containerBackground;
      box-shadow: $containerShadow;
      position: relative;
      overflow: hidden;
    }

    .$orbClass {
      position: absolute;
      width: 440px;
      height: 440px;
      border-radius: 50%;
      top: 160px;
      right: 160px;
      background: $orbBackground;
      box-shadow: $orbShadow;
      opacity: $orbOpacity;
    }

    .$glassClass {
      position: absolute;
      width: 520px;
      height: 520px;
      border-radius: 96px;
      bottom: 160px;
      left: 160px;
      background: $glassBackground;
      backdrop-filter: blur(64px) saturate(150%);
      -webkit-backdrop-filter: blur(64px) saturate(150%);
      border: $glassBorder;
      box-shadow: $glassShadow;
    }
  </style>
</head>
<body>
  <div class="$containerClass">
    <div class="$orbClass"></div>
    <div class="$glassClass"></div>
  </div>
</body>
</html>
"@ | Set-Content -LiteralPath $Path -Encoding UTF8
}

function Invoke-ChromeScreenshot {
  param(
    [string]$HtmlPath,
    [string]$OutputPath,
    [int]$Width,
    [int]$Height
  )

  $browserPath = Resolve-BrowserPath
  if (!(Test-Path -LiteralPath $browserPath)) {
    throw "Chromium browser was not found at $browserPath"
  }

  Remove-Item -LiteralPath $OutputPath -ErrorAction SilentlyContinue
  $url = "file:///$($HtmlPath -replace '\\', '/')"
  & $browserPath `
    --headless=new `
    --disable-gpu `
    --hide-scrollbars `
    --window-size="$Width,$Height" `
    --force-device-scale-factor=1 `
    --default-background-color=00000000 `
    --disable-logging `
    --log-level=3 `
    --screenshot="$OutputPath" `
    $url | Out-Null

  for ($i = 0; $i -lt 30; $i++) {
    if (Test-Path -LiteralPath $OutputPath) { return }
    Start-Sleep -Milliseconds 100
  }
  throw "Chrome did not create $OutputPath"
}

function New-ResizedBitmap {
  param(
    [System.Drawing.Bitmap]$Source,
    [int]$Width,
    [int]$Height
  )

  $target = New-Object System.Drawing.Bitmap $Width, $Height, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $graphics = [System.Drawing.Graphics]::FromImage($target)
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
  $graphics.DrawImage($Source, 0, 0, $Width, $Height)
  $graphics.Dispose()
  return $target
}

function Save-Png {
  param(
    [System.Drawing.Bitmap]$Bitmap,
    [string]$Path
  )

  $Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
}

function ConvertTo-PngBytes {
  param([System.Drawing.Bitmap]$Bitmap)

  $stream = New-Object System.IO.MemoryStream
  $Bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
  $bytes = $stream.ToArray()
  $stream.Dispose()
  return $bytes
}

function Write-UInt16LE {
  param([System.IO.BinaryWriter]$Writer, [int]$Value)
  $Writer.Write([uint16]$Value)
}

function Write-UInt32LE {
  param([System.IO.BinaryWriter]$Writer, [long]$Value)
  $Writer.Write([uint32]$Value)
}

function Write-UInt32BE {
  param([System.IO.BinaryWriter]$Writer, [long]$Value)
  $Writer.Write([byte](($Value -shr 24) -band 0xff))
  $Writer.Write([byte](($Value -shr 16) -band 0xff))
  $Writer.Write([byte](($Value -shr 8) -band 0xff))
  $Writer.Write([byte]($Value -band 0xff))
}

function Write-Ico {
  param(
    [System.Drawing.Bitmap]$Source,
    [string]$Path
  )

  $sizes = @(16, 24, 32, 48, 64, 128, 256)
  $entries = @()
  foreach ($size in $sizes) {
    $resized = New-ResizedBitmap -Source $Source -Width $size -Height $size
    $entries += [pscustomobject]@{
      Size = $size
      Bytes = ConvertTo-PngBytes -Bitmap $resized
    }
    $resized.Dispose()
  }

  $stream = [System.IO.File]::Create($Path)
  $writer = New-Object System.IO.BinaryWriter $stream
  Write-UInt16LE $writer 0
  Write-UInt16LE $writer 1
  Write-UInt16LE $writer $entries.Count

  $offset = 6 + (16 * $entries.Count)
  foreach ($entry in $entries) {
    $dimension = if ($entry.Size -eq 256) { 0 } else { $entry.Size }
    $writer.Write([byte]$dimension)
    $writer.Write([byte]$dimension)
    $writer.Write([byte]0)
    $writer.Write([byte]0)
    Write-UInt16LE $writer 1
    Write-UInt16LE $writer 32
    Write-UInt32LE $writer $entry.Bytes.Length
    Write-UInt32LE $writer $offset
    $offset += $entry.Bytes.Length
  }
  foreach ($entry in $entries) {
    $writer.Write([byte[]]$entry.Bytes)
  }
  $writer.Dispose()
  $stream.Dispose()
}

function Write-Icns {
  param(
    [System.Drawing.Bitmap]$Source,
    [string]$Path
  )

  $types = @(
    @{ Size = 16; Type = "icp4" },
    @{ Size = 32; Type = "icp5" },
    @{ Size = 64; Type = "icp6" },
    @{ Size = 128; Type = "ic07" },
    @{ Size = 256; Type = "ic08" },
    @{ Size = 512; Type = "ic09" },
    @{ Size = 1024; Type = "ic10" }
  )
  $entries = @()
  foreach ($item in $types) {
    $resized = New-ResizedBitmap -Source $Source -Width $item.Size -Height $item.Size
    $entries += [pscustomobject]@{
      Type = $item.Type
      Bytes = ConvertTo-PngBytes -Bitmap $resized
    }
    $resized.Dispose()
  }

  $totalLength = 8
  foreach ($entry in $entries) {
    $totalLength += 8 + $entry.Bytes.Length
  }

  $stream = [System.IO.File]::Create($Path)
  $writer = New-Object System.IO.BinaryWriter $stream
  $writer.Write([System.Text.Encoding]::ASCII.GetBytes("icns"))
  Write-UInt32BE $writer $totalLength
  foreach ($entry in $entries) {
    $writer.Write([System.Text.Encoding]::ASCII.GetBytes($entry.Type))
    Write-UInt32BE $writer (8 + $entry.Bytes.Length)
    $writer.Write([byte[]]$entry.Bytes)
  }
  $writer.Dispose()
  $stream.Dispose()
}

function New-Brush {
  param(
    [int]$X,
    [int]$Y,
    [int]$Width,
    [int]$Height,
    [System.Drawing.Color]$Start,
    [System.Drawing.Color]$End
  )
  return New-Object System.Drawing.Drawing2D.LinearGradientBrush `
    ([System.Drawing.Rectangle]::new($X, $Y, $Width, $Height)),
    $Start,
    $End,
    ([System.Drawing.Drawing2D.LinearGradientMode]::ForwardDiagonal)
}

function Draw-BrandSidebar {
  param(
    [System.Drawing.Bitmap]$Logo,
    [string]$Path
  )

  $bitmap = New-Object System.Drawing.Bitmap 164, 314, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $background = New-Brush 0 0 164 314 ([System.Drawing.Color]::FromArgb(15, 23, 42)) ([System.Drawing.Color]::FromArgb(4, 8, 18))
  $graphics.FillRectangle($background, 0, 0, 164, 314)
  $background.Dispose()

  $blueGlow = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(72, 0, 122, 255))
  $greenGlow = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(55, 16, 185, 129))
  $graphics.FillEllipse($blueGlow, -42, -24, 145, 145)
  $graphics.FillEllipse($greenGlow, 70, 174, 126, 126)
  $blueGlow.Dispose()
  $greenGlow.Dispose()

  $graphics.DrawImage($Logo, 45, 44, 74, 74)

  $titleFont = New-Object System.Drawing.Font "Segoe UI", 13, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Point)
  $captionFont = New-Object System.Drawing.Font "Segoe UI", 7, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Point)
  $titleBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(242, 246, 255))
  $captionBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(132, 151, 178))
  $format = New-Object System.Drawing.StringFormat
  $format.Alignment = [System.Drawing.StringAlignment]::Center
  $graphics.DrawString("Fylura", $titleFont, $titleBrush, ([System.Drawing.RectangleF]::new(0, 138, 164, 24)), $format)
  $graphics.DrawString("SPATIAL ELEGANCE", $captionFont, $captionBrush, ([System.Drawing.RectangleF]::new(0, 164, 164, 20)), $format)

  $linePen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(45, 255, 255, 255)), 1
  $graphics.DrawLine($linePen, 42, 214, 122, 214)
  $graphics.DrawString("Local-first" + [Environment]::NewLine + "Safe by default", $captionFont, $captionBrush, ([System.Drawing.RectangleF]::new(0, 232, 164, 42)), $format)

  $linePen.Dispose()
  $format.Dispose()
  $titleBrush.Dispose()
  $captionBrush.Dispose()
  $titleFont.Dispose()
  $captionFont.Dispose()
  $graphics.Dispose()
  $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Bmp)
  $bitmap.Dispose()
}

function Draw-InstallerHeader {
  param(
    [System.Drawing.Bitmap]$Logo,
    [string]$Path
  )

  $bitmap = New-Object System.Drawing.Bitmap 150, 57, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $background = New-Brush 0 0 150 57 ([System.Drawing.Color]::FromArgb(246, 250, 255)) ([System.Drawing.Color]::FromArgb(218, 232, 246))
  $graphics.FillRectangle($background, 0, 0, 150, 57)
  $background.Dispose()

  $graphics.DrawImage($Logo, 10, 10, 36, 36)
  $titleFont = New-Object System.Drawing.Font "Segoe UI", 10, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Point)
  $captionFont = New-Object System.Drawing.Font "Segoe UI", 6, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Point)
  $titleBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(15, 23, 42))
  $captionBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(100, 116, 139))
  $graphics.DrawString("Fylura", $titleFont, $titleBrush, 52, 11)
  $graphics.DrawString("PREMIUM INSTALLER", $captionFont, $captionBrush, 53, 31)
  $titleBrush.Dispose()
  $captionBrush.Dispose()
  $titleFont.Dispose()
  $captionFont.Dispose()
  $graphics.Dispose()
  $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Bmp)
  $bitmap.Dispose()
}

function Draw-DmgBackground {
  param(
    [System.Drawing.Bitmap]$Logo,
    [string]$Path
  )

  $bitmap = New-Object System.Drawing.Bitmap 640, 420, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $background = New-Brush 0 0 640 420 ([System.Drawing.Color]::FromArgb(17, 25, 39)) ([System.Drawing.Color]::FromArgb(7, 12, 23))
  $graphics.FillRectangle($background, 0, 0, 640, 420)
  $background.Dispose()

  $blueGlow = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(48, 0, 122, 255))
  $greenGlow = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(42, 16, 185, 129))
  $graphics.FillEllipse($blueGlow, 46, -68, 260, 260)
  $graphics.FillEllipse($greenGlow, 430, 258, 250, 250)
  $blueGlow.Dispose()
  $greenGlow.Dispose()

  $graphics.DrawImage($Logo, 276, 46, 88, 88)
  $titleFont = New-Object System.Drawing.Font "Segoe UI", 24, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Point)
  $captionFont = New-Object System.Drawing.Font "Segoe UI", 9, ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Point)
  $textBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(248, 250, 252))
  $mutedBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(148, 163, 184))
  $format = New-Object System.Drawing.StringFormat
  $format.Alignment = [System.Drawing.StringAlignment]::Center
  $graphics.DrawString("Fylura", $titleFont, $textBrush, ([System.Drawing.RectangleF]::new(0, 146, 640, 38)), $format)
  $graphics.DrawString("Drag to Applications to install", $captionFont, $mutedBrush, ([System.Drawing.RectangleF]::new(0, 186, 640, 20)), $format)
  $graphics.DrawString("Local-first file lifecycle assistant", $captionFont, $mutedBrush, ([System.Drawing.RectangleF]::new(0, 368, 640, 20)), $format)
  $format.Dispose()
  $textBrush.Dispose()
  $mutedBrush.Dispose()
  $titleFont.Dispose()
  $captionFont.Dispose()
  $graphics.Dispose()
  Save-Png $bitmap $Path
  $bitmap.Dispose()
}

$logoSource = Test-LogoSource -Path $LogoHtmlPath
$iconHtml = Join-Path $buildDir "brand-icon.html"
$darkIconHtml = Join-Path $buildDir "brand-icon-dark.html"
$iconPng = Join-Path $buildDir "icon.png"
$darkIconPng = Join-Path $buildDir "icon-dark.png"
Write-BrandIconHtml -Path $iconHtml -SourcePath $LogoHtmlPath -Mode "light"
Write-BrandIconHtml -Path $darkIconHtml -SourcePath $LogoHtmlPath -Mode "dark"
Invoke-ChromeScreenshot -HtmlPath $iconHtml -OutputPath $iconPng -Width 1024 -Height 1024
Invoke-ChromeScreenshot -HtmlPath $darkIconHtml -OutputPath $darkIconPng -Width 1024 -Height 1024

$source = [System.Drawing.Bitmap]::FromFile($iconPng)
$darkSource = [System.Drawing.Bitmap]::FromFile($darkIconPng)
Write-Ico -Source $source -Path (Join-Path $buildDir "icon.ico")
Write-Ico -Source $darkSource -Path (Join-Path $buildDir "icon-dark.ico")
Write-Ico -Source $source -Path (Join-Path $buildDir "installerIcon.ico")
Write-Ico -Source $source -Path (Join-Path $buildDir "uninstallerIcon.ico")
Write-Ico -Source $source -Path (Join-Path $buildDir "installerHeaderIcon.ico")
Write-Icns -Source $source -Path (Join-Path $buildDir "icon.icns")
Write-Icns -Source $darkSource -Path (Join-Path $buildDir "icon-dark.icns")
Draw-BrandSidebar -Logo $source -Path (Join-Path $buildDir "installerSidebar.bmp")
Draw-BrandSidebar -Logo $source -Path (Join-Path $buildDir "uninstallerSidebar.bmp")
Draw-InstallerHeader -Logo $source -Path (Join-Path $buildDir "installerHeader.bmp")
Draw-DmgBackground -Logo $source -Path (Join-Path $buildDir "dmg-background.png")
$source.Dispose()
$darkSource.Dispose()

Get-ChildItem -LiteralPath $buildDir |
  Sort-Object Name |
  Select-Object Name, Length
