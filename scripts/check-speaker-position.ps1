#Requires -Version 7
<#
.SYNOPSIS
    Checks the speaker artwork, mouse target and tooltip across a real Quick Settings dismissal.
.DESCRIPTION
    Run one Audio Bartender instance with speaker position on. Start this script with
    Quick Settings closed, then follow the prompts. The probe only observes the desktop.
#>
param([switch]$LanguagePicker)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SpeakerProbe {
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32")] public static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    public static uint Pid(IntPtr hwnd) {
        uint pid;
        GetWindowThreadProcessId(hwnd, out pid);
        return pid;
    }
    [StructLayout(LayoutKind.Sequential)] struct IconId { public int size; public IntPtr hwnd; public uint id; public Guid guid; }
    [DllImport("shell32")] static extern int Shell_NotifyIconGetRect(ref IconId id, out Rect rect);
    public static bool HasFallback(string path, uint pid) {
        ulong hash = 0xcbf29ce484222325UL;
        foreach (byte b in System.Text.Encoding.UTF8.GetBytes(path.ToLowerInvariant()))
            hash = unchecked((hash ^ b) * 0x100000001b3UL);
        string hex = hash.ToString("x16");
        IntPtr tray = IntPtr.Zero;
        do { tray = FindWindowEx(IntPtr.Zero, tray, "AudioBartender.Tray", null); }
        while (tray != IntPtr.Zero && Pid(tray) != pid);
        var icon = new IconId { size = Marshal.SizeOf(typeof(IconId)), hwnd = tray,
            guid = new Guid("5f3a9c1e-7b42-4d08-" + hex.Substring(0,4) + "-" + hex.Substring(4)) };
        Rect r;
        if (Shell_NotifyIconGetRect(ref icon, out r) >= 0 && r.Right > r.Left) return true;
        icon.guid = Guid.Empty;
        return Shell_NotifyIconGetRect(ref icon, out r) >= 0 && r.Right > r.Left;
    }
    public static bool TooltipVisible(uint pid) {
        foreach (string cls in new[] { "AudioBartender.Card", "tooltips_class32" }) {
            IntPtr hwnd = IntPtr.Zero;
            while ((hwnd = FindWindowEx(IntPtr.Zero, hwnd, cls, null)) != IntPtr.Zero)
                if (Pid(hwnd) == pid && IsWindowVisible(hwnd)) return true;
        }
        return false;
    }
}
'@

function PanelHasFocus {
    $owner = [SpeakerProbe]::Pid([SpeakerProbe]::GetForegroundWindow())
    (Get-Process -Id $owner -ErrorAction SilentlyContinue).ProcessName -in 'ShellHost', 'ShellExperienceHost'
}

