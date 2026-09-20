param([Parameter(Mandatory=$true)][string]$Binary)
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Binary).Path
$tag = Join-Path ([IO.Path]::GetTempPath()) ('packing-resume-' + [guid]::NewGuid())
$firstRecord = "$tag-first.json"
$lastRecord = "$tag-last.json"
$checkpoint = "$tag-checkpoint.json"
$baseline = "$tag-baseline.json"
$cpuMetrics = "$tag-cpu-metrics.json"
$gpuMetrics = "$tag-gpu-metrics.json"
$tailMetrics = "$tag-tail-metrics.json"
$baselineMetrics = "$tag-baseline-metrics.json"
$previousCache = $env:CUDA_CACHE_PATH
$fixed = 'abandon abandon abandon abandon abandon abandon abandon abandon'
$common = @('0x0000000000000000000000000000000000000000', '--batch-size', '7')
$query = @('--pattern', "$fixed ? ? ? ?", '--pool', 'abandon ability able about above',
    '--exclude-record', $firstRecord, '--exclude-record', $lastRecord)
function Invoke-Search([string[]]$SearchArgs, [switch]$RequireGpu) {
    $output = & $exe @SearchArgs 2>&1
    $text = $output -join "`n"
    if ($LASTEXITCODE -ne 0) { throw "Search failed:`n$text" }
    if ($RequireGpu -and -not $text.Contains('Using GPU (CUDA)')) { throw "CUDA required:`n$text" }
}
try {
    $env:CUDA_CACHE_PATH = Join-Path (Split-Path $PSScriptRoot -Parent) '.cuda/jit-cache'
    Invoke-Search ($common + @('--cpu', '--threads', '2', '--pattern', "$fixed abandon ? ? ?",
        '--pool', 'ability able about above', '--record-search', $firstRecord))
    Invoke-Search ($common + @('--cpu', '--threads', '2', '--pattern', "$fixed ? ? ? above",
        '--pool', 'abandon ability able about', '--record-search', $lastRecord))
    Invoke-Search ($common + $query + @('--cpu', '--threads', '2', '--max-candidates', '13',
        '--checkpoint', $checkpoint, '--metrics-json', $cpuMetrics))
    $saved = Get-Content -LiteralPath $checkpoint -Raw | ConvertFrom-Json
    if ($saved.checked -ne 13 -or $saved.excluded -ne 13) { throw 'CPU limit mismatch' }
    Invoke-Search ($common + $query + @('--max-candidates', '17', '--checkpoint', $checkpoint, '--resume', '--adaptive-batch', '--metrics-json', $gpuMetrics)) -RequireGpu
    $saved = Get-Content -LiteralPath $checkpoint -Raw | ConvertFrom-Json
    if ($saved.checked -ne 30) { throw 'GPU raw limit mismatch' }
    Invoke-Search ($common + $query + @('--checkpoint', $checkpoint, '--resume', '--no-pack-history', '--no-count-cache', '--metrics-json', $tailMetrics)) -RequireGpu
    Invoke-Search ($common + $query + @('--checkpoint', $baseline, '--no-pack-history', '--no-count-cache', '--no-prune-history', '--metrics-json', $baselineMetrics)) -RequireGpu
    $saved = Get-Content -LiteralPath $checkpoint -Raw | ConvertFrom-Json
    $expected = Get-Content -LiteralPath $baseline -Raw | ConvertFrom-Json
    # Each record covers 24 phrases; their intersection contains 3P2 = 6.
    if ($saved.checked -ne 120 -or $saved.excluded -ne 42 -or
        $saved.checked -ne $expected.checked -or $saved.excluded -ne $expected.excluded -or
        ($saved.cursor | ConvertTo-Json -Compress) -ne ($expected.cursor | ConvertTo-Json -Compress)) {
        throw 'Final checkpoint differs from uninterrupted baseline'
    }
    $parts = @($cpuMetrics, $gpuMetrics, $tailMetrics) | ForEach-Object { Get-Content -LiteralPath $_ -Raw | ConvertFrom-Json }
    $reference = Get-Content -LiteralPath $baselineMetrics -Raw | ConvertFrom-Json
    if ($parts[0].completed_raw -ne 13 -or $parts[1].completed_raw -ne 17 -or $parts[2].completed_raw -ne 90 -or
        $parts[0].backend -ne 'CPU' -or $parts[1].backend -ne 'CUDA') { throw 'Per-run metrics mismatch' }
    foreach ($field in @('completed_raw', 'excluded', 'retained', 'checksum_survivors')) {
        $sum = ($parts | Measure-Object -Property $field -Sum).Sum
        if ($sum -ne $reference.$field) { throw "Resumed metrics disagree with baseline: $field" }
    }
    foreach ($part in $parts) {
        if ($part.completed_raw -ne $part.excluded + $part.retained -or
            $part.pruned -gt $part.excluded -or $part.checksum_survivors -gt $part.retained) { throw 'Metrics conservation failed' }
    }
    Write-Output 'PASS: CPU -> adaptive packed GPU -> unpacked/uncached GPU; identical cursor, 120 raw, 42 exclusions and matching per-run metrics.'
} finally {
    $env:CUDA_CACHE_PATH = $previousCache
    foreach ($path in @($firstRecord, $lastRecord, $checkpoint, $baseline, $cpuMetrics, $gpuMetrics, $tailMetrics, $baselineMetrics)) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
}
