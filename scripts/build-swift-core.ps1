# Maintainer/CI build only. SwiftPM consumers never execute this script.
[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string[]]$Target
)

$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    $buildProfile = if ($env:FORWIROH_PROFILE) { $env:FORWIROH_PROFILE } else { 'release' }
    $targetDirectory = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    foreach ($rustTarget in $Target) {
        # Swift uses the shared MSVC CRT. The standalone CLI's +crt-static
        # setting must not be used for these libraries.
        $previousFlags = $env:RUSTFLAGS
        try {
            $env:RUSTFLAGS = "$previousFlags -C target-feature=-crt-static"
            # Emit only staticlib so Rust completes cross-crate LTO before Swift links.
            cargo rustc --locked --profile $buildProfile -p forwiroh --lib --crate-type staticlib --target $rustTarget -- --print native-static-libs
            if ($LASTEXITCODE -ne 0) { throw "Rust build failed for $rustTarget" }
        } finally {
            $env:RUSTFLAGS = $previousFlags
        }
        $destination = Join-Path 'dist/libraries' $rustTarget
        New-Item -ItemType Directory -Force -Path $destination | Out-Null
        Copy-Item (Join-Path $targetDirectory "$rustTarget/$buildProfile/forwiroh.lib") $destination
    }
} finally {
    Pop-Location
}
