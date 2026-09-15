param(
    [Parameter(Mandatory)][string]$Payload,
    [Parameter(Mandatory)][string]$Output
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$windMsvcEnv = msvc-env x64
if ($LASTEXITCODE -ne 0) { throw 'MSVC environment setup failed' }
$windMsvcEnv | Invoke-Expression
$env:WIND_OUTTO_INSTALLER = (Resolve-Path -LiteralPath $Payload).Path
cargo build -p divvun-wind-installer --release --locked --features embedded-payload
if ($LASTEXITCODE -ne 0) { throw 'Installer launcher build failed' }
Copy-Item -LiteralPath 'target/x86_64-pc-windows-msvc/release/divvun-wind-installer.exe' -Destination $Output
