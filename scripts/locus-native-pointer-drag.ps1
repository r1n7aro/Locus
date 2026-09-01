param(
  [Parameter(Mandatory = $true)][int]$StartX,
  [Parameter(Mandatory = $true)][int]$StartY,
  [Parameter(Mandatory = $true)][int]$EndX,
  [Parameter(Mandatory = $true)][int]$EndY,
  [int]$DurationMs = 320,
  [int]$HoldBeforeReleaseMs = 0,
  [string]$RuntimeRoot = "",
  [switch]$MoveOnly,
  [switch]$HoldAtEnd
)

Add-Type @"
using System;
using System.Runtime.InteropServices;

public static class LocusPointerInput {
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X; public int Y; }

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }

    public delegate bool EnumWindowsCallback(IntPtr windowHandle, IntPtr state);

    [StructLayout(LayoutKind.Sequential)]
    public struct MouseInput {
        public int Dx;
        public int Dy;
        public uint MouseData;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Explicit)]
    public struct InputUnion {
        [FieldOffset(0)] public MouseInput Mouse;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct Input {
        public uint Type;
        public InputUnion Data;
    }

    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint SendInput(uint count, Input[] inputs, int size);

    [DllImport("user32.dll")]
    public static extern bool SetProcessDpiAwarenessContext(IntPtr value);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr windowHandle);

    [DllImport("user32.dll")]
    public static extern bool BringWindowToTop(IntPtr windowHandle);

    [DllImport("user32.dll")]
    public static extern void SwitchToThisWindow(IntPtr windowHandle, bool altTab);

    [DllImport("user32.dll")]
    public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extraInfo);

    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(IntPtr windowHandle, IntPtr insertAfter, int x, int y, int width, int height, uint flags);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    public static extern bool GetCursorPos(out Point point);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr state);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr windowHandle, out uint processId);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr windowHandle, out Rect rect);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr windowHandle);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr windowHandle, System.Text.StringBuilder className, int maximumLength);

    [DllImport("user32.dll")]
    public static extern IntPtr WindowFromPoint(Point point);

    [DllImport("user32.dll")]
    public static extern IntPtr GetAncestor(IntPtr windowHandle, uint flags);

    [DllImport("kernel32.dll")]
    public static extern uint GetCurrentThreadId();

    [DllImport("user32.dll")]
    public static extern bool AttachThreadInput(uint sourceThreadId, uint targetThreadId, bool attach);

    [DllImport("user32.dll")]
    public static extern IntPtr SetActiveWindow(IntPtr windowHandle);

    [DllImport("user32.dll")]
    public static extern IntPtr SetFocus(IntPtr windowHandle);

    [DllImport("user32.dll")]
    public static extern bool ShowWindowAsync(IntPtr windowHandle, int command);

    public static void ForceForeground(IntPtr windowHandle) {
        IntPtr foreground = GetForegroundWindow();
        uint ignored;
        uint currentThread = GetCurrentThreadId();
        uint targetThread = GetWindowThreadProcessId(windowHandle, out ignored);
        uint foregroundThread = foreground == IntPtr.Zero
            ? 0
            : GetWindowThreadProcessId(foreground, out ignored);
        bool attachedTarget = targetThread != 0 && targetThread != currentThread
            && AttachThreadInput(currentThread, targetThread, true);
        bool attachedForeground = foregroundThread != 0 && foregroundThread != currentThread
            && foregroundThread != targetThread
            && AttachThreadInput(currentThread, foregroundThread, true);
        ShowWindowAsync(windowHandle, 9);
        BringWindowToTop(windowHandle);
        SetForegroundWindow(windowHandle);
        SetActiveWindow(windowHandle);
        SetFocus(windowHandle);
        if (attachedForeground) AttachThreadInput(currentThread, foregroundThread, false);
        if (attachedTarget) AttachThreadInput(currentThread, targetThread, false);
    }

    public static IntPtr FindVisibleWindowAtPoint(int processId, int x, int y) {
        IntPtr result = IntPtr.Zero;
        long smallestArea = long.MaxValue;
        EnumWindows((windowHandle, state) => {
            uint ownerProcessId;
            GetWindowThreadProcessId(windowHandle, out ownerProcessId);
            if (ownerProcessId != (uint)processId || !IsWindowVisible(windowHandle)) return true;
            Rect rect;
            if (!GetWindowRect(windowHandle, out rect)) return true;
            if (x < rect.Left || x >= rect.Right || y < rect.Top || y >= rect.Bottom) return true;
            long area = (long)(rect.Right - rect.Left) * (rect.Bottom - rect.Top);
            if (area < smallestArea) {
                smallestArea = area;
                result = windowHandle;
            }
            return true;
        }, IntPtr.Zero);
        return result;
    }

    public const uint LEFT_DOWN = 0x0002;
    public const uint LEFT_UP = 0x0004;

    public static uint SendMouse(uint flags) {
        Input input = new Input();
        input.Type = 0;
        input.Data.Mouse = new MouseInput { Flags = flags, ExtraInfo = UIntPtr.Zero };
        return SendInput(1, new Input[] { input }, Marshal.SizeOf(typeof(Input)));
    }
}
"@

