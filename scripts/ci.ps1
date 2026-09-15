param([switch]$Build)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

# The runner must provide an x64 MSVC developer environment and Windows SDK.
# rust-toolchain.toml and Cargo.lock determine Rust and dependency versions.
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
cargo clippy --workspace --all-targets --locked -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
cargo test --workspace --locked
if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
if ($Build) {
    cargo build --workspace --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
}
