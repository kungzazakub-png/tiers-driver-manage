# New-CatalogEntry.ps1 - add or update one driver in driverdb.json.
# Downloads the file (https + built-in host list only), computes SHA-256, reads the digital signature,
# shows you what it found, and writes the entry. Run it only for official vendor links you trust.
#
# Example (NVIDIA game driver, covers every GeForce card):
#   .\New-CatalogEntry.ps1 -Name 'NVIDIA GeForce Driver' -Url 'https://us.download.nvidia.com/.../xxx.exe' -Type installer `
#       -Match 'PCI\VEN_10DE&DEV_*' -Class DISPLAY -Version '32.0.16.2114' -Publisher NVIDIA -SilentArgs '-s -noreboot' -Os 10,11 -Arch x64
param(
    [Parameter(Mandatory)][string]$Name,
    [Parameter(Mandatory)][string]$Url,
    [Parameter(Mandatory)][ValidateSet('installer', 'inf-zip')][string]$Type,
    [Parameter(Mandatory)][string[]]$Match,
    [Parameter(Mandatory)][string]$Version,          # Windows driver version format, e.g. 32.0.16.2114 (what Device Manager shows)
    [string]$Publisher = '',
    [string]$Class = '',                              # device class, e.g. DISPLAY, NET, MEDIA, BLUETOOTH, SYSTEM
    [string[]]$NameLike = @(),                       # only devices whose name fits one of these wildcards, e.g. 'NVIDIA GeForce RTX*'
    [string[]]$Os = @('10', '11'),
    [string[]]$Arch = @('x64'),
    [string]$SilentArgs = '',
    [string]$Signer = '',                             # default: the common name found in the file's signature
    [string]$ExpectedSha512 = '',                     # hash the vendor prints on its download page (Intel does): checked before anything is saved
    [string]$Date = (Get-Date -Format 'yyyy-MM-dd'),
    [string]$Catalog = (Join-Path $PSScriptRoot 'driverdb.json')
)
. (Join-Path $PSScriptRoot 'DriverCore.ps1')
$ErrorActionPreference = 'Stop'

Assert-AllowedUrl $Url
$work = Join-Path $env:TEMP ('dscan_gen_' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $work | Out-Null
try {
    $file = Join-Path $work ([IO.Path]::GetFileName(([uri]$Url).AbsolutePath))
    Write-Output "Downloading $Url ..."
    Invoke-WebRequest -Uri $Url -OutFile $file -UseBasicParsing -TimeoutSec 1800
    if ($ExpectedSha512) {
        $got512 = (Get-FileHash $file -Algorithm SHA512).Hash
        if ($got512 -ne $ExpectedSha512.ToUpperInvariant()) { throw "SHA-512 does not match the vendor's published value (got $got512). Refusing to add this file." }
        Write-Output 'SHA-512 matches the vendor page.'
    }
    $sha = (Get-FileHash $file -Algorithm SHA256).Hash
    $sizeMB = [math]::Round((Get-Item $file).Length / 1MB, 1)

    $signerFound = ''
    if ($Type -eq 'installer') {
        $sig = Get-AuthenticodeSignature $file
        if ($sig.Status -ne 'Valid') { throw "Signature is not valid ($($sig.Status)). Refusing to add this file." }
        $signerFound = $sig.SignerCertificate.Subject
        if (-not $Signer) { $Signer = [regex]::Match($signerFound, 'CN="?([^",]+)').Groups[1].Value }
        if (-not $SilentArgs) { throw 'installer entries need -SilentArgs (silent install switches from the vendor).' }
    }

    Write-Output ''
    Write-Output "  file     : $(Split-Path $file -Leaf)  ($sizeMB MB)"
    Write-Output "  sha256   : $sha"
    Write-Output "  signed by: $signerFound"
    Write-Output "  signer   : $Signer"

    $entry = [ordered]@{
        name = $Name; publisher = $Publisher; match = @($Match); version = $Version; date = $Date; sizeMB = $sizeMB
        type = $Type; url = $Url; sha256 = $sha; os = @($Os); arch = @($Arch)
    }
    if ($Class) { $entry['class'] = $Class }
    if ($NameLike.Count) { $entry['nameLike'] = @($NameLike) }
    if ($Type -eq 'installer') { $entry['signer'] = $Signer; $entry['silentArgs'] = $SilentArgs }

    $existing = @()
    if (Test-Path $Catalog) { $existing = @((([IO.File]::ReadAllText($Catalog, [Text.Encoding]::UTF8)) | ConvertFrom-Json).drivers) | Where-Object { $_.name } }
    $kept = @($existing | Where-Object { $_.name -ne $Name })
    $doc = [ordered]@{ version = 1; updated = (Get-Date -Format 'yyyy-MM-dd'); drivers = @($kept) + @([pscustomobject]$entry) }
    [IO.File]::WriteAllText($Catalog, ($doc | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding($true)))
    Write-Output "Saved '$Name' to $Catalog ($(@($kept).Count + 1) entries)."
} finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
