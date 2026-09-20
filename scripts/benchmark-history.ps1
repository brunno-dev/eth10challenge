param(
    [Parameter(Mandatory)][string]$Before,
    [Parameter(Mandatory)][string]$After,
    [Parameter(Mandatory)][string]$History,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [int]$Trials = 3,
    [int]$MaxCandidates = 5000000,
    [ValidateRange(1,16777216)][int]$BatchSize = 65536,
    [string[]]$Cases = @('coins', 'combined')
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$beforePath = (Resolve-Path -LiteralPath $Before).Path
$afterPath = (Resolve-Path -LiteralPath $After).Path
$historyPath = (Resolve-Path -LiteralPath $History).Path
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Use a new output directory.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$outputPath = (Resolve-Path -LiteralPath $OutputDirectory).Path
$catalog = Get-Content -LiteralPath (Join-Path $projectRoot 'history/research/video-onscreen-words.json') -Raw | ConvertFrom-Json
$results = @()
$env:CUDA_CACHE_PATH = Join-Path $projectRoot '.cuda/jit-cache'
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
    for ($trial = 1; $trial -le $Trials; $trial++) {
        $order = if ($trial % 2) { @('before', 'after') } else { @('after', 'before') }
        foreach ($variant in $order) {
            $prefix = Join-Path $outputPath "$case-$trial-$variant"
            $binary = if ($variant -eq 'before') { $beforePath } else { $afterPath }
            $arguments = @(
                '0x9c2f44efad0c1e852a09df9939e6daf061140caf',
                '--post', 'dutch@1 fiber fork dinner cloud live', '--video', $video,
                '--batch-size', "$BatchSize", '--max-candidates', "$MaxCandidates",
                '--exclude-tested', 'RO1', '--exclude-record-dir', $historyPath,
                '--checkpoint', "$prefix.checkpoint.json", '--record-progress', "$prefix.record.json",
                '--metrics-json', "$prefix.metrics.json", '--metrics'
            )
            $timer = [System.Diagnostics.Stopwatch]::StartNew()
            $log = & $binary @arguments 2>&1
            $code = $LASTEXITCODE
            $timer.Stop()
            $log | Set-Content -LiteralPath "$prefix.log" -Encoding utf8
            if ($code -ne 0) { throw "Engine failed ($code): $prefix.log" }
            if ($log -match 'MATCH FOUND|FOUND MATCH|Found matching|Match found') {
                throw "Match reported; inspect $prefix.log before continuing."
            }
            if (-not (Test-Path -LiteralPath "$prefix.record.json")) { throw 'No confirmed negative record; inspect the run.' }
            $metrics = Get-Content -LiteralPath "$prefix.metrics.json" -Raw | ConvertFrom-Json
            if ($metrics.backend -ne 'CUDA') { throw 'Benchmark did not use CUDA.' }
            $results += [pscustomobject]@{
                case = $case; trial = $trial; variant = $variant
                wall_seconds = $timer.Elapsed.TotalSeconds; metrics = $metrics
            }
            $results | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputPath 'results.json') -Encoding utf8
            [pscustomobject]@{
                case=$case; trial=$trial; variant=$variant; wall_seconds=$timer.Elapsed.TotalSeconds
                raw=$metrics.completed_raw; excluded=$metrics.excluded
                producer=$metrics.producer_seconds; derive=$metrics.derive_seconds; pruned=$metrics.pruned
                seed=$metrics.gpu_seed_seconds; address=$metrics.gpu_address_seconds
            } | ConvertTo-Json -Compress
        }
        $old = Get-Content -LiteralPath (Join-Path $outputPath "$case-$trial-before.checkpoint.json") -Raw | ConvertFrom-Json
        $new = Get-Content -LiteralPath (Join-Path $outputPath "$case-$trial-after.checkpoint.json") -Raw | ConvertFrom-Json
        # Entire saved progress must agree, including original order, evidence
        # fingerprint, checked/excluded counts and the next enumeration cursor.
        if (($old | ConvertTo-Json -Depth 30 -Compress) -ne ($new | ConvertTo-Json -Depth 30 -Compress)) {
            throw "Checkpoint mismatch: $case trial $trial"
        }
        $oldRecord = (Get-Content -LiteralPath (Join-Path $outputPath "$case-$trial-before.record.json") -Raw | ConvertFrom-Json).sha256
        $newRecord = (Get-Content -LiteralPath (Join-Path $outputPath "$case-$trial-after.record.json") -Raw | ConvertFrom-Json).sha256
        if ($oldRecord -ne $newRecord) { throw "Negative record mismatch: $case trial $trial" }
    }
}
