param(
    [int]$DurationSeconds = 240,
    [int]$SampleIntervalSeconds = 10,
    [string]$OutputRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    return (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
}

function Get-OutputRoot {
    param([string]$ConfiguredOutputRoot)

    if ($ConfiguredOutputRoot -and $ConfiguredOutputRoot.Trim()) {
        if ([System.IO.Path]::IsPathRooted($ConfiguredOutputRoot)) {
            return [System.IO.Path]::GetFullPath($ConfiguredOutputRoot)
        }
        return [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $ConfiguredOutputRoot))
    }

    return (Join-Path (Get-RepoRoot) "tmp\memory-monitor")
}

function Ensure-Directory {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Safe-ToIso {
    param([datetime]$Value)

    return $Value.ToString("o")
}

function Mb {
    param([double]$Bytes)

    return [math]::Round($Bytes / 1MB, 2)
}

function Normalize-String {
    param([object]$Value)

    if ($null -eq $Value) {
        return ""
    }
    return [string]$Value
}

function Or-Zero {
    param([object]$Value)

    if ($null -eq $Value) {
        return 0
    }
    if ($Value -is [string] -and [string]::IsNullOrWhiteSpace($Value)) {
        return 0
    }
    return $Value
}

function Get-WebViewRole {
    param([string]$CommandLine)

    if (-not $CommandLine) {
        return "webview"
    }

    if ($CommandLine -match "--type=renderer") {
        return "webview-renderer"
    }
    if ($CommandLine -match "--type=gpu-process") {
        return "webview-gpu"
    }
    if ($CommandLine -match "--type=crashpad-handler") {
        return "webview-crashpad"
    }
    if ($CommandLine -match "--type=utility") {
        if ($CommandLine -match "--utility-sub-type=([^\s]+)") {
            return "webview-utility:$($Matches[1])"
        }
        return "webview-utility"
    }

    return "webview-browser"
}

function Get-ProcessRole {
    param(
        [string]$Name,
        [string]$CommandLine
    )

    $normalizedName = (Normalize-String $Name).ToLowerInvariant()
    $normalizedCmd = (Normalize-String $CommandLine).ToLowerInvariant()

    switch ($normalizedName) {
        "locus.exe" { return "app-main" }
        "bun.exe" {
            if ($normalizedCmd -match "tauri dev") { return "dev-bun-tauri" }
            if ($normalizedCmd -match "run dev") { return "dev-bun-vite" }
            return "dev-bun"
        }
        "cargo.exe" { return "dev-cargo" }
        "msedgewebview2.exe" { return (Get-WebViewRole -CommandLine $CommandLine) }
        default { return "related-process" }
    }
}

function Test-IsSeedProcess {
    param(
        [string]$Name,
        [string]$CommandLine
    )

    $normalizedName = (Normalize-String $Name).ToLowerInvariant()
    $normalizedCmd = (Normalize-String $CommandLine).ToLowerInvariant()

    if ($normalizedName -eq "locus.exe") {
        return $true
    }
    if ($normalizedName -eq "bun.exe" -and ($normalizedCmd -match "tauri dev" -or $normalizedCmd -match "run dev")) {
        return $true
    }
    if ($normalizedName -eq "cargo.exe" -and ($normalizedCmd -match "locus" -or $normalizedCmd -match "tauri")) {
        return $true
    }
    if ($normalizedName -eq "msedgewebview2.exe" -and ($normalizedCmd -match "--webview-exe-name=locus\.exe" -or $normalizedCmd -match "com\.locus\.app\\ebwebview")) {
        return $true
    }

    return $false
}

function Get-RelevantProcesses {
    $rows = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine)
    if ($rows.Count -eq 0) {
        return @()
    }

    $byPid = @{}
    foreach ($row in $rows) {
        $byPid[[int]$row.ProcessId] = $row
    }

    $relevantPids = New-Object 'System.Collections.Generic.HashSet[int]'
    foreach ($row in $rows) {
        if (Test-IsSeedProcess -Name $row.Name -CommandLine $row.CommandLine) {
            [void]$relevantPids.Add([int]$row.ProcessId)
        }
    }

    $expanded = $true
    while ($expanded) {
        $expanded = $false
        foreach ($row in $rows) {
            $procId = [int]$row.ProcessId
            $parentPid = [int]$row.ParentProcessId
            if ($relevantPids.Contains($procId)) {
                continue
            }
            if ($relevantPids.Contains($parentPid)) {
                [void]$relevantPids.Add($procId)
                $expanded = $true
            }
        }
    }

    $processMetrics = @{}
    foreach ($proc in @(Get-Process -ErrorAction SilentlyContinue)) {
        $processMetrics[[int]$proc.Id] = $proc
    }

    $result = New-Object System.Collections.Generic.List[object]
    foreach ($procId in (@($relevantPids) | Sort-Object)) {
        $row = $byPid[$procId]
        $proc = $processMetrics[$procId]
        if (-not $row -or -not $proc) {
            continue
        }

        $startTime = $null
        try {
            $startTime = Safe-ToIso -Value $proc.StartTime
        } catch {
            $startTime = $null
        }

        $result.Add([pscustomobject][ordered]@{
            pid = $procId
            parentPid = [int]$row.ParentProcessId
            name = $row.Name
            role = Get-ProcessRole -Name $row.Name -CommandLine $row.CommandLine
            commandLine = $row.CommandLine
            workingSetMB = Mb $proc.WorkingSet64
            privateMB = Mb $proc.PrivateMemorySize64
            pagedMB = Mb $proc.PagedMemorySize64
            virtualMB = Mb $proc.VirtualMemorySize64
            handles = [int]$proc.HandleCount
            threadCount = [int]$proc.Threads.Count
            cpuSec = [math]::Round((Or-Zero $proc.CPU), 2)
            startTime = $startTime
        })
    }

    return $result.ToArray()
}

