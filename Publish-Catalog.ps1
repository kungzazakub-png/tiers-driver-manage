# Publishes the driver catalog (DriverScan\driverdb.json) as a signed online catalog. Installed copies fetch it at every scan,
# so catalog changes reach every PC without a new program release.   .\Publish-Catalog.ps1 [-NoUpload]
param([switch]$NoUpload)
$ErrorActionPreference = 'Stop'
$src = Join-Path $PSScriptRoot 'DriverScan'
. (Join-Path $src 'DriverCore.ps1')
$privFile = Join-Path $env:USERPROFILE '.tiers-update\private.xml'
if (-not (Test-Path $privFile)) { throw 'No signing key: run New-UpdateKey.ps1 first' }
$repo = [string](Get-Content (Join-Path $src 'update.json') -Raw | ConvertFrom-Json).repo
if ($repo -notmatch '^[\w\.\-]+/[\w\.\-]+$') { throw 'Set "repo" in DriverScan\update.json' }

# every entry must pass the same checks the app applies (an entry the app rejects would make the whole online catalog unusable)
$doc = [IO.File]::ReadAllText((Join-Path $src 'driverdb.json'), [Text.Encoding]::UTF8) | ConvertFrom-Json
$bad = @(@($doc.drivers) | ForEach-Object { $why = Test-CatalogEntry $_; if ($why) { "$($_.name): $why" } })
if ($bad.Count) { throw ("Catalog has invalid entries:`n  " + ($bad -join "`n  ")) }

$revFile = Join-Path $PSScriptRoot 'catalog.rev'
$rev = 1 + $(if (Test-Path $revFile) { [int](Get-Content $revFile -Raw) } else { 0 })
$dist = Join-Path $PSScriptRoot 'dist\catalog'
New-Item -ItemType Directory -Force $dist | Out-Null
$file = Join-Path $dist 'driverdb.json'
Copy-Item (Join-Path $src 'driverdb.json') $file -Force
$m = [pscustomobject]@{ version = "1.$rev"; url = "https://github.com/$repo/releases/download/catalog/driverdb.json"; sha256 = (Get-FileHash $file -Algorithm SHA256).Hash; signature = '' }
$m.signature = New-UpdateSignature $m ([IO.File]::ReadAllText($privFile))
$manifest = Join-Path $dist 'catalog.json'
[IO.File]::WriteAllText($manifest, ($m | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
if (-not (Test-UpdateSignature $m ([IO.File]::ReadAllText((Join-Path $src 'UpdatePublicKey.xml'))))) { throw 'Signature does not verify against UpdatePublicKey.xml' }
if ($NoUpload) { "Built revision $rev in $dist (not uploaded)."; return }
$gh = (Get-Command gh -ErrorAction SilentlyContinue).Source; if (-not $gh) { throw 'GitHub CLI (gh) is not installed or not on PATH' }
# a prerelease keeps the "latest release" (used by program updates) pointing at the program
$ErrorActionPreference = 'Continue'; & $gh release view catalog --repo $repo 2>$null | Out-Null; $exists = ($LASTEXITCODE -eq 0); $ErrorActionPreference = 'Stop'
if (-not $exists) { & $gh release create catalog --repo $repo --prerelease --title 'Driver catalog' --notes 'Signed online driver catalog (fetched by the program at every scan).' }
& $gh release upload catalog $file $manifest --repo $repo --clobber
Set-Content $revFile $rev
"Published catalog revision $rev to $repo"
