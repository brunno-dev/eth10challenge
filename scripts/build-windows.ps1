param([switch]$Selftest, [switch]$Check)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$cudaRoot = Join-Path $projectRoot '.cuda/v12.8'
if (-not (Test-Path -LiteralPath (Join-Path $cudaRoot 'bin/nvcc.exe'))) {
    throw 'Run scripts/setup-cuda.ps1 first, or build using your installed toolkit.'
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vsRoot) { throw 'Visual Studio C++ Build Tools are required.' }
$devCmd = Join-Path $vsRoot 'Common7/Tools/VsDevCmd.bat'
$names = @('PATH','INCLUDE','LIB','LIBPATH','NVCC','CUDA_PATH','CUDA_LIBRARY_PATH','CUDA_CACHE_PATH')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name,'Process') }
try {
    $compilerEnvironment = & cmd.exe /d /c "`"$devCmd`" -no_logo -arch=x64 -host_arch=x64 >nul && set"
    if ($LASTEXITCODE -ne 0) { throw 'Could not initialize MSVC environment.' }
    $imported = @{}
    foreach ($line in $compilerEnvironment) {
        $pair = $line -split '=',2
        # Some launchers supply both Path and PATH. VsDevCmd emits its updated
        # PATH first; do not overwrite it with a stale differently-cased entry.
        if ($pair.Length -eq 2 -and $names -contains $pair[0] -and -not $imported.ContainsKey($pair[0])) {
            [Environment]::SetEnvironmentVariable($pair[0],$pair[1],'Process')
            $imported[$pair[0]] = $true
        }
    }
    if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) { throw 'MSVC cl.exe was not found after initialization.' }
    $env:NVCC = Join-Path $cudaRoot 'bin/nvcc.exe'
    $env:CUDA_PATH = $cudaRoot
    $env:CUDA_LIBRARY_PATH = $cudaRoot
    if (-not $env:CUDA_CACHE_PATH) {
        $env:CUDA_CACHE_PATH = Join-Path $projectRoot '.cuda/jit-cache'
        New-Item -ItemType Directory -Force -Path $env:CUDA_CACHE_PATH | Out-Null
    }
    Push-Location $projectRoot
    try {
        & cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'CUDA build failed.' }
        if ($Check) {
            & cargo clippy --release --all-targets --locked -- -D warnings
            if ($LASTEXITCODE -ne 0) { throw 'CUDA Clippy checks failed.' }
            & cargo test --release --locked
            if ($LASTEXITCODE -ne 0) { throw 'CUDA-build unit tests failed.' }
            & cargo test --release --no-default-features --locked
            if ($LASTEXITCODE -ne 0) { throw 'CPU-build unit tests failed.' }
        }
        if ($Selftest) {
            $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $projectRoot 'target' }
            & (Join-Path $target 'release/words-breaker.exe') --selftest
            if ($LASTEXITCODE -ne 0) { throw 'CUDA selftest failed.' }
        }
    } finally { Pop-Location }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name,$previous[$name],'Process') }
}
