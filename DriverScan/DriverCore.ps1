# DriverCore.ps1 - shared engine for DriverScan.ps1 and New-CatalogEntry.ps1 (also loaded into worker runspaces).
$ProgressPreference = 'SilentlyContinue'   # progress bars make Invoke-WebRequest very slow on big files

# Download hosts the tool will ever accept. The catalog file can NOT widen this list (a catalog is easy to tamper with).
# A host matches itself and its subdomains. Add a vendor here only after you trust it.
$BuiltInHosts = @('nvidia.com', 'intel.com', 'windowsupdate.com')   # windowsupdate.com = Microsoft Update Catalog downloads

function ConvertTo-Ver($s) {
    try { return [version](([string]$s) -replace '[^\d\.].*$', '') } catch { return $null }
}

function Get-HostInfo {
    $arch = switch ($env:PROCESSOR_ARCHITECTURE) { 'AMD64' { 'x64' } 'ARM64' { 'arm64' } 'x86' { 'x86' } default { "$env:PROCESSOR_ARCHITECTURE".ToLower() } }
    $os = if ([Environment]::OSVersion.Version.Build -ge 22000) { '11' } else { '10' }
    return [pscustomobject]@{ Arch = $arch; Os = $os }
}

function Get-InstalledDrivers {
    Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DeviceName -and $_.DriverVersion } | ForEach-Object {
        # Windows only returns the most specific hardware ID here, so catalog patterns should look like PCI\VEN_10DE&DEV_* plus a "class".
        $ids = @($_.HardwareID, $_.CompatID) | Where-Object { $_ }
        [pscustomobject]@{
            InstanceId = $_.DeviceID
            Name       = $_.DeviceName
            Class      = $_.DeviceClass
            Maker      = $_.Manufacturer
            Version    = $_.DriverVersion
            Date       = $_.DriverDate
            HardwareId = @($ids)
        }
    }
}

# Win32_PnPSignedDriver can keep showing the OLD version after an install, so offers are re-checked against Plug and Play.
function Get-LiveDriverVersion($instanceId) {
    if (-not $instanceId) { return $null }
    try { return (Get-PnpDeviceProperty -InstanceId $instanceId -KeyName 'DEVPKEY_Device_DriverVersion' -ErrorAction Stop).Data } catch { return $null }
}

function Read-DriverCatalog($pathOrUrl) {
    $raw = $null
    if ($pathOrUrl -match '^https://') { $raw = (Invoke-WebRequest -Uri $pathOrUrl -UseBasicParsing -TimeoutSec 20).Content }
    elseif ($pathOrUrl -and (Test-Path $pathOrUrl)) { $raw = [IO.File]::ReadAllText($pathOrUrl, [Text.Encoding]::UTF8) }
    else { return @() }
    $j = $raw | ConvertFrom-Json
    return @(@($j.drivers) | Where-Object { $_.name })
}

# A catalog entry applies when: its os/arch (if given) fit this PC, one "match" wildcard fits an installed device's
# hardware/compatible ID, the device class equals "class" (if given), AND the catalog version is newer than installed.
function Find-CatalogUpdates($installed, $catalog, $hostInfo = (Get-HostInfo)) {
    $found = @()
    foreach ($c in $catalog) {
        $newV = ConvertTo-Ver $c.version
        if (-not $newV) { continue }
        if ($c.os -and (@($c.os) -notcontains $hostInfo.Os)) { continue }
        if ($c.arch -and (@($c.arch) -notcontains $hostInfo.Arch)) { continue }
        foreach ($d in $installed) {
            if ($c.class -and $d.Class -ne $c.class) { continue }
            if ($c.nameLike -and -not @(@($c.nameLike) | Where-Object { $d.Name -like $_ }).Count) { continue }
            $hit = $false
            foreach ($pat in @($c.match)) {
                foreach ($hid in $d.HardwareId) { if ($hid -and $pat -and ($hid -like $pat)) { $hit = $true; break } }
                if ($hit) { break }
            }
            if (-not $hit) { continue }
            $curV = ConvertTo-Ver $d.Version
            $live = ConvertTo-Ver (Get-LiveDriverVersion $d.InstanceId); if ($live) { $curV = $live }
            if ($curV -and $newV -le $curV) { continue }
            $found += [pscustomobject]@{
                Id = "cat:$($c.name) $($c.version)"; Title = "$($c.name) $($c.version)"; Class = $d.Class; Maker = $c.publisher
                Date = $c.date; SizeMB = $c.sizeMB; Source = 'Catalog'; Raw = $c; InstanceId = $d.InstanceId; DeviceName = $d.Name
            }
            break
        }
    }
    return $found
}

# The publisher's own mirror (GitHub release assets of this one repository, NOT all of github.com). Drivers whose license or download form
# prevents a direct vendor link are stored there; they are still hash-pinned and signature-checked like every other download.
$MirrorPrefixes = @('https://github.com/kungzazakub-png/tiers-driver-manage/releases/download/')

function Assert-AllowedUrl($url, $hosts = $BuiltInHosts) {
    $u = [uri]$url
    if ($u.Scheme -ne 'https') { throw "Links that are not https are not allowed: $url" }
    if ([object]::ReferenceEquals($hosts, $BuiltInHosts)) { foreach ($pre in $MirrorPrefixes) { if ($url.StartsWith($pre, [StringComparison]::OrdinalIgnoreCase)) { return } } }
    $ok = $false
    foreach ($h in @($hosts | Where-Object { $_ })) { if ($u.Host -eq $h -or $u.Host.EndsWith('.' + $h)) { $ok = $true } }
    if (-not $ok) { throw "Host $($u.Host) is not in the program's allowed list" }
}

