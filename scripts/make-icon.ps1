# Builds the Windows icon resources from a single square PNG.
#
#   ./scripts/make-icon.ps1 [-Source path\to\duckie.png]
#
# Produces assets/duckie.ico, which the build script compiles into duckie.exe so Explorer and a
# pinned taskbar shortcut have an icon, and assets/window-icon.rgba, the straight-alpha pixels the
# running window sets on itself. Both are committed; rerun this only when the artwork changes.
param(
    [string]$Source = 'assets/duckie.png',
    [string]$OutputDirectory = 'assets'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    $source = (Resolve-Path $Source).Path
    $original = [System.Drawing.Bitmap]::new($source)
    if ($original.Width -ne $original.Height) { throw "Icon source must be square, got $($original.Width)x$($original.Height)" }

    # Every size Windows asks for between a 16px list and a 256px jumbo view, including the
    # intermediate ones display scaling picks (125% wants 20 and 40).
    $sizes = 16, 20, 24, 32, 40, 48, 64, 96, 128, 256

    function Resize([int]$size) {
        $bitmap = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CompositingMode = 'SourceCopy'
        $graphics.InterpolationMode = 'HighQualityBicubic'
        $graphics.PixelOffsetMode = 'HighQuality'
        $graphics.SmoothingMode = 'HighQuality'
        $graphics.DrawImage($original, [System.Drawing.Rectangle]::new(0, 0, $size, $size))
        $graphics.Dispose()
        $bitmap
    }
    function ArgbRows([System.Drawing.Bitmap]$bitmap) {
        $rectangle = [System.Drawing.Rectangle]::new(0, 0, $bitmap.Width, $bitmap.Height)
        $data = $bitmap.LockBits($rectangle, 'ReadOnly', [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $bytes = [byte[]]::new($data.Stride * $bitmap.Height)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
        $bitmap.UnlockBits($data)
        # GDI+ hands back BGRA with straight alpha, which is what both outputs want.
        , $bytes
    }
    # An ICO entry below 256px is a DIB: a header, bottom-up BGRA, then the legacy 1-bit mask.
    function DibEntry([System.Drawing.Bitmap]$bitmap) {
        $size = $bitmap.Width
        $pixels = ArgbRows $bitmap
        $stream = [System.IO.MemoryStream]::new()
        $writer = [System.IO.BinaryWriter]::new($stream)
        $maskStride = [int][Math]::Floor(($size + 31) / 32) * 4
        $writer.Write([uint32]40)
        $writer.Write([int32]$size)
        $writer.Write([int32]($size * 2))   # XOR and AND bitmaps stacked
        $writer.Write([uint16]1)
        $writer.Write([uint16]32)
        $writer.Write([uint32]0)            # BI_RGB
        $writer.Write([uint32]($size * $size * 4 + $maskStride * $size))
        0..3 | ForEach-Object { $writer.Write([uint32]0) }
        for ($y = $size - 1; $y -ge 0; $y--) {
            $writer.Write($pixels, $y * $size * 4, $size * 4)
        }
        # Mark fully transparent pixels in the mask too, for anything still reading it.
        for ($y = $size - 1; $y -ge 0; $y--) {
            $row = [byte[]]::new($maskStride)
            for ($x = 0; $x -lt $size; $x++) {
                if ($pixels[($y * $size + $x) * 4 + 3] -eq 0) {
                    $row[[int][Math]::Floor($x / 8)] = $row[[int][Math]::Floor($x / 8)] -bor (0x80 -shr ($x % 8))
                }
            }
            $writer.Write($row)
        }
        $writer.Flush()
        # The unary comma matters: without it PowerShell unrolls the array into single bytes.
        , $stream.ToArray()
    }
    function PngEntry([System.Drawing.Bitmap]$bitmap) {
        $stream = [System.IO.MemoryStream]::new()
        $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        , $stream.ToArray()
    }

    $entries = @()
    foreach ($size in $sizes) {
        $bitmap = Resize $size
        # 256px is stored PNG-compressed; a raw DIB at that size would dominate the file.
        [byte[]]$data = if ($size -eq 256) { PngEntry $bitmap } else { DibEntry $bitmap }
        if ($data.Length -lt 64) { throw "Entry for ${size}px came out empty" }
        $entries += , @{ Size = $size; Data = $data }
        if ($size -eq 128) { $window = ArgbRows $bitmap }
        if ($size -ne 128) { $bitmap.Dispose() }
    }

    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $ico = [System.IO.MemoryStream]::new()
    $writer = [System.IO.BinaryWriter]::new($ico)
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$entries.Count)
    $offset = 6 + 16 * $entries.Count
    foreach ($entry in $entries) {
        $writer.Write([byte]($entry.Size % 256))   # 256 is recorded as 0
        $writer.Write([byte]($entry.Size % 256))
        $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$entry.Data.Length)
        $writer.Write([uint32]$offset)
        $offset += $entry.Data.Length
    }
    foreach ($entry in $entries) { $writer.Write($entry.Data) }
    $writer.Flush()
    [System.IO.File]::WriteAllBytes((Join-Path $OutputDirectory 'duckie.ico'), $ico.ToArray())

    # The window icon wants straight RGBA, not the BGRA GDI+ produced.
    for ($i = 0; $i -lt $window.Length; $i += 4) {
        $blue = $window[$i]; $window[$i] = $window[$i + 2]; $window[$i + 2] = $blue
    }
    [System.IO.File]::WriteAllBytes((Join-Path $OutputDirectory 'window-icon.rgba'), $window)
    $original.Dispose()

    Get-ChildItem (Join-Path $OutputDirectory 'duckie.ico'), (Join-Path $OutputDirectory 'window-icon.rgba') |
        Select-Object Name, Length
} finally { Pop-Location }
