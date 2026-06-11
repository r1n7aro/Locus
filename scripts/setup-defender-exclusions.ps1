#Requires -RunAsAdministrator
# Adds Windows Defender exclusions that speed up Locus builds (cargo emits
# huge numbers of small files; real-time scanning typically costs 20-40%).
#
# Run once from an elevated PowerShell:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-defender-exclusions.ps1
#
# Security note: excluded paths are no longer scanned in real time. Only add
# directories whose contents you control (this repo + Rust toolchain caches).

$repoRoot = Split-Path -Parent $PSScriptRoot

$paths = @(
    $repoRoot,
    (Join-Path $env:USERPROFILE ".cargo"),
    (Join-Path $env:USERPROFILE ".rustup"),
    (Join-Path $env:LOCALAPPDATA "Mozilla\sccache")
)

$processes = @(
    "cargo.exe",
    "rustc.exe",
    "rust-lld.exe",
    "link.exe",
    "sccache.exe",
    "bun.exe",
    "node.exe",
    "dotnet.exe",
    "makensis.exe"
)

foreach ($p in $paths) {
    Add-MpPreference -ExclusionPath $p
    Write-Host "excluded path:    $p"
}

foreach ($p in $processes) {
    Add-MpPreference -ExclusionProcess $p
    Write-Host "excluded process: $p"
}

Write-Host ""
Write-Host "Done. Current exclusions:"
$pref = Get-MpPreference
$pref.ExclusionPath | ForEach-Object { Write-Host "  path:    $_" }
$pref.ExclusionProcess | ForEach-Object { Write-Host "  process: $_" }
