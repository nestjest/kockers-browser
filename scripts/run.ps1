param([switch]$Release)

$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "build.ps1") -Release:$Release
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$bundle = Join-Path (Split-Path -Parent $PSScriptRoot) "target\bundle"
Start-Process -FilePath (Join-Path $bundle "kockers_browser.exe") -WorkingDirectory $bundle
