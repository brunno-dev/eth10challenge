param([string]$Engine = (Join-Path (Split-Path $PSScriptRoot -Parent) '../gpu-target/release/words-breaker.exe'))
$ErrorActionPreference = 'Stop'
$workspace = Split-Path $PSScriptRoot -Parent
$testRoot = Join-Path $workspace ('output/history-check-' + [guid]::NewGuid().ToString('N'))
$globalRecords = Join-Path $testRoot 'records'
$baseRecords = Join-Path $testRoot 'base'
New-Item -ItemType Directory -Force -Path $globalRecords,$baseRecords | Out-Null
$targetAddress = '0x0000000000000000000000000000000000000000'
$fixedNine = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon'
$basePattern = "$fixedNine ? ? ?"
$pool = 'ability able about'
function Invoke-Engine([string[]]$EngineArgs) {
    $lines = & $Engine @EngineArgs 2>&1
    if ($LASTEXITCODE -ne 0) { throw ($lines -join "`n") }
    return ($lines -join "`n")
}
function Assert-Coverage([string]$Pattern, [string]$Words, [bool]$Expected, [string]$Address = $targetAddress) {
    $result = Invoke-Engine @($Address,'--pattern',$Pattern,'--pool',$Words,'--exclude-record-dir',$globalRecords,'--preflight-history')
    $line = $result -split "`n" | Where-Object { $_.StartsWith('History preflight: ') } | Select-Object -Last 1
    if (-not $line) { throw 'Missing coverage response' }
    $report = $line.Substring('History preflight: '.Length) | ConvertFrom-Json
    if ($report.fullyCovered -ne $Expected) { throw "Unexpected coverage: $Pattern / $Words -> $line" }
}
$pinnedCheckpoint = Join-Path $testRoot 'pinned-checkpoint.json'
$pinnedRecord = Join-Path $globalRecords 'pinned.json'
Invoke-Engine @($targetAddress,'--pattern',"$fixedNine ability ? ?",'--pool','able about','--cpu','--checkpoint',$pinnedCheckpoint,'--record-progress',$pinnedRecord) | Out-Null
Copy-Item -LiteralPath $pinnedRecord -Destination $baseRecords
Assert-Coverage "$fixedNine ability ? ?" 'able about' $true
Assert-Coverage $basePattern $pool $false
$checkpointFile = Join-Path $testRoot 'partial-checkpoint.json'
$progressRecord = Join-Path $globalRecords 'partial.json'
Invoke-Engine @($targetAddress,'--pattern',$basePattern,'--pool',$pool,'--cpu','--batch-size','1','--max-candidates','3','--exclude-record-dir',$baseRecords,'--checkpoint',$checkpointFile,'--record-progress',$progressRecord) | Out-Null
$partial = Get-Content -LiteralPath $checkpointFile -Raw | ConvertFrom-Json
if ($partial.checked -ne 3 -or $partial.excluded -ne 2) { throw 'Partial search did not reuse the two pinned candidates' }
Assert-Coverage "$fixedNine able ability about" '' $true
Assert-Coverage "$fixedNine able about ability" '' $false
Invoke-Engine @($targetAddress,'--pattern',"$fixedNine able about ability",'--cpu','--record-progress',(Join-Path $globalRecords 'additional.json')) | Out-Null
Invoke-Engine @($targetAddress,'--pattern',$basePattern,'--pool',$pool,'--cpu','--batch-size','1','--exclude-record-dir',$baseRecords,'--additional-record-dir',$globalRecords,'--checkpoint',$checkpointFile,'--resume','--record-progress',$progressRecord) | Out-Null
$completed = Get-Content -LiteralPath $checkpointFile -Raw | ConvertFrom-Json
if ($completed.checked -ne 6 -or $completed.excluded -ne 3) { throw 'Resume failed to reuse new compatible history while preserving its cursor' }
Assert-Coverage $basePattern 'about ability able' $true
Assert-Coverage $basePattern 'ability able about above' $false
Assert-Coverage $basePattern $pool $false '0x0000000000000000000000000000000000000001'
$expandedCheckpoint = Join-Path $testRoot 'expanded-checkpoint.json'
Invoke-Engine @($targetAddress,'--pattern',$basePattern,'--pool','ability able about above','--cpu','--exclude-record-dir',$globalRecords,'--checkpoint',$expandedCheckpoint,'--record-progress',(Join-Path $globalRecords 'expanded.json')) | Out-Null
$expanded = Get-Content -LiteralPath $expandedCheckpoint -Raw | ConvertFrom-Json
if ($expanded.checked -ne 24 -or $expanded.excluded -ne 6) { throw 'Expanded search failed to filter exactly the prior six arrangements' }
$exportFile = Join-Path $testRoot 'exported-legacy.json'
Invoke-Engine @($targetAddress,'--pattern',"$fixedNine ability ? ?",'--pool','able about','--checkpoint',$pinnedCheckpoint,'--export-checkpoint-record',$exportFile) | Out-Null
if (-not (Test-Path -LiteralPath $exportFile)) { throw 'Legacy checkpoint export missing' }
$additionalCheckpoint = Join-Path $testRoot 'additional-only-checkpoint.json'
Invoke-Engine @($targetAddress,'--pattern',$basePattern,'--pool','ability able about above','--cpu','--max-candidates','1','--checkpoint',$additionalCheckpoint) | Out-Null
Invoke-Engine @($targetAddress,'--pattern',$basePattern,'--pool','ability able about above','--cpu','--resume','--checkpoint',$additionalCheckpoint,'--additional-record-dir',$globalRecords) | Out-Null
$additional = Get-Content -LiteralPath $additionalCheckpoint -Raw | ConvertFrom-Json
if ($additional.checked -ne 24 -or $additional.excluded -ne 23) { throw 'Additional-only resume lost original checkpoint policy' }
Invoke-Engine @($targetAddress,'--pattern',$basePattern,'--pool','ability able about above','--checkpoint',$additionalCheckpoint,'--export-checkpoint-record',(Join-Path $testRoot 'exported-additional-only.json')) | Out-Null
[pscustomobject]@{result='passed';partialChecked=$partial.checked;partialExcluded=$partial.excluded;resumedChecked=$completed.checked;resumedExcluded=$completed.excluded;expandedChecked=$expanded.checked;expandedExcluded=$expanded.excluded;artifacts=$testRoot} | ConvertTo-Json
