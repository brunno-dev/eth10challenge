param(
    [Parameter(Mandatory)][string]$Binary,
    [Parameter(Mandatory)][string]$History,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(1,100)][int]$Trials = 3,
    [ValidateRange(1,1000000000)][int]$MaxCandidates = 5000000,
    [string[]]$Cases = @('coins', 'combined')
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
$historyPath = (Resolve-Path -LiteralPath $History).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a new output directory.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$outputPath = (Resolve-Path -LiteralPath $OutputDirectory).Path
$catalog = Get-Content -LiteralPath (Join-Path $projectRoot 'history/research/video-onscreen-words.json') -Raw | ConvertFrom-Json
$settings = @(
    @{Name='fixed-65536'; Batch=65536; Adaptive=$false},
    @{Name='fixed-262144'; Batch=262144; Adaptive=$false},
    @{Name='fixed-1048576'; Batch=1048576; Adaptive=$false},
    @{Name='adaptive-1048576'; Batch=1048576; Adaptive=$true}
)
$results = @()
$previousCache = $env:CUDA_CACHE_PATH
$env:CUDA_CACHE_PATH = Join-Path $projectRoot '.cuda/jit-cache'
New-Item -ItemType Directory -Force -Path $env:CUDA_CACHE_PATH | Out-Null
try {
    # Keep initialization/JIT outside the timed samples. Public synthetic input;
    # no negative evidence for the real challenge is recorded by this warmup.
    $warmup = & $binaryPath '0x0000000000000000000000000000000000000000' `
        abandon ability able about above absent absorb abstract absurd abuse access accident `
        --max-candidates 1048576 --batch-size 65536 --block-size 64 2>&1
    if ($LASTEXITCODE -ne 0 -or -not ($warmup -match 'Using GPU')) { throw 'CUDA warmup failed.' }
    $warmup | Set-Content -LiteralPath (Join-Path $outputPath 'warmup.log') -Encoding utf8
    foreach ($case in $Cases) {
        $newWords = switch ($case) {
            'coins' { $catalog.coin_words_from_the_portfolio_table }
            'combined' { @($catalog.onscreen_not_in_pool) + @($catalog.spoken_not_in_pool) | Sort-Object -Unique }
            default { throw "Unknown case: $case" }
        }
        $videoWords = [System.Collections.Generic.List[string]]::new()
        foreach ($word in (@('fiber', 'wood', 'winter', 'rib') + $newWords)) {
            if (-not $videoWords.Contains($word)) { $videoWords.Add($word) }
        }
        $video = 'fog@5 parrot@12 ' + ($videoWords -join ' ')
        $referenceCheckpoint = $null
        $referenceDigest = $null
        $referenceCounts = $null
        for ($trial = 1; $trial -le $Trials; $trial++) {
            $order = if ($trial % 2) { 0..($settings.Count-1) } else { ($settings.Count-1)..0 }
            foreach ($index in $order) {
                $setting = $settings[$index]
                $prefix = Join-Path $outputPath "$case-$trial-$($setting.Name)"
                $arguments = @(
                    '0x9c2f44efad0c1e852a09df9939e6daf061140caf',
                    '--post', 'dutch@1 fiber fork dinner cloud live', '--video', $video,
                    '--batch-size', "$($setting.Batch)", '--block-size', '64', '--max-candidates', "$MaxCandidates",
                    '--exclude-tested', 'RO1', '--exclude-record-dir', $historyPath,
                    '--checkpoint', "$prefix.checkpoint.json", '--record-progress', "$prefix.record.json",
                    '--metrics-json', "$prefix.metrics.json", '--metrics'
                )
                if ($setting.Adaptive) { $arguments += '--adaptive-batch' }
                $timer = [System.Diagnostics.Stopwatch]::StartNew()
                $log = & $binaryPath @arguments 2>&1
                $code = $LASTEXITCODE
                $timer.Stop()
                $log | Set-Content -LiteralPath "$prefix.log" -Encoding utf8
                if ($code -ne 0) { throw "Engine failed ($code): $prefix.log" }
                if ($log -match 'MATCH FOUND|FOUND MATCH|Found matching|Match found') { throw "Match reported: inspect $prefix.log" }
                if (-not (Test-Path -LiteralPath "$prefix.record.json")) { throw "No confirmed negative record: $prefix.log" }
                $metrics = Get-Content -LiteralPath "$prefix.metrics.json" -Raw | ConvertFrom-Json
                if ($metrics.backend -ne 'CUDA') { throw 'Benchmark did not use CUDA.' }
                if ($metrics.completed_raw -ne $MaxCandidates) { throw 'Unexpected candidate count.' }
                $checkpoint = Get-Content -LiteralPath "$prefix.checkpoint.json" -Raw | ConvertFrom-Json | ConvertTo-Json -Depth 30 -Compress
                $digest = (Get-Content -LiteralPath "$prefix.record.json" -Raw | ConvertFrom-Json).sha256
                $counts = @($metrics.completed_raw, $metrics.excluded, $metrics.retained, $metrics.checksum_survivors) -join ','
                if ($null -eq $referenceCheckpoint) {
                    $referenceCheckpoint = $checkpoint; $referenceDigest = $digest; $referenceCounts = $counts
                } elseif ($referenceCheckpoint -ne $checkpoint -or $referenceDigest -ne $digest -or $referenceCounts -ne $counts) {
                    throw "Coverage or result mismatch: $prefix"
                }
                $results += [pscustomobject]@{
                    case=$case; trial=$trial; variant=$setting.Name; batch=$setting.Batch; adaptive=$setting.Adaptive
                    wall_seconds=$timer.Elapsed.TotalSeconds; metrics=$metrics
                }
                $results | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputPath 'results.json') -Encoding utf8
                [pscustomobject]@{
                    case=$case; trial=$trial; variant=$setting.Name; wall_seconds=$timer.Elapsed.TotalSeconds
                    derive=$metrics.derive_seconds; batches=$metrics.device_batches; checksum=$metrics.checksum_survivors
                } | ConvertTo-Json -Compress
            }
        }
    }
} finally { $env:CUDA_CACHE_PATH = $previousCache }