function Get-Totals {
    param([object[]]$Processes)

    if (-not $Processes -or $Processes.Count -eq 0) {
        return [pscustomobject][ordered]@{
            processCount = 0
            workingSetMB = 0
            privateMB = 0
            pagedMB = 0
            virtualMB = 0
            handles = 0
            threadCount = 0
        }
    }

    return [pscustomobject][ordered]@{
        processCount = $Processes.Count
        workingSetMB = [math]::Round((Or-Zero (($Processes | Measure-Object -Property workingSetMB -Sum).Sum)), 2)
        privateMB = [math]::Round((Or-Zero (($Processes | Measure-Object -Property privateMB -Sum).Sum)), 2)
        pagedMB = [math]::Round((Or-Zero (($Processes | Measure-Object -Property pagedMB -Sum).Sum)), 2)
        virtualMB = [math]::Round((Or-Zero (($Processes | Measure-Object -Property virtualMB -Sum).Sum)), 2)
        handles = [int](Or-Zero (($Processes | Measure-Object -Property handles -Sum).Sum))
        threadCount = [int](Or-Zero (($Processes | Measure-Object -Property threadCount -Sum).Sum))
    }
}

function Get-DeltaSummary {
    param([object[]]$Series)

    if (-not $Series -or $Series.Count -lt 2) {
        return $null
    }

    $first = $Series[0]
    $last = $Series[-1]
    $durationMinutes = [math]::Max(($Series.Count - 1) * $SampleIntervalSeconds / 60.0, 0.01)
    $privateDelta = [math]::Round($last.privateMB - $first.privateMB, 2)
    $workingDelta = [math]::Round($last.workingSetMB - $first.workingSetMB, 2)
    $privateGrowthPerMinute = [math]::Round($privateDelta / $durationMinutes, 2)
    $workingGrowthPerMinute = [math]::Round($workingDelta / $durationMinutes, 2)

    $nonDecreasingPrivateSteps = 0
    $stepCount = 0
    for ($i = 1; $i -lt $Series.Count; $i++) {
        $stepCount++
        if ($Series[$i].privateMB -ge $Series[$i - 1].privateMB) {
            $nonDecreasingPrivateSteps++
        }
    }
    $trendScore = if ($stepCount -gt 0) {
        [math]::Round($nonDecreasingPrivateSteps / $stepCount, 2)
    } else {
        0
    }

    $maxPrivate = [math]::Round((Or-Zero (($Series | Measure-Object -Property privateMB -Maximum).Maximum)), 2)
    $maxWorking = [math]::Round((Or-Zero (($Series | Measure-Object -Property workingSetMB -Maximum).Maximum)), 2)

    $signal = "stable"
    if ($privateDelta -ge 150 -and $trendScore -ge 0.7) {
        $signal = "strong-growth"
    } elseif ($privateDelta -ge 60 -and $trendScore -ge 0.55) {
        $signal = "growth"
    } elseif (($maxPrivate - $first.privateMB) -ge 100 -and [math]::Abs($privateDelta) -lt 60) {
        $signal = "spiky"
    }

    return [pscustomobject][ordered]@{
        sampleCount = $Series.Count
        firstPrivateMB = $first.privateMB
        lastPrivateMB = $last.privateMB
        firstWorkingSetMB = $first.workingSetMB
        lastWorkingSetMB = $last.workingSetMB
        maxPrivateMB = $maxPrivate
        maxWorkingSetMB = $maxWorking
        privateDeltaMB = $privateDelta
        workingSetDeltaMB = $workingDelta
        privateGrowthPerMinuteMB = $privateGrowthPerMinute
        workingSetGrowthPerMinuteMB = $workingGrowthPerMinute
        trendScore = $trendScore
        signal = $signal
    }
}