[void][LocusPointerInput]::SetProcessDpiAwarenessContext([IntPtr](-4))
if ($MoveOnly) {
  [void][LocusPointerInput]::SetCursorPos($EndX, $EndY)
  $movedPoint = New-Object LocusPointerInput+Point
  [void][LocusPointerInput]::GetCursorPos([ref]$movedPoint)
  [ordered]@{
    moveOnly = $true
    finalX = $movedPoint.X
    finalY = $movedPoint.Y
  } | ConvertTo-Json -Compress
  exit 0
}
if ($HoldAtEnd) {
  [void][LocusPointerInput]::SetCursorPos($EndX, $EndY)
  $downSent = [LocusPointerInput]::SendMouse([LocusPointerInput]::LEFT_DOWN)
  Start-Sleep -Milliseconds ([Math]::Max(100, $DurationMs))
  $upSent = [LocusPointerInput]::SendMouse([LocusPointerInput]::LEFT_UP)
  [ordered]@{
    holdAtEnd = $true
    downSent = $downSent
    upSent = $upSent
  } | ConvertTo-Json -Compress
  exit 0
}
$requestedWindowHandle = [IntPtr]::Zero
$requestedWindowRect = New-Object LocusPointerInput+Rect
$topmostSucceeded = $false
$temporarilyMinimizedHandle = [IntPtr]::Zero
if ($RuntimeRoot) {
  $runtimeNeedle = [System.IO.Path]::GetFullPath($RuntimeRoot)
  $locusProcess = Get-CimInstance Win32_Process -Filter "name='locus.exe'" |
    Where-Object { $_.CommandLine -and $_.CommandLine.IndexOf($runtimeNeedle, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 } |
    Select-Object -First 1
  if (-not $locusProcess) {
    $logPath = Join-Path $runtimeNeedle "logs\locus.log"
    if (Test-Path -LiteralPath $logPath) {
      $sdkMatch = Select-String -LiteralPath $logPath -Pattern 'listening on http://127\.0\.0\.1:(\d+)/sdk' |
        Select-Object -Last 1
      if ($sdkMatch) {
        $sdkPort = [int]$sdkMatch.Matches[0].Groups[1].Value
        $listener = Get-NetTCPConnection -State Listen -LocalPort $sdkPort -ErrorAction SilentlyContinue |
          Select-Object -First 1
        if ($listener) {
          $locusProcess = Get-CimInstance Win32_Process -Filter "ProcessId=$($listener.OwningProcess)" |
            Where-Object { $_.Name -ieq 'locus.exe' } |
            Select-Object -First 1
        }
      }
    }
  }
  if ($locusProcess) {
    $windowHandle = [LocusPointerInput]::FindVisibleWindowAtPoint($locusProcess.ProcessId, $StartX, $StartY)
    if ($windowHandle -ne [IntPtr]::Zero) {
      $requestedWindowHandle = $windowHandle
      [void][LocusPointerInput]::GetWindowRect($windowHandle, [ref]$requestedWindowRect)
      [LocusPointerInput]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)
      [LocusPointerInput]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
      [LocusPointerInput]::SwitchToThisWindow($windowHandle, $true)
      $topmostSucceeded = [LocusPointerInput]::SetWindowPos($windowHandle, [IntPtr](-1), 0, 0, 0, 0, 0x0043)
      [void][LocusPointerInput]::BringWindowToTop($windowHandle)
      [void][LocusPointerInput]::SetForegroundWindow($windowHandle)
      [LocusPointerInput]::ForceForeground($windowHandle)
      Start-Sleep -Milliseconds 120
    }
  }
}
if ($requestedWindowHandle -eq [IntPtr]::Zero) {
  $requestedPoint = New-Object LocusPointerInput+Point
  $requestedPoint.X = $StartX
  $requestedPoint.Y = $StartY
  $childAtPoint = [LocusPointerInput]::WindowFromPoint($requestedPoint)
  if ($childAtPoint -ne [IntPtr]::Zero) {
    $rootAtPoint = [LocusPointerInput]::GetAncestor($childAtPoint, 2)
    if ($rootAtPoint -ne [IntPtr]::Zero) {
      $requestedWindowHandle = $rootAtPoint
      [void][LocusPointerInput]::GetWindowRect($rootAtPoint, [ref]$requestedWindowRect)
      [LocusPointerInput]::ForceForeground($rootAtPoint)
      Start-Sleep -Milliseconds 120
    }
  }
}
[void][LocusPointerInput]::SetCursorPos($StartX, $StartY)
Start-Sleep -Milliseconds 80
$startPoint = New-Object LocusPointerInput+Point
$startPoint.X = $StartX
$startPoint.Y = $StartY
$windowAtStart = [LocusPointerInput]::WindowFromPoint($startPoint)
if ($requestedWindowHandle -ne [IntPtr]::Zero -and $windowAtStart -ne $requestedWindowHandle) {
  $foregroundBeforeMinimize = [LocusPointerInput]::GetForegroundWindow()
  if ($windowAtStart -eq $foregroundBeforeMinimize) {
    $temporarilyMinimizedHandle = $foregroundBeforeMinimize
    [void][LocusPointerInput]::ShowWindowAsync($temporarilyMinimizedHandle, 6)
    Start-Sleep -Milliseconds 180
    [void][LocusPointerInput]::SetWindowPos($requestedWindowHandle, [IntPtr](-1), 0, 0, 0, 0, 0x0043)
    [void][LocusPointerInput]::SetCursorPos($StartX, $StartY)
    Start-Sleep -Milliseconds 80
    $windowAtStart = [LocusPointerInput]::WindowFromPoint($startPoint)
  }
}
$downSent = [LocusPointerInput]::SendMouse([LocusPointerInput]::LEFT_DOWN)

