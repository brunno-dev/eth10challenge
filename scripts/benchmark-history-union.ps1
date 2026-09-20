param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [int]$Repeats = 3,
    [string]$OutputPath,
    [switch]$Cpu
)
$ErrorActionPreference = 'Stop'
if ($Repeats -lt 1) { throw 'Repeats must be positive' }
$exe = (Resolve-Path -LiteralPath $Binary).Path
$prefix = Join-Path ([System.IO.Path]::GetTempPath()) ('union-benchmark-' + [guid]::NewGuid().ToString())
$common = @('0x0000000000000000000000000000000000000000','--batch-size','65536')
if ($Cpu) { $common += '--cpu' }
$fixed = 'abandon abandon abandon abandon abandon abandon'
$words = @('abandon','ability','able','about','above','absent','absorb','abstract','absurd','abuse')
$query = @('--pattern',"$fixed ? ? ? ? ? ?",'--pool',($words -join ' '))
$records = @()
try {
    foreach ($word in $words) {
        $record = "$prefix-$word.json"
        $records += $record
        $rest = ($words | Where-Object {$_ -ne $word}) -join ' '
        $output = & $exe @common --pattern "$fixed $word ? ? ? ? ?" --pool $rest --record-search $record 2>&1
        if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $record)) { throw "Record creation failed: $output" }
    }
    $unionArgs = @($records | ForEach-Object { '--exclude-record'; $_ })
    $partialArgs = @($records[0..8] | ForEach-Object { '--exclude-record'; $_ })
    $cases = @(
        @{Name='baseline'; Extra=@(); Expected='Exhausted search without a match'},
        @{Name='complete-union'; Extra=$unionArgs; Expected='151200 candidates skipped without enumeration'},
        @{Name='missing-one-record'; Extra=$partialArgs; Expected='History excluded 136080 arrangements'}
    )
    $results = @()
    for ($trial=0; $trial -le $Repeats; $trial++) {
        $order = @(0,1,2)
        if ($trial % 2 -ne 0) { [array]::Reverse($order) }
        foreach ($index in $order) {
            $case = $cases[$index]; $extra = $case.Extra
            $timer = [System.Diagnostics.Stopwatch]::StartNew()
            $output = & $exe @common @query @extra 2>&1
            $timer.Stop()
            $text = $output -join "`n"
            if ($LASTEXITCODE -ne 0 -or -not $text.Contains($case.Expected)) { throw "Unexpected $($case.Name): $output" }
            if ($trial -gt 0) {
                $backend = if ($text.Contains('Using GPU (CUDA)')) {'CUDA'} elseif ($text.Contains('Using CPU')) {'CPU'} else {'No engine initialized'}
                $results += [pscustomobject]@{Trial=$trial; Case=$case.Name; Seconds=$timer.Elapsed.TotalSeconds.ToString('F6',[cultureinfo]::InvariantCulture); Backend=$backend}
            }
        }
    }
    if ($OutputPath) { $results | Export-Csv -LiteralPath $OutputPath -NoTypeInformation }
    $results | Format-Table -AutoSize
} finally {
    foreach ($record in $records) { if (Test-Path -LiteralPath $record) { Remove-Item -LiteralPath $record } }
}
