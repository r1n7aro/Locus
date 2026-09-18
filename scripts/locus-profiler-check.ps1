param(
    [Parameter(Mandatory = $true)][string]$UnityEditor,
    [string]$OutputRoot = [IO.Path]::GetTempPath(),
    [int]$TimeoutSeconds = 180
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$editor = (Resolve-Path -LiteralPath $UnityEditor).Path
$data = Join-Path (Split-Path $editor) 'Data'
$runtime = Join-Path ([IO.Path]::GetFullPath($OutputRoot)) ('locus-profiler-' + [Guid]::NewGuid().ToString('N'))
$project = Join-Path $runtime 'project'
$assets = Join-Path $project 'Assets/Editor'
New-Item -ItemType Directory -Path $assets,(Join-Path $project 'Packages'),(Join-Path $project 'ProjectSettings') -Force | Out-Null
Set-Content -LiteralPath (Join-Path $project 'Packages/manifest.json') -Value '{"dependencies":{}}'
$version = (Get-Item -LiteralPath $editor).VersionInfo.ProductVersion
if ($version -notmatch '^(\d+)\.(\d+)\.(\d+)') { throw "Cannot determine Unity version: $version" }
$major = [int]$Matches[1]
$minor = [int]$Matches[2]
Set-Content -LiteralPath (Join-Path $project 'ProjectSettings/ProjectVersion.txt') -Value "m_EditorVersion: $version"

$dotnet = (Get-Command dotnet -ErrorAction Stop).Source
$sdk = (& $dotnet --list-sdks | Select-Object -Last 1)
if ($sdk -notmatch '^(\S+)\s+\[(.+)\]') { throw 'A .NET SDK is required.' }
$compiler = Join-Path $Matches[2] ($Matches[1] + '/Roslyn/bincore/csc.dll')
$defines = [Collections.Generic.List[string]]::new()
foreach ($symbol in @('UNITY_EDITOR','UNITY_EDITOR_WIN','UNITY_STANDALONE_WIN','ENABLE_PROFILER','ENABLE_MONO','NET_STANDARD_2_1','UNITY_2020_1_OR_NEWER','UNITY_2020_2_OR_NEWER','UNITY_2020_3_OR_NEWER','UNITY_2021_1_OR_NEWER','UNITY_2021_2_OR_NEWER','UNITY_2021_3_OR_NEWER','UNITY_2022_1_OR_NEWER','UNITY_2022_2_OR_NEWER','UNITY_2022_3_OR_NEWER')) { $defines.Add($symbol) }
if ($major -ge 6000) {
    foreach ($n in 0..$minor) { $defines.Add("UNITY_6000_${n}_OR_NEWER") }
    $defines.Add('UNITY_6000_OR_NEWER')
    $defines.Add("UNITY_6000_$minor")
} else { $defines.Add('UNITY_2022_3') }

$references = @{}
$referenceFolders = @('NetStandard/ref/2.1.0','NetStandard/compat/2.1.0/shims/netfx','NetStandard/compat/2.1.0/shims/netstandard','NetStandard/EditorExtensions','BCLExtensions/TargetingPacks/netstandard2.1/ref','Managed/UnityEngine')
foreach ($folder in $referenceFolders) {
    $path = Join-Path $data $folder
    if (Test-Path -LiteralPath $path) {
        Get-ChildItem -LiteralPath $path -Filter '*.dll' | ForEach-Object { $references[$_.Name] = $_.FullName }
    }
}
foreach ($name in @('UnityEditor.dll','UnityEngine.dll')) {
    if (!$references.ContainsKey($name)) { $references[$name] = Join-Path $data ('Managed/' + $name) }
}
foreach ($folder in @('Roslyn','Json','Detour','HotReload')) {
    Get-ChildItem -LiteralPath (Join-Path $repo "locus_unity/Editor/$folder") -Filter '*.dll' | ForEach-Object {
        $references[$_.Name] = $_.FullName
        Copy-Item -LiteralPath $_.FullName -Destination $assets
    }
}
$rsp = @('/nologo','/nostdlib+','/target:library','/unsafe+','/langversion:latest','/nowarn:0649,0169,0414',('/define:' + ($defines -join ';')),('/out:"' + (Join-Path $assets 'Locus.Editor.dll') + '"'))
$rsp += $references.Values | Sort-Object | ForEach-Object { '/reference:"' + $_ + '"' }
$rsp += Get-ChildItem -LiteralPath (Join-Path $repo 'locus_unity/Editor'),(Join-Path $repo 'locus_unity/Runtime') -Filter '*.cs' -Recurse |
    Where-Object { $_.FullName -notmatch '[/\\]Testing[/\\]' } | ForEach-Object { '"' + $_.FullName + '"' }
$rsp += '"' + (Join-Path $repo 'scripts/tests/ProfilerApiChecks.cs') + '"'
$responsePath = Join-Path $runtime 'checks.rsp'
Set-Content -LiteralPath $responsePath -Value $rsp
& $dotnet $compiler "@$responsePath" *> (Join-Path $runtime 'compile.log')
if ($LASTEXITCODE -ne 0) { Get-Content -LiteralPath (Join-Path $runtime 'compile.log'); throw "Compilation failed: $runtime" }

$nativeDir = Join-Path $project 'Assets/Plugins/x86_64'
New-Item -ItemType Directory -Path $nativeDir -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'locus_unity/Editor/Native/x86_64/locus_native.dll') -Destination $nativeDir
$log = Join-Path $runtime 'unity.log'
$proc = Start-Process -FilePath $editor -ArgumentList @('-batchmode','-projectPath',('"' + $project + '"'),'-executeMethod','Locus.LocusBridge.RunProfilerApiChecks','-logFile',('"' + $log + '"')) -WindowStyle Hidden -PassThru
Write-Output ('LOCUS_PROFILER_RUNTIME_JSON ' + (@{ runtimeRoot = $runtime; project = $project; pid = $proc.Id; started = $proc.StartTime; executable = $editor; log = $log } | ConvertTo-Json -Compress))
$deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
while (!$proc.WaitForExit(1000)) {
    if ([DateTime]::UtcNow -ge $deadline) {
        # Only this invocation's tracked process is stopped; preserve all other Unity/Locus instances.
        $identity = Get-CimInstance Win32_Process -Filter "ProcessId = $($proc.Id)"
        $identity | Select-Object ProcessId,ParentProcessId,CreationDate,ExecutablePath,CommandLine | ConvertTo-Json -Compress | Write-Output
        if ($identity.ExecutablePath -eq $editor -and $identity.CommandLine.Contains($project)) { Stop-Process -Id $proc.Id -Force }
        throw "Profiler checks timed out: $log"
    }
}
$result = Get-Content -LiteralPath $log | Where-Object { $_ -match '^LOCUS_PROFILER_TEST_JSON ' } | Select-Object -Last 1
if ($proc.ExitCode -ne 0 -or !$result -or !(($result -replace '^LOCUS_PROFILER_TEST_JSON ','') | ConvertFrom-Json).ok) {
    Get-Content -LiteralPath $log -Tail 100
    throw "Profiler checks failed: $log"
}
Write-Output $result
