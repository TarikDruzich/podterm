# Requires PowerShell 7+ (default on GitHub Windows runners).
$ErrorActionPreference = 'Stop'
Set-Location (Resolve-Path (Join-Path $PSScriptRoot '..'))
$version = ((Get-Content Cargo.toml | Select-String '^version = "([^\"]+)"' | Select-Object -First 1).Matches.Groups[1].Value)
if (-not $version) { throw 'Cannot read Cargo.toml version' }
$binary = 'target\release\podterm-rs.exe'
if (!(Test-Path $binary)) { throw "Missing executable: $binary" }
$archive = "podterm-rs-v$version-windows-x86_64.zip"
if (Test-Path $archive) { Remove-Item $archive }
Compress-Archive -Path $binary, 'WINDOWS.md', 'LICENSE' -DestinationPath $archive
Write-Host "Created: $archive"
