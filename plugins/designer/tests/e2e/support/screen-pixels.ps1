# Inspect the composed desktop and deliver real clicks to the visible validation window.
param(
    [Parameter(Mandatory)][int] $AppProcessId,
    [ValidateSet('Show', 'Restore', 'Sample', 'Click', 'Offscreen')][string] $Action,
    [int] $Left,
    [int] $Top,
    [int] $Width,
    [int] $Height
)

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class DesktopPixels {
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    struct MouseInput {
        public int X, Y;
        public uint Data, Flags, Time;
        public UIntPtr ExtraInfo;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct Input { public uint Type; public MouseInput Mouse; }
    [DllImport("user32")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
    [DllImport("user32")] static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32")] static extern uint SendInput(uint count, Input[] input, int size);
    [DllImport("user32")] static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32")] static extern bool GetWindowRect(IntPtr window, out Rect rectangle);
    [DllImport("user32")] public static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr window, StringBuilder name, int count);
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

    static Point ScreenPoint(IntPtr window, int x, int y) {
        var point = new Point { X = x, Y = y };
        if (window == IntPtr.Zero || !ClientToScreen(window, ref point)) {
            throw new InvalidOperationException("Cannot locate the validation window.");
        }
        return point;
    }

    public static string Sample(IntPtr window, int x, int y) {
        var point = ScreenPoint(window, x, y);
        // GA_ROOT. Never sample a point covered by another application.
        var covering = GetAncestor(WindowFromPoint(point), 2);
        if (covering != window) {
            Rect rect;
            GetWindowRect(window, out rect);
            var name = new StringBuilder(128);
            GetClassName(covering, name, name.Capacity);
            return String.Format("covered by {6}/{7} at ({0},{1}) in window ({2},{3})..({4},{5})", point.X, point.Y,
                rect.Left, rect.Top, rect.Right, rect.Bottom, covering, name);
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

    static void Move(IntPtr window, int x, int y) {
        var point = ScreenPoint(window, x, y);
        if (GetAncestor(WindowFromPoint(point), 2) != window || !SetCursorPos(point.X, point.Y)) {
            throw new InvalidOperationException("The validation window does not own this pointer position.");
        }
    }

    static void Button(uint flag) {
        var input = new Input { Mouse = new MouseInput { Flags = flag } };
        if (SendInput(1, new[] { input }, Marshal.SizeOf(typeof(Input))) != 1) {
            throw new InvalidOperationException("Cannot deliver the test mouse button event.");
        }
    }

    public static void Click(IntPtr window, int x, int y) {
        Point original;
        if (!GetCursorPos(out original)) throw new InvalidOperationException("Cannot locate the pointer.");
        try {
            Move(window, x, y);
            try { Button(2); }
            finally { Button(4); }
        } finally {
            SetCursorPos(original.X, original.Y);
        }
    }
}
'@

# The DOM coordinates arrive in device pixels, including on mixed-DPI desktops.
[void][DesktopPixels]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$window = (Get-Process -Id $AppProcessId).MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'The validation window is missing.' }
if ($Action -eq 'Offscreen') {
    # MONITOR_DEFAULTTONULL reports no monitor when the real window lies off all of them.
    [DesktopPixels]::MonitorFromWindow($window, 0) -eq [IntPtr]::Zero
    return
}
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
if ($Action -eq 'Click') {
    [DesktopPixels]::Click($window, $Left, $Top)
    return
}
[void][DesktopPixels]::DwmFlush()
$right = $Left + $Width
$bottom = $Top + $Height
$middleX = $Left + ($Width -shr 1)
$middleY = $Top + ($Height -shr 1)
[DesktopPixels]::Sample($window, $Left + 2, $Top + 2)
[DesktopPixels]::Sample($window, $right - 2, $bottom - 2)
[DesktopPixels]::Sample($window, $Left - 2, $middleY)
[DesktopPixels]::Sample($window, $right + 2, $middleY)
[DesktopPixels]::Sample($window, $middleX, $Top - 2)
[DesktopPixels]::Sample($window, $middleX, $bottom + 2)