# Threshold against the tile's own background so hovering can change the tint.
function Read-Glyph($rect) {
    $size = $rect.Right - $rect.Left
    $bitmap = [Drawing.Bitmap]::new($size, $size)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $background = $bitmap.GetPixel([int]($size / 2), 1).GetBrightness()
        $inset = [int]($size / 6)
        for ($y = $inset; $y -lt $size - $inset; $y++) {
            for ($x = $inset; $x -lt $size - $inset; $x++) {
                [Math]::Abs($bitmap.GetPixel($x, $y).GetBrightness() - $background) -gt 0.22
            }
        }
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

$previous = [SpeakerProbe]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
try {
    $bar = [SpeakerProbe]::FindWindowEx(0, 0, 'Shell_TrayWnd', $null)
    $slot = [SpeakerProbe]::FindWindowEx($bar, 0, 'AudioBartender.SpeakerSlot', 'Audio Bartender volume')
    $rect = [SpeakerProbe+Rect]::new()
    if (!$slot -or ![SpeakerProbe]::GetWindowRect($slot, [ref]$rect) -or $rect.Right -le $rect.Left) {
        throw 'Enable speaker position and wait for the custom icon to appear before starting the probe.'
    }
    $point = [SpeakerProbe+Point]::new()
    $point.X = [int](($rect.Left + $rect.Right) / 2)
    $point.Y = [int](($rect.Top + $rect.Bottom) / 2)
    if ([SpeakerProbe]::WindowFromPoint($point) -ne $slot -or (!$LanguagePicker -and (PanelHasFocus))) {
        throw 'Start with the speaker visible and Quick Settings closed.'
    }
    $expected = @(Read-Glyph $rect)
    if ($expected -notcontains $true -or $expected -notcontains $false) { throw 'Expected a visible speaker glyph.' }
    $appId = [SpeakerProbe]::Pid($slot)
    if ($LanguagePicker) {
        $path = (Get-Process -Id $appId).Path
        Write-Host 'Click the language indicator and leave the picker open for three seconds.'
        $deadline = [datetime]::UtcNow.AddSeconds(30)
        while ([SpeakerProbe]::WindowFromPoint($point) -eq $slot) {
            if ([datetime]::UtcNow -gt $deadline) { throw 'The picker never covered the speaker.' }
            Start-Sleep -Milliseconds 40
        }
        $elapsed = [Diagnostics.Stopwatch]::StartNew()
        do {
            if ([SpeakerProbe]::WindowFromPoint($point) -eq $slot) { throw 'Picker closed too soon.' }
            if ([SpeakerProbe]::HasFallback($path, $appId)) { throw 'Fallback appeared under the open picker.' }
            Start-Sleep -Milliseconds 100
        } while ($elapsed.Elapsed.TotalSeconds -lt 3)
        Write-Host 'Close the picker and hover the speaker.'
        $deadline = [datetime]::UtcNow.AddSeconds(30)
        while ([SpeakerProbe]::WindowFromPoint($point) -ne $slot -or ![SpeakerProbe]::TooltipVisible($appId)) {
            if ([datetime]::UtcNow -gt $deadline) { throw 'Speaker or tooltip did not recover.' }
            Start-Sleep -Milliseconds 100
        }
        if ([SpeakerProbe]::HasFallback($path, $appId)) { throw 'Fallback visible after dismissal.' }
        Write-Host 'Passed: no duplicate during the picker, speaker and tooltip working after dismissal.'
        return
    }
    Write-Host 'Click Wi-Fi or battery to open Quick Settings.'
    $deadline = [datetime]::UtcNow.AddSeconds(30)
    while (!(PanelHasFocus)) {
        if ([datetime]::UtcNow -gt $deadline) { throw 'Quick Settings never opened.' }
        Start-Sleep -Milliseconds 40
    }
    Write-Host 'Click the same button to close it, then hover the speaker and leave the pointer still.'
    $deadline = [datetime]::UtcNow.AddSeconds(30)
    while (PanelHasFocus) {
        if ([datetime]::UtcNow -gt $deadline) { throw 'Quick Settings never closed.' }
        Start-Sleep -Milliseconds 40
    }
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    do {
        if ([SpeakerProbe]::WindowFromPoint($point) -ne $slot) { throw "Lost the speaker hit target after $elapsed." }
        $glyph = @(Read-Glyph $rect)
        $changed = 0
        for ($i = 0; $i -lt $expected.Length; $i++) {
            if ($glyph[$i] -ne $expected[$i]) { $changed++ }
        }
        if ($changed -gt $expected.Length * 0.15) { throw "Speaker artwork changed after $elapsed." }
        if ($elapsed.Elapsed.TotalSeconds -gt 5 -and ![SpeakerProbe]::TooltipVisible($appId)) {
            throw "Audio Bartender's tooltip is missing after $elapsed."
        }
        Start-Sleep -Milliseconds 100
    } while ($elapsed.Elapsed.TotalSeconds -lt 32)
    Write-Host 'Passed: speaker artwork, hit target and hover tooltip survived the dismissal.'
} finally {
    [void][SpeakerProbe]::SetThreadDpiAwarenessContext($previous)
}
