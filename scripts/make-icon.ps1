# Draws assets/icon.ico (16-256 px, PNG entries) and assets/icon.png.

Add-Type -AssemblyName System.Drawing
$root = Split-Path $PSScriptRoot -Parent

function New-IconPng([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)

    $r = [single]($s * 0.22); $d = $r * 2; $m = [single]0; $e = [single]($s - 1)
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddArc($m, $m, $d, $d, 180, 90); $path.AddArc($e - $d, $m, $d, $d, 270, 90)
    $path.AddArc($e - $d, $e - $d, $d, $d, 0, 90); $path.AddArc($m, $e - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.PointF 0, 0), (New-Object System.Drawing.PointF $s, $s), ([System.Drawing.Color]::FromArgb(255, 99, 102, 241)), ([System.Drawing.Color]::FromArgb(255, 168, 85, 247))
    $g.FillPath($brush, $path)

    # Play triangle plus pause bars.
    $white = [System.Drawing.Brushes]::White
    $u = $s / 16.0
    $tri = [System.Drawing.PointF[]]@(
        (New-Object System.Drawing.PointF ([single](3.6 * $u)), ([single](4.2 * $u))),
        (New-Object System.Drawing.PointF ([single](9.2 * $u)), ([single](8.0 * $u))),
        (New-Object System.Drawing.PointF ([single](3.6 * $u)), ([single](11.8 * $u)))
    )
    $g.FillPolygon($white, $tri)
    $g.FillRectangle($white, [single](10.0 * $u), [single](4.2 * $u), [single](1.5 * $u), [single](7.6 * $u))
    $g.FillRectangle($white, [single](12.4 * $u), [single](4.2 * $u), [single](1.5 * $u), [single](7.6 * $u))
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
