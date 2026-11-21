# Builds both front ends, copies them to dist/, then measures each in the tray and
# with its flyout open, and screenshots the flyout.
#
#   pwsh scripts/measure.ps1            # build + measure
#   pwsh scripts/measure.ps1 -NoBuild
#   pwsh scripts/measure.ps1 -NoBuild -Cold   # right after a reboot: first launch only
#
# Start is the median of -Runs launches from closed to the flyout showing. Reopen times
# open the flyout of a copy already running in the tray, right away and after
# -IdleSeconds (Tauri frees its webview after 60 s closed). A true cold start needs a
# fresh boot, because Windows keeps the exe and WebView2 files cached after one launch.
#
# RAM is the private working set (Task Manager's Memory column) summed over the
# process tree, so WebView2 helpers count toward Tauri.

param([switch]$NoBuild, [switch]$Cold, [switch]$NoReopen, [string[]]$Apps = @('gpui', 'tauri'), [int]$Runs = 5, [int]$IdleSeconds = 65, [int]$SettleSeconds = 4)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null

if (-not $NoBuild) {
    cargo build --release --manifest-path (Join-Path $root 'Cargo.toml') -p audio_bartender_gpui -p audio_bartender_tauri
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
}

Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class W {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, IntPtr e);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
    public delegate bool EnumProc(IntPtr h, IntPtr l);

    [StructLayout(LayoutKind.Sequential)] public struct PMC2 {
        public uint cb, PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage,
            QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage,
            PrivateUsage, PrivateWorkingSetSize, SharedCommitUsage;
    }
    [DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(uint a, bool i, int pid);
    [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
    [DllImport("psapi.dll")] public static extern bool GetProcessMemoryInfo(IntPtr h, out PMC2 c, uint cb);

    // Private working set in bytes (Task Manager's "Memory" column); 0 if the process is gone.
    public static ulong PrivateWs(int pid) {
        IntPtr h = OpenProcess(0x1000, false, pid);
        if (h == IntPtr.Zero) return 0;
        PMC2 c; c.cb = (uint)Marshal.SizeOf(typeof(PMC2));
        bool ok = GetProcessMemoryInfo(h, out c, c.cb);
        CloseHandle(h);
        return ok ? (ulong)c.PrivateWorkingSetSize : 0;
    }

    // The first visible top-level window owned by pid that is wider than a tooltip.
    public static IntPtr FindMain(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            RECT r; DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT)));
            if (p == pid && IsWindowVisible(h) && r.R - r.L > 100) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern int GetWindowRgn(IntPtr h, IntPtr rgn);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateRectRgn(int l, int t, int r, int b);
    [DllImport("gdi32.dll")] public static extern int GetRgnBox(IntPtr rgn, out RECT r);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);

    // The flyout card. The window never resizes and its region shows only the card plus
    // an 8 px shadow margin, or 12 px toward a Windows 11 taskbar (MARGIN and FLOAT in
    // the Tauri main.rs). Cropping that margin leaves the card alone.
    public static RECT Card(IntPtr h) {
        RECT r = Frame(h);
        double scale = GetDpiForWindow(h) / 96.0;
        int m = (int)Math.Round(8 * scale);
        int edge = Environment.OSVersion.Version.Build >= 22000 ? (int)Math.Round(12 * scale) : 0;
        r.L += m; r.T += m; r.R -= m; r.B -= edge;
        return r;
    }

    // What shows on screen: the window region's bounds, else the window's frame.
    public static RECT Frame(IntPtr h) {
        RECT frame; DwmGetWindowAttribute(h, 9, out frame, Marshal.SizeOf(typeof(RECT)));
        IntPtr rgn = CreateRectRgn(0, 0, 0, 0);
        try {
            if (GetWindowRgn(h, rgn) == 0) return frame;
            RECT box, win;
            GetRgnBox(rgn, out box);
            GetWindowRect(h, out win);
            box.L += win.L; box.R += win.L; box.T += win.T; box.B += win.T;
            return box;
        } finally { DeleteObject(rgn); }
    }

    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        mouse_event(0x2, 0, 0, 0, IntPtr.Zero);
        mouse_event(0x4, 0, 0, 0, IntPtr.Zero);
    }

    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);

    // Windows blocks focus stealing; a synthetic Alt press lifts the block.
    public static void Front(IntPtr h) {
        keybd_event(0x12, 0, 0, IntPtr.Zero);
        keybd_event(0x12, 0, 2, IntPtr.Zero);
        SetWindowPos(h, (IntPtr)(-1), 0, 0, 0, 0, 0x3);
        SetForegroundWindow(h);
        SetWindowPos(h, (IntPtr)(-2), 0, 0, 0, 0, 0x3);
    }

    public static void Escape() {
        keybd_event(0x1B, 0, 0, IntPtr.Zero);
        keybd_event(0x1B, 0, 2, IntPtr.Zero);
    }
}
'@
[void][W]::SetProcessDpiAwarenessContext([IntPtr]-4)

function Get-Tree([int]$rootPid) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    $ids = [System.Collections.Generic.List[int]]::new()
    $ids.Add($rootPid)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        $all | Where-Object { $_.ParentProcessId -eq $ids[$i] } | ForEach-Object { $ids.Add([int]$_.ProcessId) }
    }
    $ids
}

function Get-PrivateMB([int]$rootPid) {
    $ids = Get-Tree $rootPid
    [pscustomobject]@{
        MB    = [math]::Round((($ids | ForEach-Object { [W]::PrivateWs($_) } | Measure-Object -Sum).Sum) / 1MB, 1)
        Count = @($ids | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }).Count
    }
}

