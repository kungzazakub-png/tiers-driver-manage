# Publishes a 2.x (Rust) release: bumps the version, builds the exe, packs the update zip (the exe only), signs latest.json
# with the same key as 1.x, builds the Setup wizard and (unless -NoUpload) creates the GitHub release.
#   .\Publish-Rust.ps1 -Version 2.0.0 -Notes "What changed" [-NoUpload]
param([Parameter(Mandatory)][string]$Version, [string]$Notes = '', [switch]$NoUpload)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Version must look like 1.2.3' }
$root = $PSScriptRoot; $app = Join-Path $root 'app'; $dist = Join-Path $root 'dist'
$privFile = Join-Path $env:USERPROFILE '.tiers-update\private.xml'
if (-not (Test-Path $privFile)) { throw 'No signing key: run New-UpdateKey.ps1 first' }
$repo = 'kungzazakub-png/tiers-driver-manage'
$env:Path += ";$env:USERPROFILE\.cargo\bin"

# 1) version + build
$toml = Join-Path $app 'Cargo.toml'
$t = [IO.File]::ReadAllText($toml)
$t = [regex]::Replace($t, '(?m)^version = "[\d\.]+"', "version = `"$Version`"", 1)
[IO.File]::WriteAllText($toml, $t, (New-Object Text.UTF8Encoding($false)))
Push-Location $app; $ErrorActionPreference = 'Continue'; try { cargo test --lib --quiet; if ($LASTEXITCODE) { throw 'tests failed' }; cargo build --release --quiet; if ($LASTEXITCODE) { throw 'build failed' } } finally { Pop-Location; $ErrorActionPreference = 'Stop' }
$exe = Join-Path $app 'target\release\tiers-driver-manage.exe'

# 2) update zip: the exe only
New-Item -ItemType Directory -Force $dist | Out-Null
$zip = Join-Path $dist "TierS-Driver-Manage-$Version.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $exe -DestinationPath $zip

# 3) signed manifest (signing code lives in the 1.x engine)
. (Join-Path $root 'DriverScan\DriverCore.ps1')
$m = [pscustomobject]@{ version = $Version; url = "https://github.com/$repo/releases/download/v$Version/TierS-Driver-Manage-$Version.zip"; sha256 = (Get-FileHash $zip -Algorithm SHA256).Hash; signature = ''; notes = $Notes }
$m.signature = New-UpdateSignature $m ([IO.File]::ReadAllText($privFile))
if (-not (Test-UpdateSignature $m ([IO.File]::ReadAllText((Join-Path $app 'assets\UpdatePublicKey.xml'))))) { throw 'Signature does not verify against the embedded public key' }
$manifest = Join-Path $dist 'latest.json'
[IO.File]::WriteAllText($manifest, ($m | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))

# 4) Setup wizard
& "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe" /Q "/DAppVersion=$Version" (Join-Path $root 'TierS-Rust.iss'); if ($LASTEXITCODE) { throw 'Inno Setup failed' }
$setup = Join-Path $dist "TierS-Driver-Manage-Setup-$Version.exe"
$setupLatest = Join-Path $dist 'TierS-Driver-Manage-Setup.exe'; Copy-Item $setup $setupLatest -Force

# 5) GitHub release
$assets = @($zip, $manifest, $setupLatest)
if ($NoUpload) { "Built in $dist (not uploaded):`n  " + ($assets -join "`n  "); return }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw 'GitHub CLI (gh) is not installed' }
& gh release create "v$Version" @assets --repo $repo --title "TierS Driver Manage $Version" --notes $(if ($Notes) { $Notes } else { "Version $Version" })
"Published v$Version to $repo"

