# codetracer-wasmi-recorder Windows dev environment (PowerShell)
# Usage: . .\env.ps1
#
# This repo is a fork of the Wasmi WebAssembly interpreter; the CodeTracer
# recorder lives in `crates/cli` (`wasmi_cli`) and embeds the Wasmi VM
# directly.  It builds and tests with a plain `cargo build` / `cargo test`.
# Its Windows requirements are:
#
#   1. The shared CodeTracer toolchain (Rust, Nim + nimble, just, Cap'n Proto,
#      MSVC).  These are provisioned by the main `codetracer` repo's env.ps1,
#      which this script dot-sources.  Nim is needed because the
#      `codetracer_trace_writer_nim` crate (a path dependency of `wasmi_cli`)
#      has a build script that compiles a Nim static library.
#
#   2. An explicit MSVC linker for the `x86_64-pc-windows-msvc` target.
#      Pinning `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER` to MSVC's
#      absolute `link.exe` keeps `cargo` linking with the correct linker
#      even when invoked from a Git Bash shell whose PATH places coreutils'
#      `link.exe` ahead of the MSVC toolchain.

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

# --- 1. Shared CodeTracer toolchain -----------------------------------------
$env:WINDOWS_DIY_SKIP_FPC = "1"
$env:WINDOWS_DIY_SKIP_LLVM = "1"
$env:WINDOWS_DIY_SKIP_NARGO = "1"
$env:WINDOWS_DIY_SKIP_DOTNET = "1"

$codetracerEnv = Join-Path (Split-Path -Parent $scriptDir) "codetracer\env.ps1"
if (-not (Test-Path $codetracerEnv)) {
    throw "Could not find the shared CodeTracer env.ps1 at $codetracerEnv -- the ``codetracer`` repo must be checked out as a sibling of this repo."
}
. $codetracerEnv

# --- 2. Explicit MSVC linker (immune to Git Bash PATH reordering) -----------
if ($env:WINDOWS_DIY_CL_EXE -and (Test-Path $env:WINDOWS_DIY_CL_EXE)) {
    $msvcBin = Split-Path -Parent $env:WINDOWS_DIY_CL_EXE
    $msvcLink = Join-Path $msvcBin "link.exe"
    if (Test-Path $msvcLink) {
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $msvcLink
    }
    if ($env:Path -notlike "$msvcBin;*") {
        $env:Path = "$msvcBin;$($env:Path)"
    }
}

Write-Host "codetracer-wasmi-recorder dev environment ready."
Write-Host "Build/test the recorder with: cargo build -p wasmi_cli / cargo test -p wasmi_cli"