function Test-FileSha256($path, $expected) {
    if (-not $expected) { throw 'The catalog entry has no sha256 value, so it will not be installed' }
    $actual = (Get-FileHash -Path $path -Algorithm SHA256).Hash
    if ($actual -ne ([string]$expected).ToUpperInvariant()) {
        throw "SHA-256 mismatch (got $actual); the file may have been modified or corrupted"
    }
    return $true
}

function Expand-SafeZip($zip, $dest) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    New-Item -ItemType Directory -Force $dest | Out-Null
    $destFull = [IO.Path]::GetFullPath($dest.TrimEnd('\') + '\')
    $z = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($e in $z.Entries) {
            $target = [IO.Path]::GetFullPath((Join-Path $dest $e.FullName))
            if (-not $target.StartsWith($destFull, [StringComparison]::OrdinalIgnoreCase)) { throw "Unsafe path found in zip file: $($e.FullName)" }
        }
    } finally { $z.Dispose() }
    Expand-Archive -Path $zip -DestinationPath $dest -Force
}

# $localFile is for tests: use an already-downloaded file instead of fetching $c.url (the URL is still checked).
function Install-CatalogItem($c, $workDir, $localFile = $null) {
    $res = [pscustomobject]@{ Success = $false; Reboot = $false; Message = '' }
    try {
        Assert-AllowedUrl $c.url
        New-Item -ItemType Directory -Force $workDir | Out-Null
        # Work folder: Administrators and SYSTEM only, so no other program can swap the file after it is checked.
        $isAdmin = (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if ($isAdmin) { & icacls.exe $workDir /inheritance:r /grant:r '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' | Out-Null }
        $fileName = [IO.Path]::GetFileName(([uri]$c.url).AbsolutePath)
        if (-not $fileName) { $fileName = 'download.bin' }
        $file = Join-Path $workDir $fileName
        if ($localFile) { Copy-Item $localFile $file -Force } else { Invoke-WebRequest -Uri $c.url -OutFile $file -UseBasicParsing -TimeoutSec 600 }
        [void](Test-FileSha256 $file $c.sha256)

        if ($c.type -eq 'inf-zip') {
            $pkg = Join-Path $workDir 'pkg'
            Expand-SafeZip $file $pkg
            $out = & pnputil.exe /add-driver (Join-Path $pkg '*.inf') /subdirs /install 2>&1 | Out-String
            $code = $LASTEXITCODE
            $res.Message = $out.Trim()
            $res.Success = ($code -eq 0 -or $code -eq 3010)
            $res.Reboot = ($code -eq 3010) -or ($out -match 'reboot|restart')
        } elseif ($c.type -eq 'installer') {
            if (-not $c.signer) { throw 'An installer entry must specify a signer (publisher name) to be installed' }
            if (-not $c.silentArgs) { throw 'An installer entry must specify silentArgs' }
            $sig = Get-AuthenticodeSignature -FilePath $file
            if ($sig.Status -ne 'Valid') { throw "The installer's digital signature is not valid ($($sig.Status))" }
            if ($sig.SignerCertificate.Subject -notlike "*$($c.signer)*") { throw "Signer does not match the expected publisher: $($sig.SignerCertificate.Subject)" }
            $p = Start-Process -FilePath $file -ArgumentList $c.silentArgs -Wait -PassThru
            $res.Success = ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010)
            $res.Reboot = ($p.ExitCode -eq 3010)
            $res.Message = "exit code $($p.ExitCode)"
        } else {
            throw "Unknown type: $($c.type) (use inf-zip or installer)"
        }
    } catch {
        $res.Message = $_.Exception.Message
    }
    return $res
}

# ---------- Microsoft Update Catalog provider: finds drivers by hardware ID (signed by Microsoft/WHQL) ----------
# Unofficial: reads the catalog's web pages, so a layout change on Microsoft's side can break it (it then finds nothing).
function Get-CatalogHardwareKey($hid) {
    foreach ($re in 'PCI\\VEN_[0-9A-F]{4}&DEV_[0-9A-F]{4}', 'USB\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}', 'HDAUDIO\\FUNC_\d\d&VEN_[0-9A-F]{4}&DEV_[0-9A-F]{4}') {
        $m = [regex]::Match([string]$hid, '^' + $re, 'IgnoreCase')
        if ($m.Success) { return $m.Value.ToUpperInvariant() }
    }
    return $null
}

function ConvertFrom-MsCatalogHtml($html) {
    foreach ($r in [regex]::Matches($html, '(?s)<tr id="([0-9a-f\-]{36})_R\d+"[^>]*>(.*?)</tr>')) {
        $cells = @([regex]::Matches($r.Groups[2].Value, '(?s)<td[^>]*>(.*?)</td>') | ForEach-Object {
            ([Net.WebUtility]::HtmlDecode([regex]::Replace($_.Groups[1].Value, '<[^>]+>', '')) -replace '\s+', ' ').Trim()
        })
        if ($cells.Count -lt 7) { continue }
        $date = $null
        try { $date = [datetime]::ParseExact($cells[4], 'M/d/yyyy', [Globalization.CultureInfo]::InvariantCulture) } catch {}
        $mb = $null
        $sz = [regex]::Match($cells[6], '^([\d\.,]+)\s*(KB|MB|GB)')
        if ($sz.Success) { $n = [double]($sz.Groups[1].Value -replace ',', ''); $mb = [math]::Round($n * @{ KB = 1 / 1024; MB = 1; GB = 1024 }[$sz.Groups[2].Value], 2) }
        [pscustomobject]@{ UpdateId = $r.Groups[1].Value; Title = $cells[1]; Products = $cells[2]; Class = $cells[3]; Date = $date; Version = $cells[5]; SizeMB = $mb }
    }
}

