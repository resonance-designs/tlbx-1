$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$dist = Join-Path $root "dist\\windows"
$standaloneDir = Join-Path $dist "standalone"
$vst3Dir = Join-Path $dist "vst3"

Write-Host "Building standalone binary..."
& cargo build --release

New-Item -ItemType Directory -Force -Path $standaloneDir | Out-Null
New-Item -ItemType Directory -Force -Path $vst3Dir | Out-Null

Copy-Item -Force (Join-Path $root "target\\release\\grainrust.exe") (Join-Path $standaloneDir "grainrust.exe")

$VST3Path = $env:GRAINRUST_VST3_PATH
if (-not $VST3Path) {
    Write-Host "GRAINRUST_VST3_PATH not set. Skipping VST3 staging."
} else {
    if (Test-Path $VST3Path) {
        Copy-Item -Recurse -Force $VST3Path (Join-Path $vst3Dir "GrainRust.vst3")
    } else {
        Write-Host "GRAINRUST_VST3_PATH does not exist: $VST3Path"
    }
}

$nsis = Get-Command makensis -ErrorAction SilentlyContinue
if (-not $nsis) {
    Write-Host "makensis not found. Install NSIS and ensure makensis is in PATH."
    exit 1
}

Write-Host "Building NSIS installer..."
& makensis (Join-Path $PSScriptRoot "installer.nsi")
