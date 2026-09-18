param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$ReportPath,
    [ValidateRange(1, 20)][int]$Iterations = 3
)

# Native regression test. Each run starts its own isolated profile and only
# changes windows belonging to that new process. No desktop input is injected.
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This test requires Windows.' }
$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$runtimeRoot = Join-Path 'E:\LocusTemp' ('locus-resize-test-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $runtimeRoot)
$stdoutPath = Join-Path $runtimeRoot 'stdout.log'
$stderrPath = Join-Path $runtimeRoot 'stderr.log'
if (-not $ReportPath) { $ReportPath = Join-Path $runtimeRoot 'resize-report.json' }
$ReportPath = [IO.Path]::GetFullPath($ReportPath)

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class LocusResizeRegression {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left,top,right,bottom; }
  private delegate bool EnumWindowCallback(IntPtr hwnd,IntPtr state);
  [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowCallback callback,IntPtr state);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hwnd,int command);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd,out Rect rect);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent,IntPtr after,string cls,string name);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd,IntPtr after,int x,int y,int width,int height,uint flags);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
  public static IntPtr FindMain(uint processId) {
    IntPtr result=IntPtr.Zero;
    EnumWindows((hwnd,state)=>{uint owner;GetWindowThreadProcessId(hwnd,out owner);
      if(owner==processId&&FindWindowExW(hwnd,IntPtr.Zero,"TAURI_DRAG_RESIZE_BORDERS",null)!=IntPtr.Zero){result=hwnd;return false;}
      return true;},IntPtr.Zero);
    return result;
  }
  [DllImport("user32.dll")] public static extern int GetWindowRgn(IntPtr hwnd,IntPtr region);
  [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int left,int top,int right,int bottom);
  [DllImport("gdi32.dll")] static extern int GetRgnBox(IntPtr region,out Rect rect);
  [DllImport("gdi32.dll")] static extern int CombineRgn(IntPtr target,IntPtr first,IntPtr second,int mode);
  [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
  [DllImport("user32.dll")] static extern int GetSystemMetricsForDpi(int metric,uint dpi);
  [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr region);
  [DllImport("user32.dll")] static extern IntPtr SendMessageTimeoutW(IntPtr hwnd,uint message,IntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
  public static object Snapshot(IntPtr hwnd) {
    var border=FindWindowExW(hwnd,IntPtr.Zero,"TAURI_DRAG_RESIZE_BORDERS",null);
    Rect client,window,bounds; GetClientRect(hwnd,out client);GetWindowRect(hwnd,out window);GetWindowRect(border,out bounds);
    var region=CreateRectRgn(0,0,0,0);
    try {
      int regionType=GetWindowRgn(border,region);Rect regionBounds;GetRgnBox(region,out regionBounds);
      int width=client.right-client.left,height=client.bottom-client.top;
      uint dpi=GetDpiForWindow(hwnd);
      int insetX=GetSystemMetricsForDpi(32,dpi)+1,insetY=GetSystemMetricsForDpi(33,dpi)+1;
      // Test the entire interior, including narrow stale border strips; a
      // point grid would miss strips left over from arbitrary prior sizes.
      var interior=CreateRectRgn(insetX,insetY,width-insetX,height-insetY);
      var overlap=CreateRectRgn(0,0,0,0);
      bool interiorCovered;
      try {interiorCovered=bounds.right>bounds.left&&bounds.bottom>bounds.top&&CombineRgn(overlap,region,interior,1)>1;}
      finally {DeleteObject(interior);DeleteObject(overlap);}
      UIntPtr result;bool responsive=SendMessageTimeoutW(hwnd,0,IntPtr.Zero,IntPtr.Zero,3,1000,out result)!=IntPtr.Zero;
      long topResizeHit=0;
      if(!IsZoomed(hwnd)&&border!=IntPtr.Zero) {
        int x=(bounds.left+bounds.right)/2,y=bounds.top+1;
        if(SendMessageTimeoutW(border,0x84,IntPtr.Zero,new IntPtr((y<<16)|(x&65535)),3,1000,out result)!=IntPtr.Zero)
          topResizeHit=unchecked((long)result.ToUInt64());
      }
      return new {maximized=IsZoomed(hwnd),responsive,borderHwnd=border.ToInt64(),clientWidth=width,clientHeight=height,
        borderWidth=bounds.right-bounds.left,borderHeight=bounds.bottom-bounds.top,regionType,regionBounds,interiorCovered,topResizeHit,window};
    } finally {DeleteObject(region);}
  }
}
'@

function Wait-Until([scriptblock]$Predicate, [string]$Description) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 15) {
        if (& $Predicate) { return }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out: $Description"
}

$snapshots = [Collections.Generic.List[object]]::new()
$failures = [Collections.Generic.List[string]]::new()
$app = $null
$manifest = $null
function Assert-Window([string]$Phase, [bool]$Maximized) {
    # SetWindowPos can dispatch another native message after returning.
    Start-Sleep -Milliseconds 200
    $snapshot = [LocusResizeRegression]::Snapshot($script:mainHwnd)
    $snapshots.Add([pscustomobject]@{phase=$Phase;state=$snapshot})
    if (-not $snapshot.responsive) { $failures.Add("${Phase}: window is unresponsive") }
    if ($snapshot.maximized -ne $Maximized) { $failures.Add("${Phase}: incorrect maximize state") }
    if ($Maximized) {
        if ($snapshot.borderWidth -gt 0 -and $snapshot.borderHeight -gt 0) {
            $failures.Add("${Phase}: maximized resize overlay is $($snapshot.borderWidth)x$($snapshot.borderHeight), expected zero area")
        }
    } else {
        if ($snapshot.borderWidth -ne $snapshot.clientWidth -or $snapshot.borderHeight -ne $snapshot.clientHeight) {
            $failures.Add("${Phase}: restored resize overlay does not match the client rectangle")
        }
        if ($snapshot.interiorCovered) { $failures.Add("${Phase}: resize region covers interior content") }
        if ($snapshot.topResizeHit -ne 12) { $failures.Add("${Phase}: top resize edge no longer returns HTTOP") }
    }
}

