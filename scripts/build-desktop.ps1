param([switch]$SkipEngine, [switch]$Check)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$desktopRoot = Join-Path $projectRoot 'desktop'
$buildRoot = Split-Path $projectRoot -Parent
$names = @('CARGO_HOME', 'CARGO_TARGET_DIR', 'PATH', 'INCLUDE', 'LIB', 'LIBPATH')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    New-Item -ItemType Directory -Force -Path (Join-Path $desktopRoot 'src-tauri/engine') | Out-Null
    if (-not $SkipEngine) {
        $localCargo = Join-Path $buildRoot 'review-cargo-home'
        if (Test-Path -LiteralPath $localCargo) { $env:CARGO_HOME = $localCargo }
        $env:CARGO_TARGET_DIR = Join-Path $buildRoot 'gpu-target'
        & (Join-Path $PSScriptRoot 'build-windows.ps1') -Check:$Check
        Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'release/words-breaker.exe') -Destination (Join-Path $desktopRoot 'src-tauri/engine/words-breaker.exe') -Force
    }
    if (-not (Test-Path -LiteralPath (Join-Path $desktopRoot 'src-tauri/engine/words-breaker.exe'))) {
        throw 'Prepare the CUDA engine first, or run without -SkipEngine.'
    }
    # Tauri dependencies use the standard Cargo cache, separate from CUDA builds.
    $env:CARGO_HOME = Join-Path $env:USERPROFILE '.cargo'
    $env:CARGO_TARGET_DIR = Join-Path $buildRoot 'desktop-target'
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vsRoot) { throw 'Visual Studio C++ Build Tools are required.' }
    $devCmd = Join-Path $vsRoot 'Common7/Tools/VsDevCmd.bat'
    $compilerEnvironment = & cmd.exe /d /c "`"$devCmd`" -no_logo -arch=x64 -host_arch=x64 >nul && set"
    if ($LASTEXITCODE -ne 0) { throw 'Could not initialize MSVC.' }
    $imported = @{}
    foreach ($line in $compilerEnvironment) {
        $pair = $line -split '=', 2
        if ($pair.Length -eq 2 -and @('PATH','INCLUDE','LIB','LIBPATH') -contains $pair[0] -and -not $imported.ContainsKey($pair[0])) {
            [Environment]::SetEnvironmentVariable($pair[0], $pair[1], 'Process')
            $imported[$pair[0]] = $true
        }
    }
    Push-Location $desktopRoot
    try {
        & npm ci --offline=false --cache (Join-Path $buildRoot 'npm-cache')
        if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed.' }
        & npm run build
        if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
        if ($Check) {
            & cargo clippy --manifest-path src-tauri/Cargo.toml --release --locked --all-targets -- -D warnings
            if ($LASTEXITCODE -ne 0) { throw 'Tauri Clippy failed.' }
            & cargo test --manifest-path src-tauri/Cargo.toml --release --locked
            if ($LASTEXITCODE -ne 0) { throw 'Tauri tests failed.' }
        }
        & npm run tauri -- build --no-bundle
        if ($LASTEXITCODE -ne 0) { throw 'Tauri build failed.' }
        $release = Join-Path $desktopRoot 'release'
        New-Item -ItemType Directory -Force -Path (Join-Path $release 'engine') | Out-Null
        Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'release/eth-search-studio.exe') -Destination (Join-Path $release 'ETH Search Studio.exe') -Force
        Copy-Item -LiteralPath (Join-Path $desktopRoot 'src-tauri/engine/words-breaker.exe') -Destination (Join-Path $release 'engine/words-breaker.exe') -Force
        Copy-Item -LiteralPath (Join-Path $desktopRoot 'LEIA-ME.txt') -Destination $release -Force
        Copy-Item -LiteralPath (Join-Path $desktopRoot 'exemplo-lista.txt') -Destination $release -Force
        Compress-Archive -Path (Join-Path $release '*') -DestinationPath (Join-Path $desktopRoot 'ETH-Search-Studio-Windows.zip') -Force
        Write-Output "Ready: $(Join-Path $release 'ETH Search Studio.exe')"
    } finally { Pop-Location }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
