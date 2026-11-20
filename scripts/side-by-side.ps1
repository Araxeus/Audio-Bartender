# Opens both flyouts pinned, next to each other. Press Enter here to close them.
#
#   pwsh scripts/side-by-side.ps1
#   pwsh scripts/side-by-side.ps1 -Shots dist   # also save a screenshot of each window
#   pwsh scripts/side-by-side.ps1 -Flags --pinned,--settings   # open on the settings page

param([string]$Shots, [string[]]$Flags = @('--pinned'))

$root = Split-Path $PSScriptRoot -Parent
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class S {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int hh, uint f);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);

    public static IntPtr FindMain(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            RECT r; GetWindowRect(h, out r);
            if (p == pid && IsWindowVisible(h) && r.R - r.L > 100) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
[void][S]::SetProcessDpiAwarenessContext([IntPtr]-4)

$area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
$procs = @()
$x = $area.Right - 2 * 400
foreach ($app in 'gpui', 'tauri') {
    $p = Start-Process (Join-Path $root "target/release/mediakeys-$app.exe") -ArgumentList $Flags -PassThru
    $hwnd = [IntPtr]::Zero
    $deadline = (Get-Date).AddSeconds(15)
    while ($hwnd -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 50; $hwnd = [S]::FindMain([uint32]$p.Id) }
    # Let the flyout finish sliding in before moving it.
    Start-Sleep -Milliseconds 500
    # Keep each flyout's height and bottom edge; only move it sideways. SWP_NOSIZE | SWP_NOZORDER
    $r = New-Object S+RECT; [void][S]::GetWindowRect($hwnd, [ref]$r)
    [void][S]::SetWindowPos($hwnd, [IntPtr]::Zero, $x, $r.T, 0, 0, 0x5)
    $x += 400
    $procs += [pscustomobject]@{ App = $app; Process = $p; Hwnd = $hwnd }
}

if ($Shots) {
    Start-Sleep -Seconds 3
    foreach ($w in $procs) {
        # PrintWindow renders the window itself, so other windows covering it don't matter.
        $r = New-Object S+RECT; $f = New-Object S+RECT
        [void][S]::GetWindowRect($w.Hwnd, [ref]$r)
        [void][S]::DwmGetWindowAttribute($w.Hwnd, 9, [ref]$f, 16)
        $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $dc = $g.GetHdc(); [void][S]::PrintWindow($w.Hwnd, $dc, 2); $g.ReleaseHdc($dc); $g.Dispose()
        $crop = New-Object System.Drawing.Rectangle ($f.L - $r.L), ($f.T - $r.T), ($f.R - $f.L), ($f.B - $f.T)
        $frame = $bmp.Clone($crop, $bmp.PixelFormat)
        $frame.Save((Join-Path $Shots "$($w.App).png")); $frame.Dispose(); $bmp.Dispose()
    }
} else {
    Read-Host 'Press Enter to close both'
}

foreach ($w in $procs) {
    Get-CimInstance Win32_Process -Filter "ParentProcessId=$($w.Process.Id)" | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Stop-Process -Id $w.Process.Id -Force -ErrorAction SilentlyContinue
}
