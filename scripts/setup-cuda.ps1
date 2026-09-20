# Installs the compiler, headers and driver import library locally. No driver or
# system environment changes. NVIDIA's redistribution license is in each archive.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$cudaRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot '.cuda/v12.8'))
$downloads = Join-Path $projectRoot '.cuda/downloads'
New-Item -ItemType Directory -Force -Path $downloads,$cudaRoot | Out-Null
$packages = @(
    @{
        Name = 'cuda_nvcc'; Version = '12.8.93'
        Sha256 = '9fdc70b4271ed9aad4d64cd7076a7d96ec36512d074b9995fe638de669197391'
    },
    @{
        Name = 'cuda_cudart'; Version = '12.8.90'
        Sha256 = '4a39058fd8519444a81cfc7ae055d136f48d1a31ffa41ae255b35b2edd61e13b'
    }
)
Add-Type -AssemblyName System.IO.Compression.FileSystem
foreach ($package in $packages) {
    $file = "$($package.Name)-windows-x86_64-$($package.Version)-archive.zip"
    $archive = Join-Path $downloads $file
    if (-not (Test-Path -LiteralPath $archive)) {
        $url = "https://developer.download.nvidia.com/compute/cuda/redist/$($package.Name)/windows-x86_64/$file"
        Write-Host "Downloading $file"
        Invoke-WebRequest -Uri $url -OutFile $archive
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.Sha256) {
        throw "Checksum mismatch: $archive. Remove this archive and retry."
    }
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        foreach ($entry in $zip.Entries) {
            $parts = $entry.FullName -split '/',2
            if ($parts.Length -lt 2 -or -not $parts[1] -or $entry.FullName.EndsWith('/')) { continue }
            $destination = [IO.Path]::GetFullPath((Join-Path $cudaRoot $parts[1]))
            if (-not $destination.StartsWith($cudaRoot + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {
                throw "Archive entry escapes toolkit directory: $($entry.FullName)"
            }
            New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($destination)) | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry,$destination,$true)
        }
    } finally { $zip.Dispose() }
}
& (Join-Path $cudaRoot 'bin/nvcc.exe') --version
if ($LASTEXITCODE -ne 0) { throw 'CUDA compiler installation failed.' }
Write-Host 'Toolkit ready. Run scripts/build-windows.ps1 -Selftest.'
