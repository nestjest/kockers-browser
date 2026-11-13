$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot "build.ps1") -Release
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$installation = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$runtimeRoot = Join-Path $installation "VC\Redist\MSVC"
$runtime = Get-ChildItem $runtimeRoot -Filter "vcruntime140.dll" -Recurse |
    Where-Object FullName -Match "\\x64\\Microsoft.VC143.CRT\\" |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $runtime) {
    throw "Microsoft VC++ x64 runtime не найден в MSVC Build Tools."
}
Copy-Item $runtime.FullName (Join-Path $root "target\bundle\vcruntime140.dll") -Force

$iscc = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
if ($iscc) {
    $compiler = $iscc.Source
} else {
    $compiler = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $compiler) {
    throw "Inno Setup 6 не найден. Установите JRSoftware.InnoSetup через winget."
}

$installerScript = Join-Path $root "installer\KockersBrowser.iss"
& $compiler $installerScript
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$definition = Select-String -Path $installerScript -Pattern '^#define AppVersion "([^"]+)"$'
if (-not $definition) {
    throw "Не удалось определить версию установщика."
}
$version = $definition.Matches[0].Groups[1].Value
$setup = Join-Path $root "dist\KockersBrowser-$version-windows-x64-setup.exe"
if (-not (Test-Path $setup)) {
    throw "Inno Setup не создал ожидаемый файл $setup."
}
$hash = Get-FileHash -Algorithm SHA256 $setup
$name = Split-Path -Leaf $setup
$checksums = Join-Path $root "dist\SHA256SUMS.txt"
Set-Content -Path $checksums -Value "$($hash.Hash.ToLowerInvariant())  $name" -Encoding Ascii
Write-Host "SHA-256: $($hash.Hash.ToLowerInvariant())  $name"
exit 0