# A 2 px window of our own in the screen's corner. Clicking it takes focus from the
# flyout, which closes it. Windows can refuse a synthetic focus switch, never a click.
Add-Type -AssemblyName System.Windows.Forms
$focusSink = New-Object System.Windows.Forms.Form -Property @{
    ShowInTaskbar = $false; FormBorderStyle = 'None'; StartPosition = 'Manual'; TopMost = $true
    Location = New-Object System.Drawing.Point(0, 0); Size = New-Object System.Drawing.Size(2, 2)
}
$focusSink.Show()

function Close-Flyout([int]$procId) {
    [W]::Click(0, 0)
    [System.Windows.Forms.Application]::DoEvents()
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ([W]::FindMain([uint32]$procId) -ne [IntPtr]::Zero) {
        if ($sw.Elapsed.TotalSeconds -gt 5) { throw "The flyout of $procId did not close, so reopen times would be wrong." }
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 20
    }
    Write-Host "  closed after $([int]$sw.Elapsed.TotalMilliseconds) ms"
}

# Milliseconds from a second launch until the running copy's flyout shows.
function Open-Again([string]$exe, [int]$procId) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Start-Process $exe | Out-Null
    while ($sw.Elapsed.TotalSeconds -lt 20) {
        if ([W]::FindMain([uint32]$procId) -ne [IntPtr]::Zero) { return [int]$sw.Elapsed.TotalMilliseconds }
        Start-Sleep -Milliseconds 10
    }
    -1
}

# Milliseconds from launch until the flyout window shows, or -1 after 20 s.
function Start-Flyout($p) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt 20) {
        if ([W]::FindMain([uint32]$p.Id) -ne [IntPtr]::Zero) { return [int]$sw.Elapsed.TotalMilliseconds }
        Start-Sleep -Milliseconds 10
    }
    -1
}

function Save-Shot($rect, [string]$path) {
    $bmp = New-Object System.Drawing.Bitmap ($rect.R - $rect.L), ($rect.B - $rect.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.L, $rect.T, 0, 0, $bmp.Size)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

if (-not $NoReopen -and -not $Cold) {
    $live = Get-Process | Where-Object { $_.ProcessName -match '^(AudioBartender|audio_bartender_)' }
    if ($live) { throw "Quit Audio Bartender first: a running copy would answer the reopen launches." }
}

$results = foreach ($app in $Apps) {
    $exe = Join-Path $root "target/release/audio_bartender_$app.exe"
    try { Copy-Item $exe (Join-Path $dist "audio_bartender_$app.exe") -Force }
    catch { Write-Warning "dist/audio_bartender_$app.exe is in use; not replaced" }

    if ($Cold) {
        $p = Start-Process $exe -ArgumentList '--pinned', '--dev' -PassThru
        $ms = Start-Flyout $p
        Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 800
        [pscustomobject]@{ App = $app; ColdMs = $ms }
        continue
    }

    # Tray only: what the app costs while you're not looking at it.
    $p = Start-Process $exe -ArgumentList '--hidden', '--dev' -PassThru
    Start-Sleep -Seconds ($SettleSeconds + 2)
    $tray = Get-PrivateMB $p.Id
    Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 800

    # Warm starts: the median of several launches.
    $times = foreach ($i in 2..$Runs | Where-Object { $Runs -gt 1 }) {
        $p = Start-Process $exe -ArgumentList '--pinned', '--dev' -PassThru
        Start-Flyout $p
        Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 800
    }

    # Flyout open, pinned so it stays up while measured.
    $p = Start-Process $exe -ArgumentList '--pinned', '--dev' -PassThru
    $times = @($times) + (Start-Flyout $p) | Where-Object { $_ -ge 0 } | Sort-Object
    $hwnd = [W]::FindMain([uint32]$p.Id)
    if (-not $times -or $hwnd -eq [IntPtr]::Zero) { Write-Warning "$app : no flyout"; Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }; continue }
    $startup = $times[[int][math]::Floor(($times.Count - 1) / 2)]

    Start-Sleep -Seconds $SettleSeconds
    $open = Get-PrivateMB $p.Id
    Save-Shot ([W]::Card($hwnd)) (Join-Path $dist "$app.png")
    Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 800

    # Reopening from the tray. Without --dev, a second launch opens the running copy.
    $reopen = $idle = $null
    if (-not $NoReopen) {
        $p = Start-Process $exe -ArgumentList '--hidden' -PassThru
        Start-Sleep -Seconds $SettleSeconds
        [void](Open-Again $exe $p.Id)
        Start-Sleep -Seconds 1
        Close-Flyout $p.Id
        Start-Sleep -Seconds 2
        $reopen = Open-Again $exe $p.Id
        Start-Sleep -Seconds 1
        Close-Flyout $p.Id
        Write-Host "$app : idle for $IdleSeconds s"
        Start-Sleep -Seconds $IdleSeconds
        $idle = Open-Again $exe $p.Id
        Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 800
    }

    [pscustomobject]@{
        App         = $app
        ExeMB       = [math]::Round((Get-Item $exe).Length / 1MB, 2)
        FlyoutMs    = $startup
        ReopenMs    = $reopen
        IdleReopenMs = $idle
        FlyoutRamMB = $open.MB
        FlyoutProcs = $open.Count
        TrayRamMB   = $tray.MB
        TrayProcs   = $tray.Count
    }
}
$focusSink.Close()
$results | Format-Table -AutoSize
$results | Export-Csv (Join-Path $dist ($Cold ? 'results-cold.csv' : 'results.csv')) -NoTypeInformation