function Search-MsCatalog($key) {
    $url = 'https://www.catalog.update.microsoft.com/Search.aspx?q=' + [uri]::EscapeDataString($key)
    return @(Get-MsCatalogRows (Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 30).Content)
}

# Rows, or an empty list for a genuine "no results" page. Any other page means Microsoft changed it (or blocked us): fail loudly.
function Get-MsCatalogRows($html) {
    $rows = @(ConvertFrom-MsCatalogHtml $html)
    if ($rows.Count -eq 0 -and $html -notmatch 'did not find any results') { throw 'Unrecognised Microsoft Update Catalog search page format (the page may have changed or is blocked)' }
    return $rows
}

# Newest driver row that fits this Windows (11 also accepts Windows 10 drivers).
function Select-BestCatalogRow($rows, $hostInfo) {
    $osRe = if ($hostInfo.Os -eq '11') { 'Windows (11|10)' } else { 'Windows 10' }
    @($rows | Where-Object { $_.Class -like 'Drivers*' -and $_.Products -match $osRe -and (ConvertTo-Ver $_.Version) }) |
        Sort-Object @{ e = { ConvertTo-Ver $_.Version }; Descending = $true }, @{ e = { $_.Date }; Descending = $true } |
        Select-Object -First 1
}

# Asks the catalog about every non-Microsoft device (one query per distinct hardware ID, at most $maxQueries).
function Find-MsCatalogUpdates($installed, $hostInfo = (Get-HostInfo), $maxQueries = 40, $search = { param($k) Search-MsCatalog $k }) {
    $byKey = [ordered]@{}
    foreach ($d in $installed) {
        if ($d.Maker -like 'Microsoft*' -or $d.Maker -like '(Standard*' -or $d.Maker -like 'Standard*') { continue }
        $key = $null
        foreach ($hid in $d.HardwareId) { $key = Get-CatalogHardwareKey $hid; if ($key) { break } }
        if ($key -and -not $byKey.Contains($key)) { $byKey[$key] = $d }
    }
    $found = @(); $n = 0; $failed = 0; $firstErr = ''
    foreach ($key in @($byKey.Keys)) {
        if ($n -ge $maxQueries) { break }
        $n++
        $d = $byKey[$key]
        try { $best = Select-BestCatalogRow (& $search $key) $hostInfo } catch { $failed++; if (-not $firstErr) { $firstErr = $_.Exception.Message }; continue }
        if (-not $best) { continue }
        $newV = ConvertTo-Ver $best.Version; $curV = ConvertTo-Ver $d.Version
        $live = ConvertTo-Ver (Get-LiveDriverVersion $d.InstanceId); if ($live) { $curV = $live }
        if ($curV -and $newV -le $curV) { continue }
        # Intel INF drivers use a fake 1968 date, so the date is only trusted when both sides look real.
        $realDates = $best.Date -and $d.Date -and $best.Date.Year -gt 1990 -and ([datetime]$d.Date).Year -gt 1990
        if ($realDates -and $best.Date -lt ([datetime]$d.Date).Date) { continue }
        $found += [pscustomobject]@{
            Id = "ms:$($best.UpdateId)"; Title = "$($d.Name): $($best.Title) [$(if ($d.Version) { 'now ' + $d.Version } else { 'no driver yet' })]"; Class = $d.Class; Maker = $d.Maker
            Date = $(if ($best.Date) { $best.Date.ToString('yyyy-MM-dd') } else { '' }); SizeMB = $best.SizeMB; Source = 'Microsoft Catalog'
            Raw = [pscustomobject]@{ UpdateId = $best.UpdateId; Key = $key; Name = $d.Name }; InstanceId = $d.InstanceId; DeviceName = $d.Name
        }
    }
    $script:MsCatalogFailures = $failed
    if ($n -gt 0 -and $failed -eq $n) { throw "Every Microsoft Catalog query failed ($failed attempts): $firstErr" }
    return @($found | Group-Object { $_.Raw.UpdateId } | ForEach-Object { $_.Group[0] })
}

# Download links and Microsoft's own SHA-1 digest (base64) for one catalog update.
function Get-MsCatalogDownload($updateId) {
    $body = @{ updateIDs = ('[{"size":0,"languages":"","uidInfo":"' + $updateId + '","updateID":"' + $updateId + '"}]') }
    $c = (Invoke-WebRequest -Uri 'https://www.catalog.update.microsoft.com/DownloadDialog.aspx' -Method Post -Body $body -UseBasicParsing -TimeoutSec 30).Content
    @([regex]::Matches($c, "(?s)\.url\s*=\s*'(https://[^']+)'.*?\.digest\s*=\s*'([^']+)'") | ForEach-Object { [pscustomobject]@{ Url = $_.Groups[1].Value; Digest = $_.Groups[2].Value } })
}

