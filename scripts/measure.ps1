$ErrorActionPreference='Stop'
$projectRoot=Split-Path $PSScriptRoot -Parent
$env:TEMP=Join-Path $projectRoot '.local'
$env:TMP=$env:TEMP
$exe=Join-Path $projectRoot 'target/release/examples/benchmark.exe'
foreach($rows in @(25001,250001)) {
    $log=Join-Path $projectRoot ".local/benchmark-$rows.txt"
    $p=Start-Process -FilePath $exe -ArgumentList "$rows" -WorkingDirectory $projectRoot -WindowStyle Hidden -RedirectStandardOutput $log -PassThru
    $peak=0L
    do {
        $p.Refresh()
        if(-not $p.HasExited){$peak=[math]::Max($peak,$p.PeakWorkingSet64)}
        Start-Sleep -Milliseconds 50
    } while(-not $p.HasExited)
    $p.WaitForExit()
    if($p.ExitCode -ne 0){throw "Benchmark failed: $($p.ExitCode)"}
    [pscustomobject]@{RowsPerShardPerConversation=$rows;PeakWorkingSetMiB=[math]::Round($peak/1MB,2);Result=(Get-Content -LiteralPath $log -Raw).Trim()} | ConvertTo-Json -Compress
    Remove-Item -LiteralPath $log
}
