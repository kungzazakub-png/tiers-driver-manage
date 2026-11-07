# Stores a driver file in the publisher's own mirror (release tag "drivers" of the update repository) and prints the URL to use with
# New-CatalogEntry.ps1.  For drivers a vendor only offers behind a CAPTCHA/form: download it yourself, check the vendor's license
# allows redistribution, then run this.     .\Publish-Driver.ps1 -File C:\path\Realtek-xxxx.zip
param([Parameter(Mandatory)][string]$File)
$ErrorActionPreference = 'Stop'
$item = Get-Item $File
$repo = [string](Get-Content (Join-Path $PSScriptRoot 'DriverScan\update.json') -Raw | ConvertFrom-Json).repo
if ($repo -notmatch '^[\w\.\-]+/[\w\.\-]+$') { throw 'Set "repo" in DriverScan\update.json' }
"File      : $($item.Name) ($([math]::Round($item.Length / 1MB, 1)) MB)"
"SHA-256   : $((Get-FileHash $item.FullName -Algorithm SHA256).Hash)"
if ($item.Extension -in '.exe', '.msi', '.dll', '.sys') { $s = Get-AuthenticodeSignature $item.FullName; "Signature  : $($s.Status)  $($s.SignerCertificate.Subject)" }
if ($item.Length -ge 2GB) { throw 'GitHub release assets are limited to 2 GB' }
$gh = (Get-Command gh -ErrorAction SilentlyContinue).Source; if (-not $gh) { throw 'GitHub CLI (gh) is not installed or not on PATH' }
& $gh release view drivers --repo $repo 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) { & $gh release create drivers --repo $repo --prerelease --title 'Driver mirror' --notes 'Driver files mirrored by the publisher (hash-pinned in the catalog).' }
& $gh release upload drivers $item.FullName --repo $repo --clobber
"URL       : https://github.com/$repo/releases/download/drivers/$([uri]::EscapeDataString($item.Name))"