# $download / $localFile are for tests (skip the catalog lookup / the network download).
function Install-MsCatalogItem($item, $workDir, $download = $null, $localFile = $null) {
    $res = [pscustomobject]@{ Success = $false; Reboot = $false; Message = '' }
    try {
        $files = if ($download) { @($download) } else { @(Get-MsCatalogDownload $item.Raw.UpdateId) }
        if ($files.Count -eq 0) { throw 'No download link found in the Microsoft catalog' }
        New-Item -ItemType Directory -Force $workDir | Out-Null
        $isAdmin = (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if ($isAdmin) { & icacls.exe $workDir /inheritance:r /grant:r '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' | Out-Null }
        $pkg = Join-Path $workDir 'pkg'
        New-Item -ItemType Directory -Force $pkg | Out-Null
        $i = 0
        foreach ($f in $files) {
            Assert-AllowedUrl $f.Url
            $cab = Join-Path $workDir ('part' + ($i++) + '.cab')
            if ($localFile) { Copy-Item $localFile $cab -Force } else { Invoke-WebRequest -Uri $f.Url -OutFile $cab -UseBasicParsing -TimeoutSec 600 }
            $fs = [IO.File]::OpenRead($cab)
            try { $got = [Convert]::ToBase64String([Security.Cryptography.SHA1]::Create().ComputeHash($fs)) } finally { $fs.Dispose() }
            if ($got -cne $f.Digest) { throw 'File SHA-1 does not match the value reported by the Microsoft catalog' }
            & expand.exe $cab '-F:*' $pkg | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Failed to extract the cab file' }
        }
        # Never install a package that does not mention this device's hardware ID.
        $mentions = Get-ChildItem $pkg -Recurse -Filter *.inf | Select-String -Pattern ([regex]::Escape($item.Raw.Key)) -Quiet
        if (-not $mentions) { throw "This package does not contain hardware $($item.Raw.Key), so it will not be installed" }
        $out = & pnputil.exe /add-driver (Join-Path $pkg '*.inf') /subdirs /install 2>&1 | Out-String
        $code = $LASTEXITCODE
        $res.Message = $out.Trim()
        $res.Success = ($code -eq 0 -or $code -eq 3010)
        $res.Reboot = ($code -eq 3010) -or ($out -match 'reboot|restart')
    } catch {
        $res.Message = $_.Exception.Message
    }
    return $res
}

# ---------- vendor-direct providers: ask the maker's own site for the newest driver ----------
# Trust model: the lookup uses the maker's public API; the download URL must be on an allowed host (https) and the installer must carry a
# valid Authenticode signature from the maker (no hash is published, so the signature is the check).
$ApiHosts = @('nvidia.com', 'geforce.com')

# Windows reports NVIDIA drivers as 32.0.16.2114; NVIDIA calls that 621.14 (last digit of the 3rd part + the 4th part).
function ConvertTo-NvidiaVersion($winVer) {
    $p = ([string]$winVer).Split('.')
    if ($p.Count -lt 4 -or $p[2].Length -lt 1 -or $p[3].Length -ne 4) { return $null }
    $s = $p[2].Substring($p[2].Length - 1) + $p[3].Substring(0, 4)   # 6 + 2114 -> 62114 -> 621.14
    try { return [double]($s.Substring(0, 3) + '.' + $s.Substring(3)) } catch { return $null }
}

function Get-NvidiaProduct($deviceName, $get) {
    $name = ($deviceName -replace '^NVIDIA\s+', '').Trim()
    $m = [regex]::Match($name, '(?:RTX|GTX|GT)\s*(\d{3,4})')
    if (-not $m.Success) { return $null }
    $num = $m.Groups[1].Value
    $gen = if ($num.Length -eq 4) { $num.Substring(0, 2) } else { $num.Substring(0, 1) + '00' }
    $laptop = $name -match 'Laptop|Max-Q|Mobile'
    $series = [xml](& $get 'https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=2&ParentID=1')
    foreach ($s in @($series.LookupValueSearch.LookupValues.LookupValue)) {
        if ($s.Name -notmatch "\b$gen\s+Series" -or (($s.Name -match 'Notebook') -ne $laptop)) { continue }
        $prods = [xml](& $get ('https://www.nvidia.com/Download/API/lookupValueSearch.aspx?TypeID=3&ParentID=' + $s.Value))
        foreach ($p in @($prods.LookupValueSearch.LookupValues.LookupValue)) {
            if ($p.Name -ieq $name) { return [pscustomobject]@{ Psid = $s.Value; Pfid = $p.Value; Name = $p.Name } }
        }
    }
    return $null
}

# $get is for tests: { param($url) -> response text }.
function Find-NvidiaUpdates($installed, $hostInfo = (Get-HostInfo), $get = { param($u) Assert-AllowedUrl $u $ApiHosts; (Invoke-WebRequest -Uri $u -UseBasicParsing -TimeoutSec 30).Content }) {
    $found = @()
    if ($hostInfo.Arch -ne 'x64') { return $found }
    foreach ($d in @($installed | Where-Object { $_.Class -eq 'DISPLAY' -and $_.Maker -like 'NVIDIA*' -and $_.Name -match 'GeForce' })) {
        $cur = ConvertTo-NvidiaVersion $d.Version
        $live = ConvertTo-NvidiaVersion (Get-LiveDriverVersion $d.InstanceId); if ($live) { $cur = $live }
        if (-not $cur) { continue }
        $prod = Get-NvidiaProduct $d.Name $get
        if (-not $prod) { continue }
        $os = if ($hostInfo.Os -eq '11') { 135 } else { 57 }
        $json = (& $get ("https://gfwsl.geforce.com/services_toolkit/services/com/nvidia/services/AjaxDriverService.php?func=DriverManualLookup&psid=$($prod.Psid)&pfid=$($prod.Pfid)&osID=$os&languageCode=1033&beta=0&isWHQL=0&dltype=-1&dch=1&upCRD=0&qnf=0&sort1=0&numberOfResults=1")) | ConvertFrom-Json
        $info = @($json.IDS)[0].downloadInfo
        if (-not $info -or -not $info.DownloadURL) { continue }
        $new = [double]($info.Version -replace '[^\d\.]', '')
        if ($new -le $cur) { continue }
        $mb = $null; $sz = [regex]::Match([string]$info.DownloadURLFileSize, '([\d\.]+)\s*(MB|GB)')
        if ($sz.Success) { $mb = [math]::Round([double]$sz.Groups[1].Value * $(if ($sz.Groups[2].Value -eq 'GB') { 1024 } else { 1 }), 1) }
        $date = ''; try { $date = ([datetime]::ParseExact(([string]$info.ReleaseDateTime), 'ddd MMM dd, yyyy', [Globalization.CultureInfo]::InvariantCulture)).ToString('yyyy-MM-dd') } catch {}
        $found += [pscustomobject]@{
            Id = "ven:nvidia:$($info.Version)"; Title = "NVIDIA GeForce Game Ready Driver $($info.Version) [now $($cur.ToString('0.00'))]"; Class = 'DISPLAY'; Maker = 'NVIDIA'
            Date = $date; SizeMB = $mb; Source = 'NVIDIA'; InstanceId = $d.InstanceId; DeviceName = $d.Name
            Raw = [pscustomobject]@{ Url = [string]$info.DownloadURL; Version = [string]$info.Version; Signer = 'NVIDIA Corporation'; SilentArgs = '-s -noreboot -noeula'; InstanceId = $d.InstanceId; DeviceName = $d.Name }
        }
    }
    return $found
}

# Download, verify the maker's signature, run the installer silently. $localFile is for tests.
function Install-VendorItem($item, $workDir, $localFile = $null) {
    $res = [pscustomobject]@{ Success = $false; Reboot = $false; Message = '' }
    try {
        $r = $item.Raw
        Assert-AllowedUrl $r.Url
        New-Item -ItemType Directory -Force $workDir | Out-Null
        $isAdmin = (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if ($isAdmin) { & icacls.exe $workDir /inheritance:r /grant:r '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' | Out-Null }
        $file = Join-Path $workDir ([IO.Path]::GetFileName(([uri]$r.Url).AbsolutePath))
        if ($localFile) { Copy-Item $localFile $file -Force } else { Invoke-WebRequest -Uri $r.Url -OutFile $file -UseBasicParsing -TimeoutSec 3600 }
        $sig = Get-AuthenticodeSignature -FilePath $file
        if ($sig.Status -ne 'Valid') { throw "The installer's digital signature is not valid ($($sig.Status)), so it will not be installed" }
        if ($sig.SignerCertificate.Subject -notlike "*$($r.Signer)*") { throw "Signer does not match $($r.Signer): $($sig.SignerCertificate.Subject)" }
        $p = Start-Process -FilePath $file -ArgumentList $r.SilentArgs -Wait -PassThru
        $res.Message = "exit code $($p.ExitCode)"
        $res.Success = ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010)
        $res.Reboot = ($p.ExitCode -eq 3010)
        if (-not $res.Success -and $r.InstanceId) {
            # the installer may report "reboot needed" with another code: trust the driver Windows now reports
            $now = ConvertTo-NvidiaVersion (Get-LiveDriverVersion $r.InstanceId); $want = [double]($r.Version -replace '[^\d\.]', '')
            if ($now -and $now -ge $want) { $res.Success = $true; $res.Reboot = $true }
        }
    } catch {
        $res.Message = $_.Exception.Message
    }
    return $res
}

# ---------- self-update: the program checks a signed manifest on GitHub and replaces its own script files ----------
# Trust model: latest.json is signed with the publisher's private key (RSA/SHA-256); the app only holds the PUBLIC key
# (UpdatePublicKey.xml). The signature covers version + zip hash + zip URL, so a tampered release cannot be installed.
$UpdateHosts = @('github.com', 'githubusercontent.com')
$UpdateAllowedExt = @('.ps1', '.json', '.png', '.ico', '.txt')   # an update never carries executables
$UpdateKeepFiles = @('driverdb.json', 'update.json')             # the user's own catalog / settings are never overwritten

function New-UpdateSignature($m, $privateKeyXml) {
    $rsa = New-Object Security.Cryptography.RSACryptoServiceProvider; $rsa.FromXmlString($privateKeyXml)
    return [Convert]::ToBase64String($rsa.SignData([Text.Encoding]::UTF8.GetBytes("$($m.version)|$($m.sha256)|$($m.url)"), 'SHA256'))
}

function Test-UpdateSignature($m, $publicKeyXml) {
    try {
        $rsa = New-Object Security.Cryptography.RSACryptoServiceProvider; $rsa.FromXmlString($publicKeyXml)
        return $rsa.VerifyData([Text.Encoding]::UTF8.GetBytes("$($m.version)|$($m.sha256)|$($m.url)"), 'SHA256', [Convert]::FromBase64String([string]$m.signature))
    } catch { return $false }
}

# Returns the manifest when a NEWER, correctly signed version exists; $null when up to date or updates are not configured.
# Throws when a manifest exists but is not trustworthy.  $get is for tests: { param($url) -> text }.
function Get-AppUpdate($repo, $currentVersion, $publicKeyXml, $get = $null) {
    if (-not $get) {
        $get = { param($u)
            Assert-AllowedUrl $u $UpdateHosts
            $c = (Invoke-WebRequest -Uri $u -UseBasicParsing -TimeoutSec 20).Content
            if ($c -is [byte[]]) { $c = [Text.Encoding]::UTF8.GetString($c) }
            return $c.TrimStart([char]0xFEFF)
        }
    }
    if (-not $publicKeyXml -or $repo -notmatch '^[\w\.\-]+/[\w\.\-]+$') { return $null }
    $m = (& $get "https://github.com/$repo/releases/latest/download/latest.json") | ConvertFrom-Json
    $newV = ConvertTo-Ver $m.version; $curV = ConvertTo-Ver $currentVersion
    if (-not $newV) { throw 'The version format in the update file is not valid' }
    if ($curV -and $newV -le $curV) { return $null }
    if (-not (Test-UpdateSignature $m $publicKeyXml)) { throw 'The update file signature is not valid, so it will not be applied (the file may have been tampered with)' }
    if ([string]$m.sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'The sha256 value in the update file is not valid' }
    Assert-AllowedUrl $m.url $UpdateHosts
    return $m
}

# Downloads, verifies and applies an update to $installDir. Old files are kept in _backup\<old version> and restored if anything fails.
# $localZip is for tests (skips the download; the URL is still checked).
function Install-AppUpdate($m, $installDir, $workDir, $oldVersion, $localZip = $null) {
    $res = [pscustomobject]@{ Success = $false; Message = '' }
    $backup = $null; $touched = @()
    try {
        Assert-AllowedUrl $m.url $UpdateHosts
        New-Item -ItemType Directory -Force $workDir | Out-Null
        $isAdmin = (New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if ($isAdmin) { & icacls.exe $workDir /inheritance:r /grant:r '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' | Out-Null }
        $zip = Join-Path $workDir 'update.zip'
        if ($localZip) { Copy-Item $localZip $zip -Force } else { Invoke-WebRequest -Uri $m.url -OutFile $zip -UseBasicParsing -TimeoutSec 600 }
        [void](Test-FileSha256 $zip $m.sha256)
        $new = Join-Path $workDir 'new'
        Expand-SafeZip $zip $new
        # (compare by listing, not by path text: %TEMP% can be an 8.3 short path while the listing returns long names)
        $sub = @(Get-ChildItem $new -Directory)
        if ($sub.Count -gt 0) { throw "The update contains a subfolder: $($sub[0].Name)" }
        $files = @(Get-ChildItem $new -File)
        foreach ($f in $files) {
            if ($UpdateAllowedExt -notcontains $f.Extension.ToLowerInvariant()) { throw "The update contains a file type that is not allowed: $($f.Name)" }
        }
        foreach ($must in 'DriverScan.ps1', 'DriverCore.ps1', 'DriverUI.ps1') { if (-not (Test-Path (Join-Path $new $must))) { throw "The update is missing $must" } }
        foreach ($f in @($files | Where-Object { $_.Extension -eq '.ps1' })) {
            $err = $null; [void][Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$null, [ref]$err)
            if ($err.Count -gt 0) { throw "Script $($f.Name) in the update has a syntax error, so it will not be applied" }
        }
        $backup = Join-Path $installDir ('_backup\' + $oldVersion)
        New-Item -ItemType Directory -Force $backup | Out-Null
        foreach ($f in $files) {
            if ($UpdateKeepFiles -contains $f.Name.ToLowerInvariant() -and (Test-Path (Join-Path $installDir $f.Name))) { continue }
            $dst = Join-Path $installDir $f.Name
            if (Test-Path $dst) { Copy-Item $dst (Join-Path $backup $f.Name) -Force }
            $touched += $f.Name
            Copy-Item $f.FullName $dst -Force
        }
        $res.Success = $true; $res.Message = "Updated to version $($m.version) ($($touched.Count) files)"
    } catch {
        $res.Message = $_.Exception.Message
        foreach ($n in $touched) { $b = if ($backup) { Join-Path $backup $n } else { $null }; if ($b -and (Test-Path $b)) { Copy-Item $b (Join-Path $installDir $n) -Force } }
    }
    return $res
}


# ---------- online catalog: the signed driverdb.json published on GitHub (release tag "catalog") ----------
# Same trust model as program updates: a manifest signed with the publisher's private key covers revision + file hash + url.
# An online catalog can add drivers but can never widen the download-host list (that stays hard-coded above).
function Test-CatalogEntry($c) {
    if (-not $c.name) { return 'no name' }
    if (-not (ConvertTo-Ver $c.version)) { return "invalid version ($($c.version))" }
    if (-not @($c.match).Count -or -not (@($c.match) | Where-Object { $_ }).Count) { return 'no match' }
    if ($c.type -notin 'installer', 'inf-zip') { return "unknown type ($($c.type))" }
    if ([string]$c.sha256 -notmatch '^[0-9a-fA-F]{64}$') { return 'invalid sha256' }
    try { Assert-AllowedUrl $c.url } catch { return $_.Exception.Message }
    if ($c.type -eq 'installer' -and (-not $c.signer -or -not $c.silentArgs)) { return 'installer needs signer and silentArgs' }
    return $null
}

# Returns @{ Updated; Revision; Count }. Throws when the online catalog is not trustworthy (the old cache stays untouched).
# $fetch is for tests: { param($url) -> byte[] }.
function Get-OnlineCatalog($repo, $publicKeyXml, $cachePath, $fetch = $null) {
    if (-not $fetch) {
        $fetch = { param($u)
            Assert-AllowedUrl $u $UpdateHosts
            $tmp = [IO.Path]::GetTempFileName()
            try { Invoke-WebRequest -Uri $u -OutFile $tmp -UseBasicParsing -TimeoutSec 60; return ,[IO.File]::ReadAllBytes($tmp) } finally { Remove-Item $tmp -ErrorAction SilentlyContinue }
        }
    }
    if (-not $publicKeyXml -or $repo -notmatch '^[\w\.\-]+/[\w\.\-]+$') { return $null }
    $m = [Text.Encoding]::UTF8.GetString((& $fetch "https://github.com/$repo/releases/download/catalog/catalog.json")).TrimStart([char]0xFEFF) | ConvertFrom-Json
    $rev = [int](([string]$m.version) -replace '^1\.', '')
    $curRev = 0
    if (Test-Path $cachePath) { try { $curRev = [int]((Get-Content $cachePath -Raw -Encoding UTF8 | ConvertFrom-Json).revision) } catch {} }
    if ($rev -le $curRev) { return [pscustomobject]@{ Updated = $false; Revision = $curRev; Count = 0 } }
    if (-not (Test-UpdateSignature $m $publicKeyXml)) { throw 'The online catalog signature is not valid, so it will not be used (the file may have been tampered with)' }
    Assert-AllowedUrl $m.url $UpdateHosts
    $bytes = & $fetch $m.url
    $got = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($bytes)).Replace('-', '')
    if ($got -ne ([string]$m.sha256).ToUpperInvariant()) { throw 'The online catalog SHA-256 does not match, so it will not be used' }
    $doc = [Text.Encoding]::UTF8.GetString($bytes).TrimStart([char]0xFEFF) | ConvertFrom-Json
    $entries = @(@($doc.drivers) | Where-Object { $_.name })
    foreach ($e in $entries) { $why = Test-CatalogEntry $e; if ($why) { throw "Entry '$($e.name)' in the online catalog failed validation: $why" } }
    $copy = [pscustomobject]@{ revision = $rev; drivers = $entries }
    $tmpOut = $cachePath + '.new'
    [IO.File]::WriteAllText($tmpOut, ($copy | ConvertTo-Json -Depth 8), (New-Object Text.UTF8Encoding($true)))
    Move-Item $tmpOut $cachePath -Force
    return [pscustomobject]@{ Updated = $true; Revision = $rev; Count = $entries.Count }
}

# Local file first, then the online cache; an online entry replaces a local one with the same name (the online one is the maintained one).
function Read-AllCatalogs($localPath, $onlinePath) {
    $byName = [ordered]@{}
    foreach ($c in @(Read-DriverCatalog $localPath)) { $byName[$c.name] = $c }
    if ($onlinePath -and (Test-Path $onlinePath)) {
        foreach ($c in @(Read-DriverCatalog $onlinePath)) { $byName[$c.name] = $c }
    }
    return @($byName.Values)
}


# ---------- devices that have NO driver yet (a newly plugged printer, an unknown device) ----------
# Win32_PnPSignedDriver only lists devices that already have a driver, so these are read from Win32_PnPEntity (problem codes 1 and 28)
# and shaped like installed drivers (no Version) so the same Microsoft Catalog search can look for them by hardware/compatible ID.
function Get-DevicesWithoutDriver($entities = $null) {
    if ($null -eq $entities) { $entities = Get-CimInstance Win32_PnPEntity | Where-Object { $_.ConfigManagerErrorCode -in 1, 28 } }
    foreach ($e in @($entities)) {
        $ids = @(@($e.HardwareID) + @($e.CompatibleID)) | Where-Object { $_ }
        [pscustomobject]@{ InstanceId = $e.PNPDeviceID; Name = $(if ($e.Name) { $e.Name } else { 'Unknown device' }); Class = [string]$e.PNPClass; Maker = [string]$e.Manufacturer
            Version = $null; Date = $null; HardwareId = @($ids); ErrorCode = $e.ConfigManagerErrorCode }
    }
}

# Devices without a driver for which no offer was found (the list shows them so the user knows to look manually).
function Get-UnresolvedDevices($devices, $offers) {
    $covered = @($offers | Where-Object { $_.Raw -and $_.Raw.Name } | ForEach-Object { $_.Raw.Name })
    return @($devices | Where-Object { $covered -notcontains $_.Name })
}


# ---------- driver backup before install, and roll back ----------
# Before a driver is replaced, the package currently bound to that device is exported with pnputil (Windows' own tool) into
# C:\ProgramData\TierS Driver Manage\Backups\<date-time>\<n>\ with a manifest, so the user can roll back later.
# Only devices we can identify (Microsoft Catalog, NVIDIA and catalog offers) are backed up; Windows Update offers carry no device.
$BackupRoot = Join-Path $env:ProgramData 'TierS Driver Manage\Backups'

# $run is for tests: { param($pnputilArgs) -> output text } (sets $LASTEXITCODE like a native command).
function Invoke-PnpUtil([string[]]$pnpArgs) { return (& pnputil.exe @pnpArgs 2>&1 | Out-String) }

function Get-DeviceDriverInfo($instanceId) {
    try {
        $p = @{}
        foreach ($x in @(Get-PnpDeviceProperty -InstanceId $instanceId -KeyName 'DEVPKEY_Device_DriverInfPath', 'DEVPKEY_Device_DriverVersion' -ErrorAction Stop)) { $p[$x.KeyName] = $x.Data }
        return [pscustomobject]@{ Inf = [string]$p['DEVPKEY_Device_DriverInfPath']; Version = [string]$p['DEVPKEY_Device_DriverVersion'] }
    } catch { return $null }
}

# Exports the driver now bound to the device. Returns the manifest entry, or $null when there is nothing to back up
# (no driver yet, or a driver built into Windows: rolling back then simply removes the new package).
function Backup-DeviceDriver($instanceId, $deviceName, $setDir, $index, $info = $null, $run = { param($a) Invoke-PnpUtil $a }) {
    if (-not $info) { $info = Get-DeviceDriverInfo $instanceId }
    if (-not $info -or -not $info.Inf) { return $null }
    $entry = [ordered]@{ Device = $deviceName; InstanceId = $instanceId; OldInf = $info.Inf; OldVersion = $info.Version; Folder = ''; Time = (Get-Date).ToString('s') }
    if ($info.Inf -match '^oem\d+\.inf$') {
        $dir = Join-Path $setDir ([string]$index)
        New-Item -ItemType Directory -Force $dir | Out-Null
        [void](& $run @('/export-driver', $info.Inf, $dir))
        if (-not (Get-ChildItem $dir -Filter *.inf -Recurse -ErrorAction SilentlyContinue)) { return $null }
        $entry['Folder'] = [string]$index
    }
    return [pscustomobject]$entry
}

function Save-BackupManifest($setDir, $entries) {
    New-Item -ItemType Directory -Force $setDir | Out-Null
    [IO.File]::WriteAllText((Join-Path $setDir 'manifest.json'), (@($entries) | ConvertTo-Json -Depth 4), (New-Object Text.UTF8Encoding($true)))
}

# Every backed-up device, newest first.
function Get-DriverBackups($root = $BackupRoot) {
    if (-not (Test-Path $root)) { return @() }
    $all = foreach ($set in Get-ChildItem $root -Directory -ErrorAction SilentlyContinue) {
        $mf = Join-Path $set.FullName 'manifest.json'
        if (-not (Test-Path $mf)) { continue }
        foreach ($e in @([IO.File]::ReadAllText($mf, [Text.Encoding]::UTF8) | ConvertFrom-Json | ForEach-Object { $_ })) {   # unroll: Windows PowerShell 5.1 returns a JSON array as ONE object
            [pscustomobject]@{ Set = $set.Name; SetDir = $set.FullName; Device = $e.Device; InstanceId = $e.InstanceId; OldInf = $e.OldInf; OldVersion = $e.OldVersion; Folder = $e.Folder; Time = $e.Time }
        }
    }
    return @($all | Sort-Object Time -Descending)
}

# Puts the old driver back: install the saved package (when one was saved), then remove the package the device uses now so Windows
# falls back to the old one (or to the built-in driver). $infoNow / $run are for tests.
function Restore-DeviceDriver($backup, $infoNow = $null, $run = { param($a) Invoke-PnpUtil $a }) {
    $res = [pscustomobject]@{ Success = $false; Message = '' }
    try {
        if (-not $infoNow) { $infoNow = Get-DeviceDriverInfo $backup.InstanceId }
        if (-not $infoNow -or -not $infoNow.Inf) { throw 'The device is not present or has no driver now.' }
        if ($infoNow.Inf -ieq $backup.OldInf) { $res.Success = $true; $res.Message = 'The device already uses the backed-up driver.'; return $res }
        if ($backup.Folder) {
            $dir = Join-Path $backup.SetDir $backup.Folder
            if (-not (Get-ChildItem $dir -Filter *.inf -Recurse -ErrorAction SilentlyContinue)) { throw 'The backup files are missing.' }
            $out = & $run @('/add-driver', (Join-Path $dir '*.inf'), '/subdirs', '/install')
            if ($LASTEXITCODE -notin 0, 3010) { throw "Could not add the old driver: $($out.Trim())" }
        }
        if ($infoNow.Inf -notmatch '^oem\d+\.inf$') { throw 'The current driver is built into Windows and cannot be removed.' }
        $out2 = & $run @('/delete-driver', $infoNow.Inf, '/uninstall', '/force')
        if ($LASTEXITCODE -notin 0, 3010) { throw "Could not remove the new driver: $($out2.Trim())" }
        $res.Success = $true; $res.Message = "Rolled back $($backup.Device) to version $($backup.OldVersion). A restart may be needed."
    } catch { $res.Message = $_.Exception.Message }
    return $res
}