$steps = [Math]::Max(18, [Math]::Ceiling($DurationMs / 12))
for ($index = 1; $index -le $steps; $index++) {
  $unit = $index / $steps
  $eased = $unit * $unit * (3 - 2 * $unit)
  $x = [Math]::Round($StartX + (($EndX - $StartX) * $eased))
  $y = [Math]::Round($StartY + (($EndY - $StartY) * $eased))
  [void][LocusPointerInput]::SetCursorPos($x, $y)
  Start-Sleep -Milliseconds ([Math]::Max(6, [Math]::Floor($DurationMs / $steps)))
}

Start-Sleep -Milliseconds 80
if ($HoldBeforeReleaseMs -gt 0) {
  Start-Sleep -Milliseconds $HoldBeforeReleaseMs
}
$upSent = [LocusPointerInput]::SendMouse([LocusPointerInput]::LEFT_UP)
if ($requestedWindowHandle -ne [IntPtr]::Zero) {
  [void][LocusPointerInput]::SetWindowPos($requestedWindowHandle, [IntPtr](-2), 0, 0, 0, 0, 0x0043)
}

$finalPoint = New-Object LocusPointerInput+Point
[void][LocusPointerInput]::GetCursorPos([ref]$finalPoint)
$foregroundAfterDrag = [LocusPointerInput]::GetForegroundWindow()
$windowAtStartProcessId = 0
[void][LocusPointerInput]::GetWindowThreadProcessId($windowAtStart, [ref]$windowAtStartProcessId)
$windowAtStartClass = New-Object System.Text.StringBuilder 256
[void][LocusPointerInput]::GetClassName($windowAtStart, $windowAtStartClass, $windowAtStartClass.Capacity)
if ($temporarilyMinimizedHandle -ne [IntPtr]::Zero) {
  [void][LocusPointerInput]::ShowWindowAsync($temporarilyMinimizedHandle, 9)
}
[ordered]@{
  requestedWindowHandle = $requestedWindowHandle.ToInt64()
  sourceWindowRect = [ordered]@{
    left = $requestedWindowRect.Left
    top = $requestedWindowRect.Top
    right = $requestedWindowRect.Right
    bottom = $requestedWindowRect.Bottom
  }
  windowAtStart = $windowAtStart.ToInt64()
  windowAtStartProcessId = $windowAtStartProcessId
  windowAtStartClass = $windowAtStartClass.ToString()
  topmostSucceeded = $topmostSucceeded
  foregroundWindowHandle = $foregroundAfterDrag.ToInt64()
  restoredWindowHandle = $temporarilyMinimizedHandle.ToInt64()
  downSent = $downSent
  upSent = $upSent
  finalX = $finalPoint.X
  finalY = $finalPoint.Y
} | ConvertTo-Json -Compress
