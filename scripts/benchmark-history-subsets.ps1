param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [int]$Repeats = 3,
    [string]$OutputPath,
    [switch]$Cpu
)
$ErrorActionPreference = 'Stop'
if ($Repeats -lt 1) { throw 'Repeats must be positive' }
$exe = (Resolve-Path -LiteralPath $Binary).Path
$recordPath = Join-Path ([System.IO.Path]::GetTempPath()) ('subset-benchmark-' + [guid]::NewGuid().ToString() + '.json')
$common = @('0x0000000000000000000000000000000000000000', '--batch-size', '65536')
if ($Cpu) { $common += '--cpu' }
$pattern = 'abandon abandon abandon abandon abandon abandon ? ? ? ? ? ?'
$pool = 'abandon ability able about above absent absorb abstract absurd abuse'
$cases = @(
    @{ Name='smaller-pool'; Count=60480; Args=@('--pattern',$pattern,'--pool','abandon ability able about above absent absorb abstract absurd') },
    @{ Name='additional-pin'; Count=15120; Args=@('--pattern','abandon abandon abandon abandon abandon abandon abandon ? ? ? ? ?','--pool','ability able about above absent absorb abstract absurd abuse') },
    @{ Name='two-sources'; Count=720; Args=@('--post','abandon@1 abandon@2 abandon@3 ability able about','--video','abandon@4 abandon@5 abandon@6 above absent absorb') }
)
try {
    $output = & $exe @common --pattern $pattern --pool $pool --record-search $recordPath 2>&1
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $recordPath)) { throw "Record creation failed: $output" }
    $results = @()
    for ($trial=0; $trial -le $Repeats; $trial++) {
        foreach ($case in $cases) {
            $modes = if ($trial % 2 -eq 0) { @('baseline','subset-proof') } else { @('subset-proof','baseline') }
            foreach ($mode in $modes) {
                $extraArgs = $case.Args
                if ($mode -eq 'subset-proof') { $extraArgs += @('--exclude-record',$recordPath) }
                $timer = [System.Diagnostics.Stopwatch]::StartNew()
                $output = & $exe @common @extraArgs 2>&1
                $timer.Stop()
                $text = $output -join "`n"
                $expected = if ($mode -eq 'baseline') { 'Exhausted search without a match' } else { "$($case.Count) candidates skipped without enumeration" }
                if ($LASTEXITCODE -ne 0 -or -not $text.Contains($expected)) { throw "Unexpected $($case.Name)/$mode output: $output" }
                if ($mode -eq 'subset-proof' -and $text.Contains('Using GPU')) { throw 'Subset proof initialized GPU' }
                if ($trial -gt 0) {
                    $backend = if ($text.Contains('Using GPU (CUDA)')) {'CUDA'} elseif ($text.Contains('Using CPU')) {'CPU'} else {'No engine initialized'}
                    $results += [pscustomobject]@{Trial=$trial; Case=$case.Name; Candidates=$case.Count; Mode=$mode; Seconds=$timer.Elapsed.TotalSeconds.ToString('F6',[cultureinfo]::InvariantCulture); Backend=$backend}
                }
            }
        }
    }
    if ($OutputPath) { $results | Export-Csv -LiteralPath $OutputPath -NoTypeInformation }
    $results | Format-Table -AutoSize
} finally {
    if (Test-Path -LiteralPath $recordPath) { Remove-Item -LiteralPath $recordPath }
}
