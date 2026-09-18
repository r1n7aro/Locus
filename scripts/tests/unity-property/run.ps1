param(
    [Parameter(Mandatory = $true)][string]$UnityEditor,
    [string]$OutputRoot = 'E:\LocusTemp',
    [Parameter(Mandatory = $true)][string]$YamlDriver
)
$ErrorActionPreference = 'Stop'
$reviewRepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..\..')).Path
$reviewEditor = (Resolve-Path -LiteralPath $UnityEditor).Path
$reviewRuntimeRoot = Join-Path ([IO.Path]::GetFullPath($OutputRoot)) ('property-review-' + [guid]::NewGuid().ToString('N').Substring(0, 12))
New-Item -ItemType Directory -Path $reviewRuntimeRoot,
    (Join-Path $reviewRuntimeRoot 'Assets\PropertyReview\Editor'),
    (Join-Path $reviewRuntimeRoot 'Packages'),
    (Join-Path $reviewRuntimeRoot 'ProjectSettings') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $reviewRepoRoot 'locus_unity') -Destination (Join-Path $reviewRuntimeRoot 'Packages\com.farlocus.locus') -Recurse
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'ReviewData.cs'), (Join-Path $PSScriptRoot 'ReviewComponent.cs') -Destination (Join-Path $reviewRuntimeRoot 'Assets\PropertyReview')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'PropertyReviewRunner.cs') -Destination (Join-Path $reviewRuntimeRoot 'Assets\PropertyReview\Editor')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'PropertyYamlParity.cs') -Destination (Join-Path $reviewRuntimeRoot 'Assets\PropertyReview\Editor')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'PropertyAuthoringParity.cs') -Destination (Join-Path $reviewRuntimeRoot 'Assets\PropertyReview\Editor')
Set-Content -LiteralPath (Join-Path $reviewRuntimeRoot 'Assets\PropertyReview\PropertyReview.Runtime.asmdef') -Value '{"name":"PropertyReview.Runtime","references":[],"autoReferenced":true}'
Set-Content -LiteralPath (Join-Path $reviewRuntimeRoot 'Assets\PropertyReview\Editor\PropertyReview.Editor.asmdef') -Value '{"name":"PropertyReview.Editor","references":["PropertyReview.Runtime","Locus.Editor"],"includePlatforms":["Editor"],"autoReferenced":true}'
$reviewModules = @('animation', 'audio', 'physics', 'physics2d', 'ui', 'imgui', 'jsonserialize', 'unitywebrequest', 'unitywebrequesttexture', 'imageconversion', 'terrain', 'terrainphysics', 'particlesystem', 'director', 'video')
$reviewDependencies = @{}
foreach ($reviewModule in $reviewModules) { $reviewDependencies['com.unity.modules.' + $reviewModule] = '1.0.0' }
@{ dependencies = $reviewDependencies } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $reviewRuntimeRoot 'Packages\manifest.json')
$reviewEditorVersion = [regex]::Match([Diagnostics.FileVersionInfo]::GetVersionInfo($reviewEditor).ProductVersion, '^\d+\.\d+\.\d+[abfp]\d+').Value
if (-not $reviewEditorVersion) { throw "Cannot determine the Unity version from $reviewEditor" }
Set-Content -LiteralPath (Join-Path $reviewRuntimeRoot 'ProjectSettings\ProjectVersion.txt') -Value ('m_EditorVersion: ' + $reviewEditorVersion)
$reviewLog = Join-Path $reviewRuntimeRoot 'Editor.log'
$reviewProcess = Start-Process -FilePath $reviewEditor -ArgumentList @('-batchmode', '-nographics', '-projectPath', ('"' + $reviewRuntimeRoot + '"'), '-executeMethod', 'PropertyReview.PropertyReviewRunner.Run', '-logFile', ('"' + $reviewLog + '"'), '-locusPropertyYamlDriver', ('"' + $YamlDriver + '"')) -WindowStyle Hidden -PassThru
@{ runtimeRoot = $reviewRuntimeRoot; pid = $reviewProcess.Id; startedAt = $reviewProcess.StartTime.ToString('O'); log = $reviewLog } | ConvertTo-Json -Compress | Write-Output
$reviewDeadline = [DateTime]::UtcNow.AddMinutes(10)
while (-not $reviewProcess.WaitForExit(1000)) {
    if ([DateTime]::UtcNow -gt $reviewDeadline) { throw "Review timed out; inspect owned PID $($reviewProcess.Id) and $reviewLog" }
}
$reviewResultPath = Join-Path $reviewRuntimeRoot 'property-review-results.json'
if (-not (Test-Path -LiteralPath $reviewResultPath)) { throw "No results were produced; inspect $reviewLog" }
$reviewResult = Get-Content -Raw -LiteralPath $reviewResultPath | ConvertFrom-Json
$reviewResult.cases | Select-Object id, passed, actual | Format-Table -Wrap
Write-Output "Results: $reviewResultPath"
Write-Output ('LOCUS_PROPERTY_TEST_JSON ' + (@{ runtimeRoot = $reviewRuntimeRoot; results = $reviewResultPath; cases = $reviewResult.cases.Count; passed = @($reviewResult.cases | Where-Object passed).Count; failed = @($reviewResult.cases | Where-Object { -not $_.passed }).Count } | ConvertTo-Json -Compress))
if (@($reviewResult.cases | Where-Object { -not $_.passed }).Count -gt 0) { exit 1 }

