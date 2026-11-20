# Draws assets/icon.ico (16-256 px, PNG entries) and assets/icon.png from the Back Bar
# geometry in assets/logo/icon.svg and icon-small.svg. Sizes up to 32 px use the small cut,
# which has chunkier bottles so they stay apart.

Add-Type -AssemblyName System.Drawing
$root = Split-Path $PSScriptRoot -Parent

# One bottle on the 256 grid: a neck with a softened top, 45-degree shoulders, and a body with
# rounded base corners. Mirrors bottle() in the SVG masters.
function Add-Bottle($path, [double]$k, $nx0, $nx1, $ntop, $sx0, $sx1, $stop, $base, $nr, $br) {
    $f = { param($v) [single]($v * $k) }
    $nb = $stop - ($sx1 - $nx1)
    $path.StartFigure()
    $path.AddArc((& $f $nx0), (& $f $ntop), (& $f (2 * $nr)), (& $f (2 * $nr)), 180, 90)
    $path.AddArc((& $f ($nx1 - 2 * $nr)), (& $f $ntop), (& $f (2 * $nr)), (& $f (2 * $nr)), 270, 90)
    $path.AddLine((& $f $nx1), (& $f ($ntop + $nr)), (& $f $nx1), (& $f $nb))
    $path.AddLine((& $f $nx1), (& $f $nb), (& $f $sx1), (& $f $stop))
    $path.AddLine((& $f $sx1), (& $f $stop), (& $f $sx1), (& $f ($base - $br)))
    $path.AddArc((& $f ($sx1 - 2 * $br)), (& $f ($base - 2 * $br)), (& $f (2 * $br)), (& $f (2 * $br)), 0, 90)
    $path.AddArc((& $f $sx0), (& $f ($base - 2 * $br)), (& $f (2 * $br)), (& $f (2 * $br)), 90, 90)
    $path.AddLine((& $f $sx0), (& $f ($base - $br)), (& $f $sx0), (& $f $stop))
    $path.AddLine((& $f $sx0), (& $f $stop), (& $f $nx0), (& $f $nb))
    $path.CloseFigure()
}

function New-IconPng([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    $k = $s / 256.0

    # The tile: 240 wide with a 56 radius, inset 8.
    $r = [single](112 * $k); $m = [single](8 * $k); $e = [single](248 * $k)
    $tile = New-Object System.Drawing.Drawing2D.GraphicsPath
    $tile.AddArc($m, $m, $r, $r, 180, 90); $tile.AddArc($e - $r, $m, $r, $r, 270, 90)
    $tile.AddArc($e - $r, $e - $r, $r, $r, 0, 90); $tile.AddArc($m, $e - $r, $r, $r, 90, 90)
    $tile.CloseFigure()
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.PointF $m, $m), (New-Object System.Drawing.PointF $e, $e), ([System.Drawing.Color]::FromArgb(255, 0x63, 0x66, 0xF1)), ([System.Drawing.Color]::FromArgb(255, 0x7C, 0x3A, 0xED))
    $g.FillPath($brush, $tile)

    $bottles = New-Object System.Drawing.Drawing2D.GraphicsPath
    if ($s -le 32) {
        Add-Bottle $bottles $k 58 78 100 44 92 142 208 6 12
        Add-Bottle $bottles $k 118 138 48 104 152 94 208 6 12
        Add-Bottle $bottles $k 178 198 128 164 212 162 208 6 12
    }
    else {
        Add-Bottle $bottles $k 66 82 96 56 92 124 212 4 10
        Add-Bottle $bottles $k 120 136 44 108 148 82 212 4 10
        Add-Bottle $bottles $k 174 190 128 164 200 152 212 4 10
    }
    $g.FillPath([System.Drawing.Brushes]::White, $bottles)
    $g.Dispose()

    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , $ms.ToArray()
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$pngs = foreach ($s in $sizes) { , (New-IconPng $s) }

$out = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $out
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]; $len = $pngs[$i].Length
    $w.Write([byte]($s % 256)); $w.Write([byte]($s % 256)); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$len); $w.Write([uint32]$offset)
    $offset += $len
}
foreach ($p in $pngs) { $w.Write($p) }
[IO.File]::WriteAllBytes((Join-Path $root 'assets/icon.ico'), $out.ToArray())
[IO.File]::WriteAllBytes((Join-Path $root 'assets/icon.png'), (New-IconPng 512))
