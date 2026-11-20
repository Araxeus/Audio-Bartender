# Draws the installer's wizard images from assets/icon.png, at 100%, 150% and 200%
# scale: a small mark for the page headers and a tall panel for the finish page.
# Run it again after changing the icon.

Add-Type -AssemblyName System.Drawing
$root = Split-Path $PSScriptRoot -Parent
$out = Join-Path $root 'installer'
$icon = [System.Drawing.Image]::FromFile((Join-Path $root 'assets/icon.png'))

function Save-Png($bmp, [string]$name) {
    $bmp.Save((Join-Path $out $name), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

function New-Canvas([int]$w, [int]$h) {
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.TextRenderingHint = 'AntiAliasGridFit'
    $bmp, $g
}

foreach ($scale in 1, 1.5, 2) {
    # Header mark: the icon on a transparent square, so it works in light and dark wizards.
    $s = [int][math]::Round(55 * $scale)
    $bmp, $g = New-Canvas $s $s
    $pad = [int]($s * 0.08)
    $g.DrawImage($icon, $pad, $pad, $s - 2 * $pad, $s - 2 * $pad)
    $g.Dispose()
    Save-Png $bmp "wizard-small-$s.png"

    # Finish panel: a deep gradient with the icon and the name.
    $w = [int][math]::Round(164 * $scale); $h = [int][math]::Round(314 * $scale)
    $bmp, $g = New-Canvas $w $h
    $bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Point 0, 0), (New-Object System.Drawing.Point $w, $h), ([System.Drawing.Color]::FromArgb(255, 24, 24, 40)), ([System.Drawing.Color]::FromArgb(255, 49, 30, 88))
    $g.FillRectangle($bg, 0, 0, $w, $h)
    # A soft glow behind the icon.
    $glow = New-Object System.Drawing.Drawing2D.GraphicsPath
    $gs = [int]($w * 0.95); $gx = [int](($w - $gs) / 2); $gy = [int]($h * 0.36 - $gs / 2)
    $glow.AddEllipse($gx, $gy, $gs, $gs)
    $gb = New-Object System.Drawing.Drawing2D.PathGradientBrush $glow
    $gb.CenterColor = [System.Drawing.Color]::FromArgb(110, 139, 92, 246)
    $gb.SurroundColors = @([System.Drawing.Color]::FromArgb(0, 139, 92, 246))
    $g.FillPath($gb, $glow)
    $is = [int]($w * 0.5)
    $g.DrawImage($icon, [int](($w - $is) / 2), [int]($h * 0.36 - $is / 2), $is, $is)
    $font = New-Object System.Drawing.Font 'Segoe UI Semibold', ([single](13 * $scale)), ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
    $fmt = New-Object System.Drawing.StringFormat; $fmt.Alignment = 'Center'
    $g.DrawString('Audio Bartender', $font, [System.Drawing.Brushes]::White, (New-Object System.Drawing.RectangleF 0, ([single]($h * 0.36 + $is * 0.62)), $w, ([single](30 * $scale))), $fmt)
    $g.Dispose()
    Save-Png $bmp "wizard-$w.png"
}
$icon.Dispose()
Get-ChildItem $out -Filter 'wizard-*.png' | ForEach-Object { $_.Name }