function Format-ProcessLine {
    param([object]$Item)

    return "- {0} [{1}] ({2}, PID {3}): Private {4} MB -> {5} MB ({6:+0.##;-0.##;0}), WorkingSet {7} MB -> {8} MB ({9:+0.##;-0.##;0}), trend={10}, signal={11}" -f `
        $Item.name,
        $Item.role,
        $Item.commandHint,
        $Item.pid,
        $Item.delta.firstPrivateMB,
        $Item.delta.lastPrivateMB,
        $Item.delta.privateDeltaMB,
        $Item.delta.firstWorkingSetMB,
        $Item.delta.lastWorkingSetMB,
        $Item.delta.workingSetDeltaMB,
        $Item.delta.trendScore,
        $Item.delta.signal
}

if ($DurationSeconds -lt 20) {
    throw "DurationSeconds must be at least 20."
}
if ($SampleIntervalSeconds -lt 2) {
    throw "SampleIntervalSeconds must be at least 2."
}

$resolvedOutputRoot = Get-OutputRoot -ConfiguredOutputRoot $OutputRoot
Ensure-Directory -Path $resolvedOutputRoot

$windowId = Get-Date -Format "yyyyMMdd-HHmmss"
$windowDir = Join-Path $resolvedOutputRoot $windowId
Ensure-Directory -Path $windowDir

$startedAt = Get-Date
$samples = New-Object System.Collections.Generic.List[object]
$sampleTicks = [math]::Max([math]::Floor($DurationSeconds / $SampleIntervalSeconds), 1)

for ($tick = 0; $tick -le $sampleTicks; $tick++) {
    $capturedAt = Get-Date
    $processes = @(Get-RelevantProcesses)
    $samples.Add([pscustomobject][ordered]@{
        index = $tick
        capturedAt = Safe-ToIso -Value $capturedAt
        totals = Get-Totals -Processes $processes
        processes = $processes
    })

    if ($tick -lt $sampleTicks) {
        Start-Sleep -Seconds $SampleIntervalSeconds
    }
}

$endedAt = Get-Date
$allSamples = $samples.ToArray()

$byPid = @{}
foreach ($sample in $allSamples) {
    foreach ($proc in @($sample.processes)) {
        $procId = [int]$proc.pid
        if (-not $byPid.ContainsKey($procId)) {
            $byPid[$procId] = New-Object System.Collections.Generic.List[object]
        }
        $byPid[$procId].Add([pscustomobject][ordered]@{
            capturedAt = $sample.capturedAt
            privateMB = $proc.privateMB
            workingSetMB = $proc.workingSetMB
            handles = $proc.handles
            threadCount = $proc.threadCount
            cpuSec = $proc.cpuSec
        })
    }
}

$processSummaries = New-Object System.Collections.Generic.List[object]
foreach ($entry in ($byPid.GetEnumerator() | Sort-Object Key)) {
    $procId = [int]$entry.Key
    $series = $entry.Value.ToArray()
    $firstSeen = $null
    foreach ($sample in $allSamples) {
        $candidate = @($sample.processes | Where-Object { $_.pid -eq $procId }) | Select-Object -First 1
        if ($candidate) {
            $firstSeen = $candidate
            break
        }
    }
    if (-not $firstSeen) {
        continue
    }

    $commandHint = ""
    if ($firstSeen.commandLine) {
        $commandHint = ($firstSeen.commandLine -replace "\s+", " ").Trim()
        if ($commandHint.Length -gt 72) {
            $commandHint = $commandHint.Substring(0, 72) + "..."
        }
    }

    $processSummaries.Add([pscustomobject][ordered]@{
        pid = $procId
        parentPid = $firstSeen.parentPid
        name = $firstSeen.name
        role = $firstSeen.role
        commandHint = $commandHint
        startTime = $firstSeen.startTime
        delta = Get-DeltaSummary -Series $series
    })
}

$processSummaries = @($processSummaries | Where-Object { $_.delta } | Sort-Object { $_.delta.privateDeltaMB } -Descending)

$totalSeries = @()
foreach ($sample in $allSamples) {
    $totalSeries += [pscustomobject][ordered]@{
        privateMB = $sample.totals.privateMB
        workingSetMB = $sample.totals.workingSetMB
    }
}
$totalDelta = Get-DeltaSummary -Series $totalSeries

$topProcesses = @($processSummaries | Select-Object -First 8)
$topSuspects = @($processSummaries | Where-Object { $_.delta.signal -in @("strong-growth", "growth") } | Select-Object -First 5)
$notes = New-Object System.Collections.Generic.List[string]

if ($totalDelta) {
    if ($totalDelta.signal -eq "strong-growth") {
        $notes.Add("Total memory kept growing across the window. Treat this as a likely leak first.")
    } elseif ($totalDelta.signal -eq "growth") {
        $notes.Add("Total memory increased across the window, but more windows are needed to confirm whether it falls back.")
    } elseif ($totalDelta.signal -eq "spiky") {
        $notes.Add("Total memory looks more like a spike. Separate cache or preview peaks from objects that never return.")
    } else {
        $notes.Add("Total memory stayed mostly flat in this window. No strong leak signal was captured this time.")
    }
}

if (@($topSuspects | Where-Object { $_.role -like "webview-*" }).Count -gt 0) {
    $notes.Add("Most growth is in WebView2 child processes. Check frontend object lifetimes, heavy preview components, Three.js or Canvas resources, and unreleased reactive caches first.")
}
if (@($topSuspects | Where-Object { $_.role -eq "app-main" }).Count -gt 0) {
    $notes.Add("locus.exe itself is growing. This points more to Rust or Tauri resident state, watchers, index caches, or long-lived background tasks.")
}
if (@($topSuspects | Where-Object { $_.role -like "dev-*" }).Count -gt 0) {
    $notes.Add("Growth includes bun or cargo. That may come from the dev toolchain, so do not attribute it to the product process without a packaged-build comparison.")
}

$staticHints = @(
    "Frontend path: if growth is concentrated in msedgewebview2 renderer, revisit binary previews, diff panels, large lists, and object cleanup after session switches.",
    "Backend path: if growth is concentrated in locus.exe, inspect long-lived watchers, index state, task registries, and cross-session caches for upper bounds and cleanup timing.",
    "Dev-mode path: if growth is mostly in bun.exe or cargo.exe, compare against a packaged build before calling it a product leak."
)

$summary = [pscustomobject][ordered]@{
    windowId = $windowId
    startedAt = Safe-ToIso -Value $startedAt
    endedAt = Safe-ToIso -Value $endedAt
    durationSeconds = [int][math]::Round(($endedAt - $startedAt).TotalSeconds)
    sampleIntervalSeconds = $SampleIntervalSeconds
    sampleCount = $allSamples.Count
    outputDir = $windowDir
    total = $totalDelta
    topProcesses = $topProcesses
    topSuspects = $topSuspects
    notes = $notes.ToArray()
    staticHints = $staticHints
}

$summaryMd = @()
$summaryMd += "# Memory Window Summary"
$summaryMd += ""
$summaryMd += "- Window: $windowId"
$summaryMd += "- Started: $($summary.startedAt)"
$summaryMd += "- Ended: $($summary.endedAt)"
$summaryMd += "- Duration: $($summary.durationSeconds) sec"
$summaryMd += "- Sample interval: $($summary.sampleIntervalSeconds) sec"
$summaryMd += "- Sample count: $($summary.sampleCount)"
$summaryMd += ""
if ($summary.total) {
    $summaryMd += "## Total Trend"
    $summaryMd += ""
    $summaryMd += "- Private: $($summary.total.firstPrivateMB) MB -> $($summary.total.lastPrivateMB) MB ($("{0:+0.##;-0.##;0}" -f $summary.total.privateDeltaMB))"
    $summaryMd += "- WorkingSet: $($summary.total.firstWorkingSetMB) MB -> $($summary.total.lastWorkingSetMB) MB ($("{0:+0.##;-0.##;0}" -f $summary.total.workingSetDeltaMB))"
    $summaryMd += "- Trend score: $($summary.total.trendScore)"
    $summaryMd += "- Signal: $($summary.total.signal)"
    $summaryMd += ""
}
if ($topSuspects.Count -gt 0) {
    $summaryMd += "## Suspect Processes"
    $summaryMd += ""
    foreach ($item in $topSuspects) {
        $summaryMd += (Format-ProcessLine -Item $item)
    }
    $summaryMd += ""
}
if ($topProcesses.Count -gt 0) {
    $summaryMd += "## Top Process Deltas"
    $summaryMd += ""
    foreach ($item in $topProcesses) {
        $summaryMd += (Format-ProcessLine -Item $item)
    }
    $summaryMd += ""
}
if ($notes.Count -gt 0) {
    $summaryMd += "## Notes"
    $summaryMd += ""
    foreach ($note in $notes) {
        $summaryMd += "- $note"
    }
    $summaryMd += ""
}
$summaryMd += "## Static Investigation Hints"
$summaryMd += ""
foreach ($hint in $staticHints) {
    $summaryMd += "- $hint"
}
$summaryMd += ""

$metadata = [pscustomobject][ordered]@{
    repoRoot = Get-RepoRoot
    machine = $env:COMPUTERNAME
    user = $env:USERNAME
    startedAt = $summary.startedAt
    endedAt = $summary.endedAt
    durationSeconds = $summary.durationSeconds
    sampleIntervalSeconds = $SampleIntervalSeconds
}

$metadataPath = Join-Path $windowDir "metadata.json"
$samplesPath = Join-Path $windowDir "samples.json"
$summaryPath = Join-Path $windowDir "summary.json"
$summaryMdPath = Join-Path $windowDir "summary.md"

$metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $metadataPath -Encoding utf8
$allSamples | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $samplesPath -Encoding utf8
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding utf8
$summaryMd -join [Environment]::NewLine | Set-Content -LiteralPath $summaryMdPath -Encoding utf8

Write-Output "Memory monitor window complete."
Write-Output "summary.json: $summaryPath"
Write-Output "summary.md: $summaryMdPath"
