param(
    [Parameter(Mandatory=$true)][string]$Binary,
    [int]$Repeats = 3,
    [string]$OutputPath
)
$ErrorActionPreference = 'Stop'
if ($Repeats -lt 1) { throw 'Repeats must be positive.' }
$exe = (Resolve-Path -LiteralPath $Binary).Path
$prefix = Join-Path ([System.IO.Path]::GetTempPath()) ('adaptive-benchmark-' + [guid]::NewGuid().ToString())
$ownedFiles = [System.Collections.Generic.List[string]]::new()
$previousCache = $env:CUDA_CACHE_PATH
$fixed = 'abandon abandon abandon abandon abandon abandon'
$pool = 'abandon ability able about above absent absorb abstract absurd abuse'
$common = @('0x0000000000000000000000000000000000000000', '--batch-size', '65536', '--pattern', "$fixed ? ? ? ? ? ?", '--pool', $pool, '--metrics')
$cases = @(
    @{ Name='fixed-65536'; Extra=@() },
    @{ Name='adaptive-ceiling-65536'; Extra=@('--adaptive-batch') }
)
$timingFields = @('producer_seconds', 'wait_seconds', 'transfer_seconds', 'filter_seconds', 'derive_seconds', 'checkpoint_seconds')
$countFields = @('completed_raw', 'excluded', 'pruned', 'retained', 'checksum_survivors', 'completed_batches', 'device_batches', 'min_batch_size', 'max_batch_size', 'adaptive_changes')
$requiredFields = @('backend') + $countFields + $timingFields
$expectedSurvivors = $null
$results = @()
try {
    if (-not $env:CUDA_CACHE_PATH) {
        $env:CUDA_CACHE_PATH = Join-Path (Split-Path $PSScriptRoot -Parent) '.cuda/jit-cache'
        New-Item -ItemType Directory -Force -Path $env:CUDA_CACHE_PATH | Out-Null
    }
    Write-Host 'Warming both batch modes; CUDA is required and the first run may compile the driver cache...'
    for ($trial = 0; $trial -le $Repeats; $trial++) {
        $order = @(0, 1)
        if ($trial % 2 -ne 0) { [array]::Reverse($order) }
        foreach ($index in $order) {
            $case = $cases[$index]
            $extra = $case.Extra
            $report = "$prefix-trial$trial-$($case.Name).json"
            $ownedFiles.Add($report)
            $timer = [System.Diagnostics.Stopwatch]::StartNew()
            $output = & $exe @common @extra --metrics-json $report 2>&1
            $exitCode = $LASTEXITCODE
            $timer.Stop()
            $text = $output -join "`n"
            if ($exitCode -ne 0 -or -not (Test-Path -LiteralPath $report) -or
                -not $text.Contains('Exhausted search without a match: 151200 candidates')) {
                throw "Unexpected result for $($case.Name), trial ${trial}:`n$text"
            }
            $metrics = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
            foreach ($field in $requiredFields) {
                if ($metrics.PSObject.Properties.Name -notcontains $field -or $null -eq $metrics.$field) {
                    throw "Missing metric '$field' for $($case.Name)."
                }
            }
            if (-not $text.Contains('Using GPU (CUDA)') -or $metrics.backend -ne 'CUDA') {
                throw "CUDA unavailable; refusing to benchmark a CPU fallback.`n$text"
            }
            if ($metrics.completed_raw -ne 151200 -or $metrics.excluded -ne 0 -or $metrics.pruned -ne 0 -or $metrics.retained -ne 151200) {
                throw "Incorrect candidate conservation for $($case.Name)."
            }
            if ($metrics.checksum_survivors -le 0 -or $metrics.checksum_survivors -gt 151200) {
                throw "Invalid checksum survivor count for $($case.Name)."
            }
            if ($null -eq $expectedSurvivors) { $expectedSurvivors = $metrics.checksum_survivors }
            if ($metrics.checksum_survivors -ne $expectedSurvivors) {
                throw "Checksum survivor counts differ between benchmark executions."
            }
            if ($metrics.completed_batches -lt 1 -or $metrics.device_batches -lt 1 -or
                $metrics.min_batch_size -lt 1 -or $metrics.max_batch_size -gt 65536 -or
                $metrics.min_batch_size -gt $metrics.max_batch_size -or $metrics.adaptive_changes -lt 0) {
                throw "Invalid batch metrics for $($case.Name)."
            }
            if ($case.Name -eq 'fixed-65536' -and $metrics.adaptive_changes -ne 0) {
                throw 'Fixed batch mode unexpectedly reports adaptive changes.'
            }
            foreach ($field in $timingFields) {
                $seconds = [double]$metrics.$field
                if ([double]::IsNaN($seconds) -or [double]::IsInfinity($seconds) -or $seconds -lt 0) {
                    throw "Invalid stage timing '$field' for $($case.Name)."
                }
            }
            if ($trial -gt 0) {
                $row = [ordered]@{
                    Trial = $trial
                    Case = $case.Name
                    Seconds = $timer.Elapsed.TotalSeconds.ToString('F6', [cultureinfo]::InvariantCulture)
                    Backend = $metrics.backend
                }
                foreach ($field in $countFields) { $row[$field] = $metrics.$field }
                foreach ($field in $timingFields) {
                    $row[$field] = ([double]$metrics.$field).ToString('F9', [cultureinfo]::InvariantCulture)
                }
                $results += [pscustomobject]$row
                if ($OutputPath) { $results | Export-Csv -LiteralPath $OutputPath -NoTypeInformation }
            }
            Write-Host "trial=$trial case=$($case.Name) seconds=$($timer.Elapsed.TotalSeconds.ToString('F6', [cultureinfo]::InvariantCulture)) deviceBatches=$($metrics.device_batches) sizes=$($metrics.min_batch_size)..$($metrics.max_batch_size) changes=$($metrics.adaptive_changes)"
        }
    }
    $results | Format-Table -AutoSize
} finally {
    $env:CUDA_CACHE_PATH = $previousCache
    foreach ($path in $ownedFiles) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
}
