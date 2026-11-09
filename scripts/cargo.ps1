param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) {
    throw "Visual Studio Build Tools not found. Install the VCTools workload first."
}

$installation = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ([string]::IsNullOrWhiteSpace($installation)) {
    throw "MSVC x64 tools not found. Install the VCTools workload first."
}

$devCommand = Join-Path $installation "Common7\Tools\VsDevCmd.bat"
$environment = cmd.exe /d /s /c "`"$devCommand`" -arch=x64 -host_arch=x64 >nul && set"
if ($LASTEXITCODE -ne 0) {
    throw "VsDevCmd failed with exit code $LASTEXITCODE."
}
foreach ($line in $environment) {
    $parts = $line -split "=", 2
    if ($parts.Count -eq 2) {
        Set-Item -Path "Env:$($parts[0])" -Value $parts[1]
    }
}

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    $cargoPath = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $cargoPath) {
        $cargo = Get-Item $cargoPath
    } else {
        throw "Cargo not found. Install Rust stable from https://rustup.rs/."
    }
}

& $cargo @CargoArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
