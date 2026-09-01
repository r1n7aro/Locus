param(
  [Parameter(Mandatory = $true)][string]$RuntimeRoot,
  [Parameter(Mandatory = $true)][int]$ExpectedWidth,
  [Parameter(Mandatory = $true)][int]$ExpectedHeight,
  [string]$BaselinePath = "",
  [int]$TimeoutMs = 900
)

$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class LocusDragPreviewInspector {
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X; public int Y; }

    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }

    public sealed class Snapshot {
        public long WindowHandle { get; set; }
        public int Left { get; set; }
        public int Top { get; set; }
        public int Width { get; set; }
        public int Height { get; set; }
        public int CursorX { get; set; }
        public int CursorY { get; set; }
    }

    private delegate bool EnumWindowsCallback(IntPtr windowHandle, IntPtr state);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr state);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr windowHandle, out uint processId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr windowHandle, StringBuilder className, int maximumLength);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr windowHandle, out Rect rect);

    [DllImport("user32.dll")]
    public static extern bool GetCursorPos(out Point point);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr windowHandle);

    public static Snapshot Find(int processId, int expectedWidth, int expectedHeight) {
        Snapshot result = null;
        EnumWindows((windowHandle, state) => {
            uint ownerProcessId;
            GetWindowThreadProcessId(windowHandle, out ownerProcessId);
            if (ownerProcessId != (uint)processId || !IsWindowVisible(windowHandle)) return true;
            var className = new StringBuilder(64);
            GetClassName(windowHandle, className, className.Capacity);
            if (!String.Equals(className.ToString(), "Static", StringComparison.OrdinalIgnoreCase)) return true;
            Rect rect;
            if (!GetWindowRect(windowHandle, out rect)) return true;
            int width = rect.Right - rect.Left;
            int height = rect.Bottom - rect.Top;
            if (Math.Abs(width - expectedWidth) > 2 || Math.Abs(height - expectedHeight) > 2) return true;
            Point cursor;
            GetCursorPos(out cursor);
            result = new Snapshot {
                WindowHandle = windowHandle.ToInt64(),
                Left = rect.Left,
                Top = rect.Top,
                Width = width,
                Height = height,
                CursorX = cursor.X,
                CursorY = cursor.Y,
            };
            return false;
        }, IntPtr.Zero);
        return result;
    }
}
"@

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
if (-not $locusProcess) {
  throw "No Locus process owns runtime root $runtimeNeedle"
}

$deadline = [DateTime]::UtcNow.AddMilliseconds([Math]::Max(100, $TimeoutMs))
$snapshot = $null
do {
  $snapshot = [LocusDragPreviewInspector]::Find(
    $locusProcess.ProcessId,
    $ExpectedWidth,
    $ExpectedHeight
  )
  if ($snapshot) { break }
  Start-Sleep -Milliseconds 10
} while ([DateTime]::UtcNow -lt $deadline)

if (-not $snapshot) {
  if (-not $BaselinePath -or -not (Test-Path -LiteralPath $BaselinePath)) {
    throw "Native drag preview ${ExpectedWidth}x${ExpectedHeight} was not found."
  }
  Add-Type -AssemblyName System.Drawing
  Add-Type -AssemblyName System.Windows.Forms
  $screens = [System.Windows.Forms.Screen]::AllScreens
  $virtualLeft = ($screens | ForEach-Object { $_.Bounds.Left } | Measure-Object -Minimum).Minimum
  $virtualTop = ($screens | ForEach-Object { $_.Bounds.Top } | Measure-Object -Minimum).Minimum
  $virtualRight = ($screens | ForEach-Object { $_.Bounds.Right } | Measure-Object -Maximum).Maximum
  $virtualBottom = ($screens | ForEach-Object { $_.Bounds.Bottom } | Measure-Object -Maximum).Maximum
  $baseline = [System.Drawing.Bitmap]::FromFile([System.IO.Path]::GetFullPath($BaselinePath))
  $current = [System.Drawing.Bitmap]::new(
    [int]($virtualRight - $virtualLeft),
    [int]($virtualBottom - $virtualTop)
  )
  $graphics = [System.Drawing.Graphics]::FromImage($current)
  $cursor = New-Object LocusDragPreviewInspector+Point
  [void][LocusDragPreviewInspector]::GetCursorPos([ref]$cursor)
  try {
    $graphics.CopyFromScreen($virtualLeft, $virtualTop, 0, 0, $current.Size)
    $cropLeft = [Math]::Max($virtualLeft, $cursor.X - $ExpectedWidth - 48)
    $cropTop = [Math]::Max($virtualTop, $cursor.Y - $ExpectedHeight - 48)
    $cropRight = [Math]::Min($virtualRight, $cursor.X + $ExpectedWidth + 48)
    $cropBottom = [Math]::Min($virtualBottom, $cursor.Y + $ExpectedHeight + 48)
    $minX = [int]::MaxValue
    $minY = [int]::MaxValue
    $maxX = [int]::MinValue
    $maxY = [int]::MinValue
    for ($screenY = $cropTop; $screenY -lt $cropBottom; $screenY++) {
      for ($screenX = $cropLeft; $screenX -lt $cropRight; $screenX++) {
        $bitmapX = $screenX - $virtualLeft
        $bitmapY = $screenY - $virtualTop
        $before = $baseline.GetPixel($bitmapX, $bitmapY)
        $after = $current.GetPixel($bitmapX, $bitmapY)
        $difference = [Math]::Abs([int]$before.R - [int]$after.R) +
          [Math]::Abs([int]$before.G - [int]$after.G) +
          [Math]::Abs([int]$before.B - [int]$after.B)
        if ($difference -lt 24) { continue }
        $minX = [Math]::Min($minX, $screenX)
        $minY = [Math]::Min($minY, $screenY)
        $maxX = [Math]::Max($maxX, $screenX)
        $maxY = [Math]::Max($maxY, $screenY)
      }
    }
    if ($minX -eq [int]::MaxValue) {
      throw "Chromium drag image pixels were not detected against the baseline."
    }
    $capturePath = Join-Path ([System.IO.Path]::GetDirectoryName([System.IO.Path]::GetFullPath($BaselinePath))) "chromium-native-drag-preview.png"
    $current.Save($capturePath, [System.Drawing.Imaging.ImageFormat]::Png)
    [ordered]@{
      processId = $locusProcess.ProcessId
      windowHandle = 0
      left = $minX
      top = $minY
      width = $maxX - $minX + 1
      height = $maxY - $minY + 1
      cursorX = $cursor.X
      cursorY = $cursor.Y
      pointerOffsetX = $cursor.X - $minX
      pointerOffsetY = $cursor.Y - $minY
      capturePath = $capturePath
      detection = "screen-diff"
    } | ConvertTo-Json -Compress
    exit 0
  } finally {
    $graphics.Dispose()
    $current.Dispose()
    $baseline.Dispose()
  }
}

[ordered]@{
  processId = $locusProcess.ProcessId
  windowHandle = $snapshot.WindowHandle
  left = $snapshot.Left
  top = $snapshot.Top
  width = $snapshot.Width
  height = $snapshot.Height
  cursorX = $snapshot.CursorX
  cursorY = $snapshot.CursorY
  pointerOffsetX = $snapshot.CursorX - $snapshot.Left
  pointerOffsetY = $snapshot.CursorY - $snapshot.Top
} | ConvertTo-Json -Compress