try {
    $app = Start-Process -FilePath $executablePath -ArgumentList @(
        '--locus-runtime-root', $runtimeRoot, '--locus-skip-onboarding'
    ) -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    Wait-Until { $app.Refresh(); $app.HasExited -or $app.MainWindowHandle -ne [IntPtr]::Zero } 'isolated main window'
    if ($app.HasExited) { throw "Isolated application exited: $(Get-Content -LiteralPath $stderrPath -Tail 10 | Out-String)" }
    Wait-Until { [LocusResizeRegression]::FindMain($app.Id) -ne [IntPtr]::Zero } 'main window with a Tauri resize overlay'
    $script:mainHwnd = [LocusResizeRegression]::FindMain($app.Id)
    [uint32]$ownerPid = 0
    [void][LocusResizeRegression]::GetWindowThreadProcessId($script:mainHwnd, [ref]$ownerPid)
    if ($ownerPid -ne $app.Id) { throw 'Window owner does not match the isolated process.' }
    $manifestLine = Get-Content -LiteralPath $stdoutPath | Where-Object { $_.StartsWith('LOCUS_RUNTIME_JSON ') } | Select-Object -First 1
    if ($manifestLine) {
        $manifest = $manifestLine.Substring('LOCUS_RUNTIME_JSON '.Length) | ConvertFrom-Json
    } else {
        # Windows-subsystem release binaries need not have a console stdout.
        # Verify their launch arguments and isolated database before emitting
        # the same manifest from this launcher, as run-tauri.mjs does.
        $launched = Get-CimInstance Win32_Process -Filter "ProcessId=$($app.Id)"
        if (-not $launched.CommandLine.Contains($runtimeRoot)) { throw 'Missing isolated runtime argument.' }
        Wait-Until { Test-Path -LiteralPath (Join-Path $runtimeRoot 'database\locus.db') } 'isolated database'
        $manifest = [pscustomobject]@{
            runtimeRoot=$runtimeRoot;databaseDir=(Join-Path $runtimeRoot 'database');databaseFile=(Join-Path $runtimeRoot 'database\locus.db')
            configDir=(Join-Path $runtimeRoot 'config');logDir=(Join-Path $runtimeRoot 'logs');logFile=(Join-Path $runtimeRoot 'logs\locus.log')
            workspace=(Join-Path $runtimeRoot 'workspace');webviewDataDir=(Join-Path $runtimeRoot 'webview');skipOnboarding=$true
        }
        $manifestLine = 'LOCUS_RUNTIME_JSON ' + ($manifest | ConvertTo-Json -Compress)
    }
    if ($manifest.runtimeRoot -ne $runtimeRoot) { throw 'Unexpected runtime root.' }
    Write-Output $manifestLine
    Write-Output "LOCUS_RESIZE_TEST_PID $($app.Id)"
    Start-Sleep -Seconds 2

    for ($cycle = 0; $cycle -lt $Iterations; $cycle++) {
        [void][LocusResizeRegression]::ShowWindowAsync($script:mainHwnd,9)
        Wait-Until { -not [LocusResizeRegression]::IsZoomed($script:mainHwnd) } 'restore'
        $width = @(1200,1600,1400)[$cycle % 3]
        $height = @(800,1000,900)[$cycle % 3]
        [void][LocusResizeRegression]::SetWindowPos($script:mainHwnd,[IntPtr]::Zero,120,100,$width,$height,0x4014)
        Assert-Window "cycle-$cycle-restored" $false

        [void][LocusResizeRegression]::ShowWindowAsync($script:mainHwnd,3)
        Wait-Until { [LocusResizeRegression]::IsZoomed($script:mainHwnd) } 'maximize'
        Assert-Window "cycle-$cycle-maximized" $true
        for ($notification = 0; $notification -lt 3; $notification++) {
            [void][LocusResizeRegression]::SetWindowPos($script:mainHwnd,[IntPtr]::Zero,0,0,0,0,0x4017)
            Assert-Window "cycle-$cycle-maximized-noop-$notification" $true
        }
        if ($failures.Count -gt 0) { break }
    }
} catch {
    $failures.Add($_.Exception.Message)
} finally {
    $result = [pscustomobject]@{executable=$executablePath;pid=$app.Id;runtime=$manifest;runtimeRoot=$runtimeRoot;passed=$failures.Count -eq 0;failures=@($failures);snapshots=@($snapshots)}
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ReportPath
    Write-Output "LOCUS_RESIZE_TEST_JSON $($result | ConvertTo-Json -Depth 8 -Compress)"
    if ($app -and -not $app.HasExited) {
        # Only stop the exact process launched above after validating its
        # creation time and unique isolated runtime argument.
        $current = Get-CimInstance Win32_Process -Filter "ProcessId=$($app.Id)"
        if ($current -and $current.ExecutablePath -eq $executablePath -and $current.CommandLine.Contains($runtimeRoot) -and [Math]::Abs(($current.CreationDate - $app.StartTime).TotalSeconds) -lt 2) {
            Stop-Process -Id $app.Id -Force
        }
    }
}
if ($failures.Count -gt 0) { exit 1 }
