param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [int]$Repeats = 3,
    [string]$OutputPath
)
$ErrorActionPreference = 'Stop'
if ($Repeats -lt 1) { throw 'Repeats must be positive.' }
$exe = (Resolve-Path -LiteralPath $Binary).Path
$prefix = Join-Path ([System.IO.Path]::GetTempPath()) ('pruning-benchmark-' + [guid]::NewGuid().ToString())
$ownedFiles = [System.Collections.Generic.List[string]]::new()
$previousCache = $env:CUDA_CACHE_PATH
$common = @('0x0000000000000000000000000000000000000000', '--batch-size', '65536')
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
    Write-Host 'Creating nine independent synthetic search records; the first CUDA run may compile the driver cache...'
    foreach ($word in $words[0..8]) {
        $record = "$prefix-$word.json"
        $ownedFiles.Add($record)
        $records += $record
        $rest = ($words | Where-Object { $_ -ne $word }) -join ' '
        $output = & $exe @common --pattern "$fixed $word ? ? ? ? ?" --pool $rest --record-search $record 2>&1
        $exitCode = $LASTEXITCODE
        $text = $output -join "`n"
        if ($exitCode -ne 0 -or -not (Test-Path -LiteralPath $record) -or
            -not $text.Contains('Exhausted search without a match')) {
            throw "Record creation failed for ${word}:`n$text"
        }
    }
    $exclusions = @($records | ForEach-Object { '--exclude-record'; $_ })
    $cases = @(
        @{ Name='phrase-filter-only'; Extra=@('--no-prune-history') },
        @{ Name='prefix-pruning'; Extra=@() }
    )
    for ($trial = 0; $trial -le $Repeats; $trial++) {
        # Trial zero warms both modes; later trials alternate to reduce order bias.
        $order = @(0, 1)
        if ($trial % 2 -ne 0) { [array]::Reverse($order) }
        foreach ($index in $order) {
            $case = $cases[$index]
            $extra = $case.Extra
            # An explicit, fresh checkpoint disables the whole-search shortcut.
            $checkpoint = "$prefix-trial$trial-$($case.Name).json"
            $ownedFiles.Add($checkpoint)
            $timer = [System.Diagnostics.Stopwatch]::StartNew()
            $output = & $exe @common @query @exclusions @extra --checkpoint $checkpoint 2>&1
            $exitCode = $LASTEXITCODE
            $timer.Stop()
            $text = $output -join "`n"
            if ($exitCode -ne 0 -or -not $text.Contains('History excluded 136080 arrangements') -or
                -not $text.Contains('Exhausted search without a match') -or
                -not (Test-Path -LiteralPath $checkpoint)) {
                throw "Unexpected result for $($case.Name), trial ${trial}:`n$text"
            }
            $backend = if ($text.Contains('Using GPU (CUDA)')) { 'CUDA' }
                elseif ($text.Contains('Using CPU')) { 'CPU' }
                else { throw "No search engine initialized for $($case.Name):`n$text" }
            $pruned = 0L
            if ($text -match 'Pruned (\d+) candidates without generation this run') {
                $pruned = [long]$Matches[1]
            }
            if (($case.Name -eq 'prefix-pruning' -and $pruned -le 0) -or
                ($case.Name -eq 'phrase-filter-only' -and $pruned -ne 0)) {
                throw "Unexpected pruning counter for $($case.Name):`n$text"
            }
            if ($trial -gt 0) {
                $results += [pscustomobject]@{
                    Trial = $trial
                    Case = $case.Name
                    Seconds = $timer.Elapsed.TotalSeconds.ToString('F6', [cultureinfo]::InvariantCulture)
                    Backend = $backend
                    Candidates = 151200
                    Excluded = 136080
                    Pruned = $pruned
                }
                if ($OutputPath) { $results | Export-Csv -LiteralPath $OutputPath -NoTypeInformation }
            }
            Write-Host "trial=$trial case=$($case.Name) backend=$backend seconds=$($timer.Elapsed.TotalSeconds.ToString('F6', [cultureinfo]::InvariantCulture))"
        }
    }
    $results | Format-Table -AutoSize
} finally {
    $env:CUDA_CACHE_PATH = $previousCache
    foreach ($path in $ownedFiles) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
}
