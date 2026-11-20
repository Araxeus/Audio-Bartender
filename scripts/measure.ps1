# Builds both front ends, copies them to dist/, then measures each in the tray and
# with its flyout open, and screenshots the flyout.
#
#   pwsh scripts/measure.ps1            # build + measure
#   pwsh scripts/measure.ps1 -NoBuild
#
# RAM is the private working set (Task Manager's Memory column) summed over the
# process tree, so WebView2 helpers count toward Tauri.

param([switch]$NoBuild, [int]$SettleSeconds = 4)

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

    public static RECT Frame(IntPtr h) {
        RECT r; DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT)));
        return r;
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

function Save-Shot($rect, [string]$path) {
    $bmp = New-Object System.Drawing.Bitmap ($rect.R - $rect.L), ($rect.B - $rect.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.L, $rect.T, 0, 0, $bmp.Size)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
}

$results = foreach ($app in 'gpui', 'tauri') {
    $exe = Join-Path $root "target/release/audio_bartender_$app.exe"
    try { Copy-Item $exe (Join-Path $dist "audio_bartender_$app.exe") -Force }
    catch { Write-Warning "dist/audio_bartender_$app.exe is in use; not replaced" }

    # Tray only: what the app costs while you're not looking at it.
    $p = Start-Process $exe -ArgumentList '--hidden', '--dev' -PassThru
    Start-Sleep -Seconds ($SettleSeconds + 2)
    $tray = Get-PrivateMB $p.Id
    Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 800

    # Flyout open, pinned so it stays up while measured.
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $exe -ArgumentList '--pinned', '--dev' -PassThru
    $hwnd = [IntPtr]::Zero
    while ($sw.Elapsed.TotalSeconds -lt 20 -and $hwnd -eq [IntPtr]::Zero) {
        Start-Sleep -Milliseconds 10
        $hwnd = [W]::FindMain([uint32]$p.Id)
    }
    $startup = [int]$sw.Elapsed.TotalMilliseconds
    if ($hwnd -eq [IntPtr]::Zero) { Write-Warning "$app : no flyout"; Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }; continue }

    Start-Sleep -Seconds $SettleSeconds
    $open = Get-PrivateMB $p.Id
    Save-Shot ([W]::Frame($hwnd)) (Join-Path $dist "$app.png")
    Get-Tree $p.Id | ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 800

    [pscustomobject]@{
        App         = $app
        ExeMB       = [math]::Round((Get-Item $exe).Length / 1MB, 2)
        FlyoutMs    = $startup
        FlyoutRamMB = $open.MB
        FlyoutProcs = $open.Count
        TrayRamMB   = $tray.MB
        TrayProcs   = $tray.Count
    }
}
$results | Format-Table -AutoSize
$results | Export-Csv (Join-Path $dist 'results.csv') -NoTypeInformation
