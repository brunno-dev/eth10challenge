param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [int]$Repeats = 3,
    [string]$OutputPath
)
$ErrorActionPreference = 'Stop'
if ($Repeats -lt 1) { throw 'Repeats must be positive.' }
$exe = (Resolve-Path -LiteralPath $Binary).Path
$prefix = Join-Path ([System.IO.Path]::GetTempPath()) ('packing-benchmark-' + [guid]::NewGuid().ToString())
$ownedFiles = [System.Collections.Generic.List[string]]::new()
$previousCache = $env:CUDA_CACHE_PATH
$common = @('0x0000000000000000000000000000000000000000', '--batch-size', '1024')
$fixed = 'abandon abandon abandon abandon abandon abandon'
$words = @('abandon', 'ability', 'able', 'about', 'above', 'absent', 'absorb', 'abstract', 'absurd', 'abuse')
$query = @('--pattern', "$fixed ? ? ? ? ? ?", '--pool', ($words -join ' '))
$records = @()
$results = @()
try {
    if (-not $env:CUDA_CACHE_PATH) {
        $env:CUDA_CACHE_PATH = Join-Path (Split-Path $PSScriptRoot -Parent) '.cuda/jit-cache'
        New-Item -ItemType Directory -Force -Path $env:CUDA_CACHE_PATH | Out-Null
    }
    Write-Host 'Creating nine independent synthetic records; CUDA is required and the first run may compile the driver cache...'
    foreach ($word in $words[0..8]) {
        $record = "$prefix-$word.json"
        $ownedFiles.Add($record)
        $records += $record
        $rest = ($words | Where-Object { $_ -ne $word }) -join ' '
        # Fix the final hole so exclusions stay interleaved throughout the stream.
        $output = & $exe @common --pattern "$fixed ? ? ? ? ? $word" --pool $rest --record-search $record 2>&1
        $exitCode = $LASTEXITCODE
        $text = $output -join "`n"
        if ($exitCode -ne 0 -or -not (Test-Path -LiteralPath $record) -or
            -not $text.Contains('Exhausted search without a match: 15120 candidates')) {
            throw "Record creation failed for ${word}:`n$text"
        }
        if (-not $text.Contains('Using GPU (CUDA)')) {
            throw "CUDA unavailable; refusing to benchmark a CPU fallback.`n$text"
        }
    }
    $exclusions = @($records | ForEach-Object { '--exclude-record'; $_ })
    $cases = @(
        @{ Name='previous-behavior'; Extra=@('--no-pack-history', '--no-count-cache') },
        @{ Name='cache-only'; Extra=@('--no-pack-history') },
        @{ Name='packing-and-cache'; Extra=@() }
    )
    for ($trial = 0; $trial -le $Repeats; $trial++) {
        # Trial zero warms all cases; measured trials alternate their order.
        $order = @(0, 1, 2)
        if ($trial % 2 -ne 0) { [array]::Reverse($order) }
        foreach ($index in $order) {
            $case = $cases[$index]
            $extra = $case.Extra
            $checkpoint = "$prefix-trial$trial-$($case.Name).json"
            $ownedFiles.Add($checkpoint)
            $timer = [System.Diagnostics.Stopwatch]::StartNew()
            $output = & $exe @common @query @exclusions @extra --checkpoint $checkpoint 2>&1
            $exitCode = $LASTEXITCODE
            $timer.Stop()
            $text = $output -join "`n"
            if ($exitCode -ne 0 -or -not (Test-Path -LiteralPath $checkpoint)) {
                throw "Search failed for $($case.Name), trial ${trial}:`n$text"
            }
            if (-not $text.Contains('Using GPU (CUDA)')) {
                throw "CUDA unavailable; refusing to benchmark a CPU fallback.`n$text"
            }
            if ($text -notmatch 'History excluded (\d+) arrangements this run \((\d+) cumulative\)') {
                throw "Missing exclusion counters for $($case.Name):`n$text"
            }
            if ([long]$Matches[1] -ne 136080 -or [long]$Matches[2] -ne 136080) {
                throw "Incorrect exclusion counters for $($case.Name):`n$text"
            }
            if ($text -notmatch 'Exhausted search without a match: (\d+) candidates in ([\d.]+)s \((\d+) candidates/s\)') {
                throw "Missing completed search summary for $($case.Name):`n$text"
            }
            if ([long]$Matches[1] -ne 151200) {
                throw "Incorrect original candidate count for $($case.Name):`n$text"
            }
            $searchSeconds = $Matches[2]
            $rawCandidatesPerSecond = [long]$Matches[3]
            $pruned = 0L
            if ($text -match 'Pruned (\d+) candidates without generation this run') {
                $pruned = [long]$Matches[1]
            }
            if ($pruned -ne 0) {
                throw "Unexpected pruning in the interleaved packing workload for $($case.Name):`n$text"
            }
            if ($trial -gt 0) {
                $results += [pscustomobject]@{
                    Trial = $trial
                    Case = $case.Name
                    Seconds = $timer.Elapsed.TotalSeconds.ToString('F6', [cultureinfo]::InvariantCulture)
                    SearchSeconds = $searchSeconds
                    RawCandidatesPerSecond = $rawCandidatesPerSecond
                    Backend = 'CUDA'
                    BatchSize = 1024
                    Candidates = 151200
                    Excluded = 136080
                    Pruned = $pruned
                }
                if ($OutputPath) { $results | Export-Csv -LiteralPath $OutputPath -NoTypeInformation }
            }
            Write-Host "trial=$trial case=$($case.Name) backend=CUDA seconds=$($timer.Elapsed.TotalSeconds.ToString('F6', [cultureinfo]::InvariantCulture)) searchSeconds=$searchSeconds pruned=$pruned"
        }
    }
    $results | Format-Table -AutoSize
} finally {
    $env:CUDA_CACHE_PATH = $previousCache
    foreach ($path in $ownedFiles) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
}
