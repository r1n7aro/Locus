# Read-only helper for locus-input-diagnostics.ts. CDP debug-mode gating and
# process selection belong to that entry point; this helper sends only WM_NULL.
param(
  [Parameter(Mandatory = $true)][int]$LocusProcessId,
  [ValidateRange(1000, 300000)][int]$DurationMs = 15000
)
$ErrorActionPreference = 'Stop'
$locusProcess = Get-Process -Id $LocusProcessId
if ($locusProcess.ProcessName -ne 'locus') { throw 'The selected process is not Locus.' }
$nativeCode = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
public static class LocusNativeInput {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct GUIINFO {
    public int Size; public uint Flags;
    public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret; public RECT CaretRect;
  }
  public delegate bool EnumProc(IntPtr hwnd, IntPtr data);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc fn, IntPtr data);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr hwnd, EnumProc fn, IntPtr data);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", SetLastError=true)] static extern bool GetGUIThreadInfo(uint tid, ref GUIINFO info);
  [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr hwnd);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] static extern bool IsHungAppWindow(IntPtr hwnd);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder value, int count);
  [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hwnd);
  [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr hwnd, uint command);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT point);
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT point);
  [DllImport("user32.dll")] static extern short GetAsyncKeyState(int key);
  [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeoutW(
    IntPtr hwnd, uint msg, UIntPtr wp, IntPtr lp, uint flags, uint timeout, out UIntPtr result);
  static string ClassName(IntPtr hwnd) {
    var text = new StringBuilder(256); GetClassName(hwnd,text,text.Capacity); return text.ToString();
  }
  static object Describe(IntPtr hwnd) {
    if (hwnd == IntPtr.Zero) return null;
    uint pid; var tid = GetWindowThreadProcessId(hwnd,out pid); RECT rect; GetWindowRect(hwnd,out rect);
    return new { hwnd=hwnd.ToInt64(), pid, tid, cls=ClassName(hwnd), rect,
      enabled=IsWindowEnabled(hwnd), visible=IsWindowVisible(hwnd), hung=IsHungAppWindow(hwnd),
      parent=GetParent(hwnd).ToInt64(), owner=GetWindow(hwnd,4).ToInt64() };
  }
  public static object Snapshot(uint pid) {
    var windows = new List<IntPtr>(); var tids = new HashSet<uint>();
    EnumWindows((hwnd,data) => {
      uint owner; GetWindowThreadProcessId(hwnd,out owner);
      if (owner == pid && ClassName(hwnd) == "Tauri Window") {
        windows.Add(hwnd);
        EnumChildWindows(hwnd,(child,unused) => { windows.Add(child); return true; },IntPtr.Zero);
      }
      return true;
    },IntPtr.Zero);
    var descriptions = new List<object>(); var probes = new List<object>(); var probed = new HashSet<uint>();
    foreach (var hwnd in windows) {
      uint owner; var tid = GetWindowThreadProcessId(hwnd,out owner); tids.Add(tid);
      descriptions.Add(Describe(hwnd));
      // Probe each visible input thread once, keeping even a fully hung app bounded.
      if (IsWindowVisible(hwnd) && probed.Count < 8 && probed.Add(tid)) {
        UIntPtr result; var watch = Stopwatch.StartNew();
        var ok = SendMessageTimeoutW(hwnd,0,UIntPtr.Zero,IntPtr.Zero,0x22,100,out result) != IntPtr.Zero;
        probes.Add(new { hwnd=hwnd.ToInt64(), tid, responded=ok, elapsedMs=watch.ElapsedMilliseconds });
      }
    }
    var threads = new List<object>();
    foreach (var tid in tids) {
      var info = new GUIINFO(); info.Size = Marshal.SizeOf(info);
      var ok = GetGUIThreadInfo(tid,ref info);
      threads.Add(new { tid, available=ok, flags=info.Flags, active=Describe(info.Active),
        focus=Describe(info.Focus), capture=Describe(info.Capture),
        menuOwner=Describe(info.MenuOwner), moveSize=Describe(info.MoveSize) });
    }
    POINT point; var cursorAvailable=GetCursorPos(out point);
    return new { atMs=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), pid, cursorAvailable, cursor=point,
      leftDown=(GetAsyncKeyState(1)&0x8000)!=0, rightDown=(GetAsyncKeyState(2)&0x8000)!=0,
      underCursor=cursorAvailable ? Describe(WindowFromPoint(point)) : null,
      foreground=Describe(GetForegroundWindow()), threads, probes, windows=descriptions };
  }
}
'@
Add-Type -TypeDefinition $nativeCode
$captureClock = [Diagnostics.Stopwatch]::StartNew()
while ($captureClock.ElapsedMilliseconds -lt $DurationMs -and -not $locusProcess.HasExited) {
  [LocusNativeInput]::Snapshot($LocusProcessId) | ConvertTo-Json -Depth 9 -Compress
  Start-Sleep -Milliseconds 500
  $locusProcess.Refresh()
}
