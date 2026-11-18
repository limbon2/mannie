# Sample the composed desktop at two points inside the validation window.
# Only this external test process has access to the desktop DC.
param(
    [Parameter(Mandatory)][int] $AppProcessId,
    [ValidateSet('Show', 'Restore', 'Sample')][string] $Action,
    [int] $Left,
    [int] $Top,
    [int] $Width,
    [int] $Height
)

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class DesktopPixels {
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X, Y; }
    [DllImport("user32")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
    [DllImport("user32")] static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32")] static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32", EntryPoint="GetWindowLongW")] static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32")] static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32")] static extern IntPtr GetDC(IntPtr window);
    [DllImport("user32")] static extern int ReleaseDC(IntPtr window, IntPtr dc);
    [DllImport("gdi32")] static extern uint GetPixel(IntPtr dc, int x, int y);
    [DllImport("dwmapi")] public static extern int DwmFlush();

    // Keep the visible test window above other windows throughout the interaction.
    // SWP_NOACTIVATE leaves keyboard focus alone throughout the check.
    public static bool IsTopmost(IntPtr window) { return (GetWindowLong(window, -20) & 8) != 0; }
    public static void Topmost(IntPtr window, bool value) {
        if (!SetWindowPos(window, new IntPtr(value ? -1 : -2), 0, 0, 0, 0, 0x13)) {
            throw new InvalidOperationException("Cannot position the validation window.");
        }
    }

    public static string Sample(IntPtr window, int x, int y) {
        var point = new Point { X = x, Y = y };
        if (window == IntPtr.Zero || !ClientToScreen(window, ref point)) {
            throw new InvalidOperationException("Cannot locate the validation window.");
        }
        // GA_ROOT. Never sample a point covered by another application.
        if (GetAncestor(WindowFromPoint(point), 2) != window) {
            return "covered";
        }
        var dc = GetDC(IntPtr.Zero);
        try {
            uint rgb = GetPixel(dc, point.X, point.Y);
            if (rgb == uint.MaxValue) throw new InvalidOperationException("Cannot read the desktop pixel.");
            return String.Format("{0},{1},{2}", rgb & 255, (rgb >> 8) & 255, (rgb >> 16) & 255);
        } finally {
            ReleaseDC(IntPtr.Zero, dc);
        }
    }
}
'@

# The DOM coordinates arrive in device pixels, including on mixed-DPI desktops.
[void][DesktopPixels]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$window = (Get-Process -Id $AppProcessId).MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'The validation window is missing.' }
if ($Action -eq 'Show') {
    $alreadyTopmost = [DesktopPixels]::IsTopmost($window)
    if (!$alreadyTopmost) { [DesktopPixels]::Topmost($window, $true) }
    $alreadyTopmost
    return
}
if ($Action -eq 'Restore') {
    [DesktopPixels]::Topmost($window, $false)
    return
}
[void][DesktopPixels]::DwmFlush()
[DesktopPixels]::Sample($window, $Left + ($Width -shr 1), $Top + ($Height -shr 1))
[DesktopPixels]::Sample($window, $Left + 2, $Top + 2)
