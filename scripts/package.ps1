param([switch]$NoBuild)
$ErrorActionPreference='Stop'
$projectRoot=Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    if(-not $NoBuild){& cargo build --locked --release; if($LASTEXITCODE -ne 0){throw 'Release build failed'}}
    $destination=Join-Path $projectRoot 'dist/wechat-export-windows-x64'
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'target/release/wechat-export.exe') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $projectRoot 'README.md'),(Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md') -Destination $destination
    New-Item -ItemType Directory -Path (Join-Path $destination 'docs'),(Join-Path $destination 'assets') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'docs/reference-analysis.md'),(Join-Path $projectRoot 'docs/validation.md') -Destination (Join-Path $destination 'docs')
    Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/README.md'),(Join-Path $projectRoot 'assets/app.ico'),(Join-Path $projectRoot 'assets/app.png'),(Join-Path $projectRoot 'assets/icon-source.png') -Destination (Join-Path $destination 'assets')
    Copy-Item -LiteralPath (Join-Path $projectRoot 'licenses') -Destination $destination -Recurse -Force
    $hash=Get-FileHash -LiteralPath (Join-Path $destination 'wechat-export.exe') -Algorithm SHA256
    "$($hash.Hash.ToLower())  wechat-export.exe" | Set-Content -LiteralPath (Join-Path $destination 'SHA256SUMS.txt') -Encoding ascii
    Compress-Archive -Path $destination -DestinationPath (Join-Path $projectRoot 'dist/wechat-export-windows-x64.zip') -Force
    Get-Item -LiteralPath (Join-Path $destination 'wechat-export.exe'),(Join-Path $projectRoot 'dist/wechat-export-windows-x64.zip') | Select-Object FullName,Length
} finally {Pop-Location}
