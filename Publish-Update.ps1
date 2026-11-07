# Publishes a new version: bumps the version, packs the update zip, signs latest.json, rebuilds the Setup wizard and
# (when GitHub CLI is installed and logged in) creates the GitHub release. Installed copies pick it up the next time they start.
#   .\Publish-Update.ps1 -Version 1.0.1 -Notes "What changed"            (add -NoUpload to only build into dist\)
param([Parameter(Mandatory)][string]$Version, [string]$Notes = '', [switch]$NoUpload)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Version must look like 1.2.3' }
$src = Join-Path $PSScriptRoot 'DriverScan'
$dist = Join-Path $PSScriptRoot 'dist'
$privFile = Join-Path $env:USERPROFILE '.tiers-update\private.xml'
if (-not (Test-Path $privFile)) { throw 'No signing key: run New-UpdateKey.ps1 first' }
$repo = [string](Get-Content (Join-Path $src 'update.json') -Raw | ConvertFrom-Json).repo
if ($repo -notmatch '^[\w\.\-]+/[\w\.\-]+$') { throw 'Set "repo" in DriverScan\update.json (owner/name of the GitHub repository)' }

# 1) version number inside the program
$main = Join-Path $src 'DriverScan.ps1'
$text = [IO.File]::ReadAllText($main)
$text = [regex]::Replace($text, "AppVersion = '[\d\.]+'", "AppVersion = '$Version'")
[IO.File]::WriteAllText($main, $text, (New-Object Text.UTF8Encoding($true)))

# 2) update zip: scripts and assets only (the .exe, the user's catalog and settings are never part of an update)
New-Item -ItemType Directory -Force $dist | Out-Null
$zip = Join-Path $dist "TierS-Driver-Manage-$Version.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
# Binary assets (logo, icon) are NOT part of an update: older installed versions keep them open, which made their updater fail. Setup ships them.
$files = 'DriverScan.ps1', 'DriverCore.ps1', 'DriverUI.ps1', 'New-CatalogEntry.ps1', 'README.txt' | ForEach-Object { Join-Path $src $_ }
Compress-Archive -Path $files -DestinationPath $zip

# 3) signed manifest
. (Join-Path $src 'DriverCore.ps1')
$m = [pscustomobject]@{ version = $Version; url = "https://github.com/$repo/releases/download/v$Version/TierS-Driver-Manage-$Version.zip"; sha256 = (Get-FileHash $zip -Algorithm SHA256).Hash; signature = ''; notes = $Notes }
$m.signature = New-UpdateSignature $m ([IO.File]::ReadAllText($privFile))
$manifest = Join-Path $dist 'latest.json'
[IO.File]::WriteAllText($manifest, ($m | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
if (-not (Test-UpdateSignature $m ([IO.File]::ReadAllText((Join-Path $src 'UpdatePublicKey.xml'))))) { throw 'Signature does not verify against UpdatePublicKey.xml (wrong key pair?)' }

# 4) Setup wizard for new installs
& (Join-Path $PSScriptRoot 'Build-Setup.ps1') -Version $Version
Copy-Item (Join-Path $PSScriptRoot 'TierS-Driver-Manage-Setup.exe') (Join-Path $dist 'TierS-Driver-Manage-Setup.exe') -Force

# 5) GitHub release
$assets = @($zip, $manifest, (Join-Path $dist 'TierS-Driver-Manage-Setup.exe'))
if ($NoUpload) { "Built in $dist (not uploaded). Upload $($assets.Count) files to the release v$Version of $repo."; return }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { "Built in $dist. GitHub CLI (gh) is not installed: create release v$Version on https://github.com/$repo/releases/new and attach these files:`n  " + ($assets -join "`n  "); return }
$ErrorActionPreference = 'Continue'; & gh release view "v$Version" --repo $repo 2>$null | Out-Null; $exists = ($LASTEXITCODE -eq 0); $ErrorActionPreference = 'Stop'
if ($exists) { & gh release upload "v$Version" @assets --repo $repo --clobber } else { & gh release create "v$Version" @assets --repo $repo --title "TierS Driver Manage $Version" --notes $(if ($Notes) { $Notes } else { "Version $Version" }) }
"Published v$Version to $repo"
