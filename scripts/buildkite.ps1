$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

# Keep setup, failure checks and artifact globs out of the pipeline command:
# Buildkite only needs to invoke this file, without nested PowerShell quoting.
$windMsvcEnv = msvc-env x64
if ($LASTEXITCODE -ne 0) { throw 'MSVC environment setup failed' }
$windMsvcEnv | Invoke-Expression

& "$PSScriptRoot/ci.ps1" -Build

$windReleaseDir = 'target/x86_64-pc-windows-msvc/release'
foreach ($windArtifact in @(
    'divvun-wind.exe',
    'divvun-wind-symbols.exe',
    'divvunwind.dll',
    '*.pdb'
)) {
    buildkite-agent artifact upload "$windReleaseDir/$windArtifact"
    if ($LASTEXITCODE -ne 0) { throw "Artifact upload failed: $windArtifact" }
}
