param([switch]$Release)

$ErrorActionPreference = "Stop"
$arguments = @("run", "--bin", "kockers-bundle", "--", "-o", "target/bundle")
if ($Release) {
    $arguments += "--release"
}
& (Join-Path $PSScriptRoot "cargo.ps1") -CargoArgs $arguments
exit $LASTEXITCODE
