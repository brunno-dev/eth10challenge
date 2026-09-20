param(
    [string]$Binary = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target/release/words-breaker.exe'),
    [int]$Candidates = 4194304,
    [int]$Repeats = 3,
    [string]$BaselineBinary,
    [switch]$DefaultOnly,
    [string]$OutputCsv
)
$ErrorActionPreference = 'Stop'
if ($Candidates -lt 1 -or $Repeats -lt 1) { throw 'Candidates and Repeats must be positive.' }
$previousCache = $env:CUDA_CACHE_PATH
if (-not $env:CUDA_CACHE_PATH) {
    $env:CUDA_CACHE_PATH = Join-Path (Split-Path $PSScriptRoot -Parent) '.cuda/jit-cache'
    New-Item -ItemType Directory -Force -Path $env:CUDA_CACHE_PATH | Out-Null
}
# Public synthetic inputs only. Every setting checks the same candidate prefix.
$words = 'abandon ability able about above absent absorb abstract absurd abuse access accident' -split ' '
$settings = @(
    @{Block=32; Batch=1048576}, @{Block=64; Batch=1048576},
    @{Block=128; Batch=1048576}, @{Block=256; Batch=1048576},
    @{Block=64; Batch=262144}, @{Block=64; Batch=2097152}
)
if ($DefaultOnly) { $settings = @($settings[1]) }
function Measure-Search($setting, $limit, $executable) {
    $lines = & $executable '0x0000000000000000000000000000000000000000' @words --max-candidates $limit --block-size $setting.Block --batch-size $setting.Batch 2>&1
    if ($LASTEXITCODE -ne 0) { throw ($lines -join "`n") }
    $output = $lines -join "`n"
    if ($output -notmatch 'Using GPU') { throw "GPU unavailable; refusing to benchmark CPU fallback.`n$output" }
    if ($output -notmatch 'Candidate limit reached; search incomplete: (\d+) candidates in ([\d.]+)s \((\d+) candidates/s\)') {
        throw "Unexpected benchmark output: $output"
    }
    if ([long]$Matches[1] -ne $limit) { throw 'Unexpected number of checked candidates.' }
    [pscustomobject]@{
        Block = $setting.Block; Batch = $setting.Batch; Candidates = $limit
        # Preserve the binary's decimal point, independently of Windows locale.
        Seconds = $Matches[2]
        CandidatesPerSecond = [long]$Matches[3]
    }
}
try {
    Write-Host 'Warming up GPU and driver JIT (the first run can take several minutes)...'
    $versions = @(@{Name='current'; Binary=$Binary})
    if ($BaselineBinary) { $versions += @{Name='baseline'; Binary=$BaselineBinary} }
    foreach ($version in $versions) {
        $null = Measure-Search $settings[0] ([Math]::Min($Candidates,1048576)) $version.Binary
    }
    $rows = @()
    for ($repeat = 0; $repeat -lt $Repeats; $repeat++) {
        # Alternate order to reduce bias from heat and changing clock frequencies.
        $order = if ($repeat % 2) { ($settings.Count-1)..0 } else { 0..($settings.Count-1) }
        foreach ($index in $order) {
            $versionOrder = if ($repeat % 2) { ($versions.Count-1)..0 } else { 0..($versions.Count-1) }
            foreach ($versionIndex in $versionOrder) {
                $version = $versions[$versionIndex]
                $row = Measure-Search $settings[$index] $Candidates $version.Binary
                $row | Add-Member -NotePropertyName Repeat -NotePropertyValue ($repeat+1)
                $row | Add-Member -NotePropertyName Variant -NotePropertyValue $version.Name
                $rows += $row
                Write-Host "$($version.Name) repeat=$($repeat+1) block=$($row.Block) batch=$($row.Batch): $($row.CandidatesPerSecond) candidates/s ($($row.Seconds)s)"
                if ($OutputCsv) { $rows | Export-Csv -LiteralPath $OutputCsv -NoTypeInformation }
            }
        }
    }
    $rows
} finally { $env:CUDA_CACHE_PATH = $previousCache }
