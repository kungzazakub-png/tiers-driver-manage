# DriverScan - one-click driver updater modelled on the Driver Booster pipeline:
#   scan hardware -> compare with a driver catalog -> download (https + allowlist) -> verify (SHA-256, signature)
#   -> install (pnputil / signed silent installer) -> repeat passes.
# Three driver sources are merged into one list (Microsoft Update Catalog is searched by hardware ID):
#   1) Windows Update driver catalog (Microsoft, WHQL-signed)
#   2) your own catalog file driverdb.json (or CatalogUrl below) - see driverdb.example.json and New-CatalogEntry.ps1
# Requires an internet connection. Installing needs administrator rights (the script asks for them).
param(
    [switch]$SelfTest,
    [switch]$NoInstall,      # developer mode: run in place, do not copy to Program Files
    [switch]$InstallOnly,    # copy to the install folder and stop (used by tests)
    [switch]$Uninstall,      # remove shortcuts, the Apps entry and the install folder (called by Windows Settings)
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'TierS Driver Manage')
)

# Setup-stage log (before the GUI exists): %TEMP%\DriverScan-setup.log
function Write-SetupLog($m) { try { Add-Content -Path (Join-Path $env:TEMP 'DriverScan-setup.log') -Value ('{0:HH:mm:ss} [{1}] {2}' -f (Get-Date), $PID, $m) } catch {} }
$passArgs = @(foreach ($k in $PSBoundParameters.Keys) { if ($PSBoundParameters[$k] -is [switch]) { "-$k" } else { "-$k"; '"' + $PSBoundParameters[$k] + '"' } })
$baseArgs = @('-WindowStyle', 'Hidden', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $PSCommandPath + '"'))
Write-SetupLog "start: 64bit=$([Environment]::Is64BitProcess) from=$PSScriptRoot args=$($passArgs -join ' ')"

# The one-file setup starts 32-bit PowerShell; Program Files, the CPU type and the catalog filters need the 64-bit one.
if (-not [Environment]::Is64BitProcess -and [Environment]::Is64BitOperatingSystem) {
    Write-SetupLog '32-bit shell: relaunching in 64-bit'
    $p = Start-Process "$env:windir\sysnative\WindowsPowerShell\v1.0\powershell.exe" -Wait -PassThru -ArgumentList ($baseArgs + $passArgs)
    exit $p.ExitCode
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

function Test-Admin {
    $p = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Show-Fatal($msg) { [void][Windows.Forms.MessageBox]::Show($msg, 'TierS Driver Manage', 'OK', 'Error') }

# The console window is hidden, so any unexpected error must be shown in a box instead of vanishing.
trap { if ($SelfTest) { throw }; Show-Fatal ('An error occurred: ' + $_.Exception.Message); exit 1 }

if (-not $SelfTest -and -not (Test-Admin)) {
    Write-SetupLog 'not admin: asking for administrator rights'
    # -Wait: when started from the one-file setup, its temp files must stay until the elevated copy has installed itself.
    $vbs = Join-Path $PSScriptRoot 'TierS.vbs'   # wscript launcher: no console window flashes during the hop
    try { if (Test-Path $vbs) { Start-Process "$env:windir\System32\wscript.exe" -Verb RunAs -Wait -ArgumentList (@('//B', ('"' + $vbs + '"')) + $passArgs) } else { Start-Process powershell -Verb RunAs -Wait -ArgumentList ($baseArgs + $passArgs) } } catch { }   # declining the UAC prompt just ends here
    exit
}

# Copies the program into the install folder (Program Files by default) so nobody has to place files by hand.
# Returns $false when this already is the installed copy.
function Install-Self {
    $here = (Resolve-Path $PSScriptRoot).Path.TrimEnd('\')
    $dest = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
    if ($here -ieq $dest -or (Test-Path (Join-Path $here 'unins000.exe'))) { return $false }   # already installed (by Setup, possibly in a folder the user chose)
    foreach ($f in 'DriverScan.ps1', 'DriverCore.ps1', 'DriverUI.ps1') { if (-not (Test-Path (Join-Path $here $f))) { throw "File $f was not found next to the program, so it cannot be installed" } }
    New-Item -ItemType Directory -Force $dest | Out-Null
    foreach ($f in 'DriverScan.ps1', 'DriverCore.ps1', 'DriverUI.ps1', 'New-CatalogEntry.ps1', 'DriverScan.cmd', 'driverdb.example.json', 'README.txt', 'TierS.ico', 'tiers-logo.png', 'TierS.vbs', 'TierS Driver Manage.exe') {
        $s = Join-Path $here $f
        if (Test-Path $s) { try { Copy-Item $s $dest -Force } catch { Write-SetupLog "could not copy $f (in use?): $($_.Exception.Message)" } }   # a file in use is skipped, not fatal
    }
    # keep a catalog the user already extended, unless the one shipped now is newer
    $sc = Join-Path $here 'driverdb.json'; $dc = Join-Path $dest 'driverdb.json'
    if ((Test-Path $sc) -and (-not (Test-Path $dc) -or (Get-Item $sc).LastWriteTime -gt (Get-Item $dc).LastWriteTime)) { Copy-Item $sc $dc -Force }
    if ($dest -ieq (Join-Path $env:ProgramFiles 'TierS Driver Manage')) {
        $sh = New-Object -ComObject WScript.Shell
        foreach ($dir in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {
            Remove-Item (Join-Path $dir 'DriverScan.lnk') -Force -ErrorAction SilentlyContinue   # shortcut from before the rename
            $l = $sh.CreateShortcut((Join-Path $dir 'TierS Driver Manage.lnk'))
            $l.TargetPath = Join-Path $dest 'TierS Driver Manage.exe'; $l.Arguments = ''; $l.WorkingDirectory = $dest; $l.WindowStyle = 1
            $l.IconLocation = (Join-Path $dest 'TierS.ico'); $l.Description = 'TierS Driver Manage - scan and install drivers'
            $l.Save()
        }
        # shows up in Settings > Apps > Installed apps, with an uninstall button
        try {
            $k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TierSDriverManage'
            New-Item -Path $k -Force | Out-Null
            $exe = Join-Path $dest 'TierS Driver Manage.exe'
            $vals = @{ DisplayName = 'TierS Driver Manage'; DisplayVersion = '1.0'; Publisher = 'TierS'; DisplayIcon = $exe; InstallLocation = $dest; UninstallString = ('"' + $exe + '" -Uninstall') }
            foreach ($n in $vals.Keys) { Set-ItemProperty -Path $k -Name $n -Value $vals[$n] }
            Set-ItemProperty -Path $k -Name NoModify -Value 1 -Type DWord; Set-ItemProperty -Path $k -Name NoRepair -Value 1 -Type DWord
            Set-ItemProperty -Path $k -Name EstimatedSize -Value ([int]((Get-ChildItem $dest -Recurse | Measure-Object Length -Sum).Sum / 1KB)) -Type DWord
        } catch { Write-SetupLog "uninstall entry failed: $($_.Exception.Message)" }    }
    return $true
}

if ($Uninstall) {
    if ([Windows.Forms.MessageBox]::Show('Uninstall TierS Driver Manage from this computer?', 'TierS Driver Manage', 'YesNo', 'Question') -ne 'Yes') { exit 0 }
    $dest = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
    foreach ($dir in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) { Remove-Item (Join-Path $dir 'TierS Driver Manage.lnk') -Force -ErrorAction SilentlyContinue }
    Remove-Item 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TierSDriverManage' -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $env:ProgramData 'TierS Driver Manage') -Recurse -Force -ErrorAction SilentlyContinue
    # this script lives in the folder being removed, so a detached cmd deletes it after we exit
    Start-Process cmd.exe -WindowStyle Hidden -ArgumentList ('/c ping 127.0.0.1 -n 5 >nul & rmdir /s /q "' + $dest + '"')
    exit 0
}
if (-not $SelfTest -and -not $NoInstall) {
    Write-SetupLog "admin ok; installing into $InstallDir"
    if (Install-Self) {
        Write-SetupLog 'installed; relaunching from the install folder'
        if ($InstallOnly) { exit 0 }
        $vbsInstalled = Join-Path ([IO.Path]::GetFullPath($InstallDir)) 'TierS.vbs'
        $exeInstalled = Join-Path ([IO.Path]::GetFullPath($InstallDir)) 'TierS Driver Manage.exe'
        if (Test-Path $exeInstalled) { $rest = @($passArgs | Where-Object { $_ -ne '-InstallOnly' }); if ($rest.Count -gt 0) { Start-Process $exeInstalled -ArgumentList $rest } else { Start-Process $exeInstalled } }   # an empty -ArgumentList is an error
        elseif (Test-Path $vbsInstalled) { Start-Process "$env:windir\System32\wscript.exe" -ArgumentList (@('//B', ('"' + $vbsInstalled + '"')) + @($passArgs | Where-Object { $_ -ne '-InstallOnly' })) }
        else {
          Start-Process (Join-Path $PSHOME 'powershell.exe') -ArgumentList (@('-WindowStyle', 'Hidden', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + (Join-Path ([IO.Path]::GetFullPath($InstallDir)) 'DriverScan.ps1') + '"')) + @($passArgs | Where-Object { $_ -ne '-InstallOnly' }))
        }
        exit 0
    }
}
if ($InstallOnly) { exit 0 }
$script:DryRun = $false        # self-test only: never start a real install
$script:AutoConfirm = $false   # self-test only: skip the confirmation dialog
$script:MaxPasses = 3
$script:OldYears = 3           # installed drivers older than this are flagged
$script:CatalogUrl = ''   # optional https address of a shared catalog; empty = use driverdb.json next to this script
$script:CatalogPath = if ($script:CatalogUrl) { $script:CatalogUrl } else { Join-Path $PSScriptRoot 'driverdb.json' }
$script:LogFile = Join-Path $env:ProgramData 'TierS Driver Manage\DriverScan.log'
$script:AppVersion = '1.0.5'   # bumped by Publish-Update.ps1
# Self-update settings: update.json = {"repo":"owner/name"} (GitHub). Without a repo or the public key nothing is checked. Developer runs (-NoInstall) never replace themselves.
$script:UpdateRepo = ''; $script:UpdatePublicKey = ''
try { $uc = Join-Path $PSScriptRoot 'update.json'; if (Test-Path $uc) { $script:UpdateRepo = [string](Get-Content $uc -Raw | ConvertFrom-Json).repo } } catch {}
try { $uk = Join-Path $PSScriptRoot 'UpdatePublicKey.xml'; if (Test-Path $uk) { $script:UpdatePublicKey = Get-Content $uk -Raw } } catch {}
$script:UpdateEnabled = (-not $NoInstall) -and $script:UpdateRepo -and $script:UpdatePublicKey
$script:UpdateManifest = $null
$script:RequireProtectedFolder = -not $NoInstall   # $true = refuse catalog installs while this folder is writable by normal users (safer: a catalog edit could then run as admin)
$script:FolderWritable = $false
try {
    foreach ($r in (Get-Acl $PSScriptRoot).Access) {
        if ($r.AccessControlType -eq 'Allow' -and $r.IdentityReference.Value -match '(\\Users|Everyone|Authenticated Users)$' -and $r.FileSystemRights.ToString() -match 'Write|Modify|FullControl|CreateFiles') { $script:FolderWritable = $true }
    }
} catch {}

# ---------- core engine: shared with New-CatalogEntry.ps1, also loaded into every worker runspace ----------
$Core = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'DriverCore.ps1'), [Text.Encoding]::UTF8)
. ([scriptblock]::Create($Core))
. (Join-Path $PSScriptRoot 'DriverUI.ps1')   # window, theme and dialogs

# ---------- background workers (own STA runspace so the UI never freezes) ----------
$ScanBlock = {
    param($state)
    try {
        $state.Status = 'Checking internet connection...'
        $online = $false
        try {
            $r0 = Invoke-WebRequest -Uri 'http://www.msftconnecttest.com/connecttest.txt' -UseBasicParsing -TimeoutSec 6
            $online = ($r0.StatusCode -eq 200)
        } catch {}
        if (-not $online) {
            $state.Error = 'No internet connection found. This program needs the internet to scan and install drivers. Please connect and try again.'
            $state.Done = $true
            return
        }

        $state.Status = 'Scanning hardware and installed drivers...'
        $installed = @(Get-InstalledDrivers)
        $inv = @()
        foreach ($d in $installed) {
            $age = $null
            if ($d.Date) { $age = [math]::Round(((Get-Date) - [datetime]$d.Date).TotalDays / 365.25, 1) }
            $inv += [pscustomobject]@{ Name = $d.Name; Class = $d.Class; Maker = $d.Maker; Version = $d.Version; Date = $d.Date; Age = $age }
        }
        $state.Inventory = $inv

        try {
            $bad = Get-CimInstance Win32_PnPEntity | Where-Object { $_.ConfigManagerErrorCode -in 1, 28 }   # 1 = not configured, 28 = no driver (22 = disabled by the user, ignore)
            $state.Missing = @($bad | ForEach-Object { "$($_.Name) [problem code $($_.ConfigManagerErrorCode)]" })
            $state.MissingDevices = @(Get-DevicesWithoutDriver $bad)
        } catch {}

        $items = @()
        $state.Status = 'Checking the online driver catalog and comparing with this PC...'
        try {
            try {
                $oc = Get-OnlineCatalog $state.UpdateRepo $state.PublicKey $state.OnlineCatalogPath
                if ($oc -and $oc.Updated) { $state.Log += "Online catalog: updated to revision $($oc.Revision) ($($oc.Count) entries)`r`n" }
            } catch { $state.Log += "Online catalog: $($_.Exception.Message) (using the stored copy)`r`n" }
            $cat = @(Read-AllCatalogs $state.CatalogPath $state.OnlineCatalogPath)
            $items += @(Find-CatalogUpdates $installed $cat)
            $state.Log += "Catalog: $($cat.Count) entries, $(@($items).Count) newer entries match this PC`r`n"
        } catch {
            $state.Log += "Could not read the catalog: $($_.Exception.Message)`r`n"
        }

        $state.Status = 'Checking the latest version from the manufacturer (NVIDIA)...'
        $vendorDevices = @()
        try {
            $nv = @(Find-NvidiaUpdates $installed)
            $items += $nv; $vendorDevices += @($nv | ForEach-Object { $_.Raw.DeviceName })
            if ($nv.Count -gt 0) { $items = @($items | Where-Object { -not ($_.Source -eq 'Catalog' -and $_.Class -eq 'DISPLAY' -and $_.Maker -like 'NVIDIA*') }) }   # the maker's own offer replaces the static catalog one
            $state.Log += "NVIDIA (direct from manufacturer): $($nv.Count) newer entries found`r`n"
        } catch {
            $state.Log += "NVIDIA (manufacturer): $($_.Exception.Message)`r`n"
        }

        $state.Status = 'Searching Microsoft Update Catalog by hardware ID (may take about 1 minute)...'
        try {
            $ms = @(Find-MsCatalogUpdates (@($state.MissingDevices) + @($installed)))
            $items += @($ms | Where-Object { $vendorDevices -notcontains $_.Raw.Name })   # the maker's own offer wins
            $state.Log += "Microsoft Update Catalog: $($ms.Count) newer entries found`r`n"
            if ($MsCatalogFailures -gt 0) { $state.Log += "Warning: $MsCatalogFailures Microsoft Catalog queries failed (results may be incomplete)`r`n" }
        } catch {
            $state.Log += "Microsoft Update Catalog: $($_.Exception.Message)`r`n"
        }

        $state.Status = 'Searching Windows Update for drivers (may take 1-2 minutes)...'
        try {
            $session = New-Object -ComObject Microsoft.Update.Session
            $searcher = $session.CreateUpdateSearcher()
            $r = $searcher.Search("IsInstalled=0 and Type='Driver' and IsHidden=0")
            foreach ($u in $r.Updates) {
                $date = ''
                try { if ($u.DriverVerDate) { $date = ([datetime]$u.DriverVerDate).ToString('yyyy-MM-dd') } } catch {}
                $items += [pscustomobject]@{
                    Id = $u.Identity.UpdateID; Title = $u.Title; Class = $u.DriverClass; Maker = $u.DriverManufacturer
                    Date = $date; SizeMB = [math]::Round($u.MaxDownloadSize / 1MB, 1); Source = 'Windows Update'; Raw = $null
                }
            }
        } catch {
            $state.Log += "Windows Update: $($_.Exception.Message)`r`n"
        }
        $state.Unresolved = @(Get-UnresolvedDevices @($state.MissingDevices) $items)
        $state.Items = @($items)
        $state.Status = "Scan complete: $(@($items).Count) driver updates found"
    } catch {
        $state.Error = $_.Exception.Message
    }
    $state.Done = $true
}

$InstallBlock = {
    param($state)
    try {
        if ($state.RestorePoint) {
            $state.Status = 'Creating a system restore point...'
            try {
                Checkpoint-Computer -Description 'TierS Driver Manage' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop -WarningVariable rpWarn
                if ($rpWarn) { $state.Log += "Restore point not created: $rpWarn`r`n" } else { $state.Log += "Restore point created`r`n" }
            } catch {
                $state.Log += "Restore point skipped: $($_.Exception.Message)`r`n"
            }
        }
        # keep a copy of the drivers about to be replaced, so they can be rolled back from the Backups tab
        try {
            $state.Status = 'Backing up the current drivers...'
            $setDir = Join-Path $BackupRoot (Get-Date -Format 'yyyyMMdd-HHmmss'); $bk = @(); $bn = 0
            foreach ($bi in @($state.CatalogItems | Where-Object { $_.InstanceId })) {
                try { $bn++; $be = Backup-DeviceDriver $bi.InstanceId $bi.DeviceName $setDir $bn; if ($be) { $bk += $be; $state.Log += "Backed up the current driver of $($bi.DeviceName) ($($be.OldVersion))`r`n" } }
                catch { $state.Log += "Backup skipped for $($bi.DeviceName): $($_.Exception.Message)`r`n" }
            }
            if ($bk.Count) { Save-BackupManifest $setDir $bk }
        } catch { $state.Log += "Backup skipped: $($_.Exception.Message)`r`n" }        $ok = 0
        foreach ($id in @($state.Ids)) { $state.ItemStatus[$id] = 'wait' }
        $wuIds = @($state.Ids | Where-Object { $_ -notlike 'cat:*' -and $_ -notlike 'ms:*' -and $_ -notlike 'ven:*' })
        if ($wuIds.Count -gt 0 -and $state.Cancel) {
            foreach ($id in $wuIds) { $state.ItemStatus[$id] = 'skip'; $state.ItemReason[$id] = 'Stopped by request' }
            $wuIds = @()
        }
        if ($wuIds.Count -gt 0) {
            $session = New-Object -ComObject Microsoft.Update.Session
            $searcher = $session.CreateUpdateSearcher()
            $r = $searcher.Search("IsInstalled=0 and Type='Driver' and IsHidden=0")
            $coll = New-Object -ComObject Microsoft.Update.UpdateColl
            foreach ($u in $r.Updates) {
                if ($wuIds -contains $u.Identity.UpdateID) {
                    if (-not $u.EulaAccepted) { $u.AcceptEula() }
                    [void]$coll.Add($u)
                    $state.ItemStatus[$u.Identity.UpdateID] = 'dl'
                }
            }
            foreach ($id in $wuIds) { if ($state.ItemStatus[$id] -eq 'wait') { $state.ItemStatus[$id] = 'skip'; $state.ItemReason[$id] = 'Not found (may already be installed)' } }
            if ($coll.Count -gt 0) {
                $state.Status = "Downloading $($coll.Count) items from Windows Update..."
                $dl = $session.CreateUpdateDownloader()
                $dl.Updates = $coll
                [void]$dl.Download()
                $toInstall = New-Object -ComObject Microsoft.Update.UpdateColl
                foreach ($u in $coll) {
                    if ($u.IsDownloaded) { [void]$toInstall.Add($u) } else { $state.Log += "[FAILED] $($u.Title): download failed`r`n"; $state.ItemStatus[$u.Identity.UpdateID] = 'fail'; $state.ItemReason[$u.Identity.UpdateID] = 'Download failed' }
                }
                if ($toInstall.Count -gt 0) {
                    $state.Status = "Installing $($toInstall.Count) items from Windows Update..."
                    foreach ($u in $toInstall) { $state.ItemStatus[$u.Identity.UpdateID] = 'inst' }
                    $inst = $session.CreateUpdateInstaller()
                    $inst.Updates = $toInstall
                    $res = $inst.Install()
                    for ($i = 0; $i -lt $toInstall.Count; $i++) {
                        $code = $res.GetUpdateResult($i).ResultCode   # 2 = succeeded
                        if ($code -eq 2) { $ok++ }
                        $txt = switch ($code) { 2 { 'succeeded' } 3 { 'partly succeeded' } 4 { 'failed' } 5 { 'cancelled' } default { "code $code" } }
                        $uid = $toInstall.Item($i).Identity.UpdateID
                        if ($code -eq 2) { $state.ItemStatus[$uid] = 'ok' } else { $state.ItemStatus[$uid] = 'fail'; $state.ItemReason[$uid] = $txt }
                        $state.Log += $(if ($code -eq 2) { "[OK] " } else { "[FAILED] " }) + "$($toInstall.Item($i).Title) ($txt)`r`n"
                    }
                    if ($res.RebootRequired) { $state.Reboot = $true }
                }
            } else {
                $state.Log += "The selected Windows Update items were not found (they may already be installed)`r`n"
            }
        }
        foreach ($item in @($state.CatalogItems)) {
            if ($state.Cancel) { $state.ItemStatus[$item.Id] = 'skip'; $state.ItemReason[$item.Id] = 'Stopped by request'; $state.Log += "[SKIPPED] $($item.Title): Stopped by request`r`n"; continue }
            # A catalog in a folder normal users can edit could be tampered with, so never install from it as admin.
            if ($state.BlockCatalog -and $item.Source -eq 'Catalog') { $state.ItemStatus[$item.Id] = 'skip'; $state.ItemReason[$item.Id] = 'Program folder is writable by normal users'; $state.Log += "[SKIPPED] $($item.Title): the program folder is writable by normal users and RequireProtectedFolder is set. Install with TierS-Driver-Manage-Setup.exe or turn this option off`r`n"; continue }
            $c = $item.Raw
            $state.Status = "Downloading and verifying: $($item.Title)"
            $state.ItemStatus[$item.Id] = 'dl'
            $work = Join-Path $env:TEMP ('DriverScan_' + [guid]::NewGuid().ToString('N'))
            $r = if ($item.Source -eq 'Microsoft Catalog') { Install-MsCatalogItem $item $work } elseif ($item.Source -eq 'NVIDIA') { Install-VendorItem $item $work } else { Install-CatalogItem $c $work }
            if ($r.Success) { $ok++; $state.ItemStatus[$item.Id] = 'ok'; $state.Log += "[OK] $($item.Title)`r`n" } else { $state.ItemStatus[$item.Id] = 'fail'; $state.ItemReason[$item.Id] = [string]$r.Message; $state.Log += "[FAILED] $($item.Title): $($r.Message)`r`n" }
            if ($r.Reboot) { $state.Reboot = $true }
            Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
        }
        foreach ($id in @($state.Ids)) { if ($state.ItemStatus[$id] -eq 'wait') { $state.ItemStatus[$id] = 'skip'; $state.ItemReason[$id] = 'Stopped by request' } }
        $state.Installed = $ok
        $state.Status = 'This install round is finished'
    } catch {
        $state.Error = $_.Exception.Message
    }
    $state.Done = $true
}

function Start-Worker($block, $state) {
    $rs = [runspacefactory]::CreateRunspace()
    $rs.ApartmentState = 'STA'
    $rs.ThreadOptions = 'ReuseThread'
    $rs.Open()
    $ps = [powershell]::Create()
    $ps.Runspace = $rs
    # One script text: param first, then the shared engine functions, then the worker body (AddStatement stalls the pipeline).
    $body = [regex]::Replace($block.ToString(), '^\s*param\(\$state\)', '')
    [void]$ps.AddScript("param(`$state)`r`n" + $Core + "`r`n" + $body).AddArgument($state)
    [void]$ps.BeginInvoke()
    return @{ PS = $ps; RS = $rs }
}

function Stop-Worker($w) {
    # BeginStop does not wait, so a stuck Windows Update call cannot freeze the window.
    if ($w) { try { [void]$w.PS.BeginStop($null, $null) } catch {} }
}

function New-State {
    return [hashtable]::Synchronized(@{
        Status = ''; Done = $false; Error = $null; Items = @(); Inventory = @(); Log = ''; Reboot = $false
        Ids = @(); CatalogItems = @(); RestorePoint = $false; Missing = @(); Installed = 0; BlockCatalog = ($script:FolderWritable -and $script:RequireProtectedFolder)
        CatalogPath = $script:CatalogPath; OnlineCatalogPath = (Join-Path $PSScriptRoot 'driverdb.online.json'); UpdateRepo = $script:UpdateRepo; PublicKey = $script:UpdatePublicKey
        MissingDevices = @(); Unresolved = @(); ItemStatus = [hashtable]::Synchronized(@{}); ItemReason = [hashtable]::Synchronized(@{}); Cancel = $false
    })
}

function Write-LogFile($text) {
    if (-not $text) { return }
    try {
        $dir = Split-Path $script:LogFile
        New-Item -ItemType Directory -Force $dir | Out-Null
        Add-Content -Path $script:LogFile -Value ("[{0}]`r`n{1}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $text) -Encoding UTF8
    } catch {}
}

# Folds the per-driver outcome of one install job into the session's result list (a retry replaces the earlier row).
function Add-InstallResults($s) {
    $ids = @($s.Ids)
    $keep = @($script:ctx.Results | Where-Object { $ids -notcontains $_.Id })
    $new = foreach ($id in $ids) {
        $it = $script:ctx.ItemsById[$id]
        $code = $s.ItemStatus[$id]; $reason = $s.ItemReason[$id]
        if ($code -notin 'ok', 'fail', 'skip') { $code = 'skip'; $reason = 'Not installed' }
        [pscustomobject]@{ Id = $id; Title = $it.Title; Class = $it.Class; Source = $it.Source; Date = $it.Date; SizeMB = $it.SizeMB; Status = $code; Reason = $reason }
    }
    $script:ctx.Results = @($keep) + @($new)
}

# Final screen of a run: result rows, banner with next steps, log file. Used by every way a run can end.
function Complete-Run {
    $r = Show-Results $script:ctx.Results $script:ctx.NeedReboot
    $script:txtLog.Text = $script:ctx.Log
    Write-LogFile $script:ctx.Log
    $script:ctx.Auto = $false
    Set-Busy $false
    $script:lblStatus.Text = 'Done'   # the banner above the list carries the details; say it once
}

function Start-Job2($mode, $block, $state) {
    $script:ctx.State = $state
    $script:ctx.Mode = $mode
    $script:ctx.Worker = Start-Worker $block $state
    Set-Busy $true
    $script:timer.Start()
}

function Start-ScanJob { Start-Job2 'scan' $ScanBlock (New-State) }

function Start-InstallJob($ids) {
    $st = New-State
    $st.Ids = $ids
    $st.CatalogItems = @($ids | Where-Object { $_ -like 'cat:*' -or $_ -like 'ms:*' -or $_ -like 'ven:*' } | ForEach-Object { $script:ctx.ItemsById[$_] })
    $st.RestorePoint = ($script:chkRestore.Checked -and -not $script:ctx.RestoreDone)
    $script:ctx.RestoreDone = $true
    if ($script:DryRun) {
        # self-test: walk the whole result path without touching the system
        foreach ($id in $ids) { $st.ItemStatus[$id] = 'ok' }
        $st.Log = "[TEST] Would install $($ids.Count) items (nothing was installed)`r`n"; $st.Installed = $ids.Count; $st.Done = $true
        $script:ctx.State = $st; $script:ctx.Mode = 'install'; Set-Busy $true; $script:timer.Start()
        return
    }
    Start-Job2 'install' $InstallBlock $st
}

# ---------- program self-update (own small worker + timer, independent of scan/install) ----------
$UpdateBlock = {
    param($state)
    try { $state.Manifest = Get-AppUpdate $state.Repo $state.Version $state.PublicKey } catch { $state.Error = $_.Exception.Message }
    $state.Done = $true
}
$ApplyBlock = {
    param($state)
    $work = Join-Path $env:TEMP ('TierS_upd_' + [guid]::NewGuid().ToString('N'))
    try { $state.Result = Install-AppUpdate $state.Manifest $state.InstallDir $work $state.Version } catch { $state.Result = [pscustomobject]@{ Success = $false; Message = $_.Exception.Message } }
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    $state.Done = $true
}
function Start-UpdateJob($mode, $block, $manifest) {
    $st = [hashtable]::Synchronized(@{ Done = $false; Error = $null; Manifest = $manifest; Result = $null; Repo = $script:UpdateRepo; Version = $script:AppVersion; PublicKey = $script:UpdatePublicKey; InstallDir = $PSScriptRoot })
    $script:upd = @{ State = $st; Mode = $mode; Worker = (Start-Worker $block $st) }
    $script:updTimer.Start()
}
function Start-UpdateCheck { if ($script:UpdateEnabled) { Start-UpdateJob 'check' $UpdateBlock $null } }
function Invoke-UpdateClick {
    if ($script:isBusy -or -not $script:UpdateManifest) { return }
    $script:btnUpdate.Enabled = $false
    $script:lblStatus.Text = "Downloading and verifying the update to version $($script:UpdateManifest.version)..."
    Start-UpdateJob 'apply' $ApplyBlock $script:UpdateManifest
}
function Invoke-UpdateTick {
    $s = $script:upd.State
    if (-not $s -or -not $s.Done) { return }
    $script:updTimer.Stop(); Stop-Worker $script:upd.Worker
    if ($script:upd.Mode -eq 'check') {
        if ($s.Manifest) {
            $script:UpdateManifest = $s.Manifest
            $script:btnUpdate.Text = "Update to version $($s.Manifest.version)"; $script:btnUpdate.Visible = $true; $script:btnUpdate.Enabled = -not $script:isBusy
            $script:toolTip.SetToolTip($script:btnUpdate, [string]$s.Manifest.notes)
        } elseif ($s.Error) { $script:ctx.Log += "Program update check: $($s.Error)`r`n"; $script:txtLog.Text = $script:ctx.Log }
        return
    }
    if ($s.Result.Success) {
        $script:ctx.Log += $s.Result.Message + "`r`n"; Write-LogFile $script:ctx.Log
        $script:lblStatus.Text = 'Done: ' + $s.Result.Message + ' Restarting the program...'
        $exe = Join-Path $PSScriptRoot 'TierS Driver Manage.exe'
        if (Test-Path $exe) { Start-Process $exe -ArgumentList '--wait-for-previous' }   # the launcher waits for this instance to exit, then starts
        $script:form.Close()
    } else {
        $script:ctx.Log += "[FAILED] Program update: $($s.Result.Message)`r`n"; $script:txtLog.Text = $script:ctx.Log; Write-LogFile $script:ctx.Log
        $script:lblStatus.Text = 'Stopped: the program update failed (your existing files were not changed). See the log for details.'
        $script:btnUpdate.Enabled = $true
    }
}
# ---------- window actions (DriverUI.ps1 wires these to the buttons and the timer) ----------
function Invoke-ScanClick {
    $script:grid.Rows.Clear()
    $script:ctx.Auto = $false; $script:ctx.Pass = 0; $script:ctx.Log = ''; $script:ctx.Installed = 0; $script:ctx.NeedReboot = $false
    $script:ctx.Results = @(); $script:ctx.Declined = @(); $script:ctx.ItemsById = @{}; $script:ctx.RestoreDone = $false
    $script:txtLog.Text = ''
    Set-Banner 'hidden' ''
    $script:gridMode = 'review'
    Start-ScanJob
}

# Installs whatever is ticked: no confirmation dialog (the button itself says how many), progress shows in the list.
function Invoke-InstallClick {
    $ids = @(); $declined = @()
    for ($i = $script:grid.Rows.Count - 1; $i -ge 0; $i--) {
        $row = $script:grid.Rows[$i]
        if ($row.Cells[0].Value -eq $true) { $ids = @($row.Tag) + $ids } else { $declined += $row.Tag; $script:grid.Rows.RemoveAt($i) }
    }
    if ($ids.Count -eq 0) { return }
    $script:ctx.Declined = @($script:ctx.Declined) + $declined
    $script:ctx.Auto = $true; $script:ctx.Pass = 1
    Enter-RunMode
    Start-InstallJob $ids
}

# Puts the failed drivers back in the list, ticked, and installs them again.
function Invoke-RetryClick {
    $failed = @($script:ctx.Results | Where-Object { $_.Status -eq 'fail' })
    if ($failed.Count -eq 0) { return }
    $script:grid.Rows.Clear()
    $script:gridMode = 'review'
    $script:syncing = $true
    foreach ($r in $failed) { Add-UpdateRow $script:ctx.ItemsById[$r.Id]; $script:grid.Rows[$script:grid.Rows.Count - 1].Cells[0].Value = $true }
    $script:syncing = $false
    Invoke-InstallClick
}

# Rolls one backed-up device back (asks first: it removes the driver the device uses now).
function Invoke-RollbackClick {
    $row = $script:bkGrid.CurrentRow
    if (-not $row -or -not $row.Tag) { return }
    $b = $row.Tag
    $ask = [Windows.Forms.MessageBox]::Show("Roll back $($b.Device) to version $($b.OldVersion)?`nThe driver it uses now will be removed from this PC. A restart may be needed.", 'Roll back driver', 'YesNo', 'Question')
    if ($ask -ne 'Yes') { return }
    $script:form.Cursor = 'WaitCursor'
    try { $r = Restore-DeviceDriver $b } finally { $script:form.Cursor = 'Default' }
    $script:ctx.Log += $(if ($r.Success) { '[OK] ' } else { '[FAILED] ' }) + "Roll back $($b.Device): $($r.Message)`r`n"
    $script:txtLog.Text = $script:ctx.Log; Write-LogFile $script:ctx.Log
    $script:lblStatus.Text = $(if ($r.Success) { 'Done: ' } else { 'Stopped: ' }) + $r.Message
    Update-BackupsView
}
function Invoke-BannerClick($name) {
    switch ($name) {
        'stop' { if ($script:ctx.State) { $script:ctx.State.Cancel = $true; Update-RunUi $script:ctx.State } }
        'retry' { Invoke-RetryClick }
        'rescan' { Invoke-ScanClick }
        'restart' {
            if (-not $script:DryRun) { Start-Process shutdown.exe -ArgumentList '/r', '/t', '30', '/c', 'TierS Driver Manage: restart to finish installing drivers' -WindowStyle Hidden }
            Set-Banner 'warn' 'The computer will restart in 30 seconds · save any open work' @(@{ Name = 'abort'; Text = 'Cancel restart'; Kind = 'primary' })
        }
        'abort' {
            if (-not $script:DryRun) { Start-Process shutdown.exe -ArgumentList '/a' -WindowStyle Hidden }
            Set-Banner 'warn' 'Restart cancelled · drivers will work fully after a restart' @(@{ Name = 'restart'; Text = 'Restart now'; Kind = 'primary' })
        }
        'later' { Set-Banner 'warn' 'Not restarted yet · installed drivers will work fully after a restart' @(@{ Name = 'restart'; Text = 'Restart now'; Kind = 'primary' }) }
    }
}

function Invoke-Tick {
    $s = $script:ctx.State
    if (-not $s) { return }
    $script:lblStatus.Text = $s.Status
    $shown = $script:ctx.Log + $s.Log
    if ($shown -and $script:txtLog.Text -ne $shown) { $script:txtLog.Text = $shown }
    if ($script:ctx.Mode -eq 'install' -and $script:gridMode -eq 'run') { Update-RunUi $s }
    if (-not $s.Done) { return }
    $script:timer.Stop()
    Stop-Worker $script:ctx.Worker
    $script:ctx.Worker = $null
    $script:ctx.Log += $s.Log
    $mode = $script:ctx.Mode

    if ($s.Error) {
        $script:ctx.Log += "Stopped: $($s.Error)`r`n"
        if ($mode -eq 'install') {
            $script:ctx.Installed += $s.Installed
            if ($s.Reboot) { $script:ctx.NeedReboot = $true }
            Add-InstallResults $s
            Complete-Run
        } else {
            $script:txtLog.Text = $script:ctx.Log
            Write-LogFile $script:ctx.Log
            $script:ctx.Auto = $false
            Set-Busy $false
            Show-View 'log'
            if ($script:ctx.Results.Count -gt 0) { Complete-Run }
        }
        $script:lblStatus.Text = "Stopped: $($s.Error)"
        return
    }

    if ($mode -eq 'scan') {
        $doneIds = @($script:ctx.Results | ForEach-Object { $_.Id })
        $items = @($s.Items | Where-Object { $script:ctx.Declined -notcontains $_.Id -and $doneIds -notcontains $_.Id })
        $script:grid.Rows.Clear()
        $script:syncing = $true
        foreach ($it in $items) { $script:ctx.ItemsById[$it.Id] = $it; Add-UpdateRow $it }
        foreach ($u in @($s.Unresolved)) { Add-InfoRow $u }   # devices with no driver and no automatic match: shown, not installable
        $script:syncing = $false
        $script:invGrid.Rows.Clear()
        foreach ($d in ($s.Inventory | Sort-Object { if ($null -eq $_.Age) { -1 } else { $_.Age } } -Descending)) { Add-InventoryRow $d }
        $oldCount = @($s.Inventory | Where-Object { $null -ne $_.Age -and $_.Age -gt $script:OldYears -and $_.Maker -notlike 'Microsoft*' }).Count
        $script:scanned = $true
        Update-Cards $s.Inventory $items.Count @($s.Missing).Count $oldCount
        $script:ctx.Log += "Installed drivers: $(@($s.Inventory).Count); older than $($script:OldYears) years (Microsoft drivers not counted): $oldCount`r`n"
        if ($script:FolderWritable) { $script:ctx.Log += "Warning: the program folder is writable by normal users. If malicious software edits driverdb.json it would be installed with administrator rights (install the program in C:\Program Files for safety)`r`n" }
        if ($s.Missing.Count -gt 0) {
            $script:ctx.Log += "Devices without a working driver ($($s.Missing.Count)):`r`n  - " + ($s.Missing -join "`r`n  - ") + "`r`n"
        }
        $script:txtLog.Text = $script:ctx.Log
        $script:ctx.Auto = $false
        if ($items.Count -eq 0) {
            $script:ctx.Log += "Scan complete: no newer drivers found from Windows Update or your catalog`r`n"
            if ($script:ctx.Results.Count -gt 0) { Complete-Run; return }
            Write-LogFile $script:ctx.Log
            $script:txtLog.Text = $script:ctx.Log
            Set-Busy $false
            Enter-ReviewMode 0
            $script:lblStatus.Text = 'Scan complete: no newer drivers found'
            return
        }
        Set-Busy $false
        Enter-ReviewMode @($script:ctx.Results | Where-Object { $_.Status -eq 'ok' }).Count
        Show-View 'updates'
        return
    }

    if ($mode -eq 'install') {
        $script:ctx.Installed += $s.Installed
        if ($s.Reboot) { $script:ctx.NeedReboot = $true }
        Add-InstallResults $s
        if ($script:ctx.Auto -and $s.Installed -gt 0 -and -not $s.Cancel -and $script:ctx.Pass -lt $script:MaxPasses) {
            $script:ctx.Pass++
            $script:ctx.Log += "--- Round $($script:ctx.Pass): scanning again in case newly detected hardware needs drivers ---`r`n"
            [void](Show-Results $script:ctx.Results $script:ctx.NeedReboot)
            Set-Banner 'info' 'This install round is finished · scanning again in case newly detected hardware needs drivers' @()
            Start-ScanJob
            return
        }
        Complete-Run
        return
    }
}


if ($SelfTest) {
    # Headless checks. Nothing is ever installed.
    $fail = 0
    function Check($name, $cond) { if ($cond) { "PASS  $name" } else { "FAIL  $name"; $script:fail++ } }

    # 1) installed-driver inventory
    $inv = @(Get-InstalledDrivers)
    Check "inventory returns drivers ($($inv.Count))" ($inv.Count -gt 0)
    Check 'inventory items carry hardware IDs' (@($inv | Where-Object { $_.HardwareId.Count -gt 0 }).Count -gt 0)

    # 2) catalog matching: newer matches, older/other does not
    $dev = @($inv | Where-Object { $_.HardwareId.Count -gt 0 -and (ConvertTo-Ver $_.Version) } | Select-Object -First 1)[0]
    $pat = $dev.HardwareId[0]
    $curV = ConvertTo-Ver $dev.Version
    $newer = ('{0}.{1}.{2}.{3}' -f ($curV.Major + 1), 0, 0, 0)
    $cat = @(
        [pscustomobject]@{ name = 'T-newer'; match = @($pat); version = $newer; publisher = 'T'; date = '2026-01-01'; sizeMB = 1; url = 'https://example.com/x.zip'; sha256 = 'AA'; type = 'inf-zip' },
        [pscustomobject]@{ name = 'T-same'; match = @($pat); version = $dev.Version; publisher = 'T'; date = '2026-01-01'; sizeMB = 1; url = 'https://example.com/y.zip'; sha256 = 'AA'; type = 'inf-zip' },
        [pscustomobject]@{ name = 'T-other'; match = @('PCI\VEN_FFFF&DEV_FFFF*'); version = '99.0.0.0'; publisher = 'T'; date = '2026-01-01'; sizeMB = 1; url = 'https://example.com/z.zip'; sha256 = 'AA'; type = 'inf-zip' }
    )
    $hits = @(Find-CatalogUpdates $inv $cat)
    Check 'catalog: newer version matched' (@($hits | Where-Object { $_.Id -like 'cat:T-newer *' }).Count -eq 1)
    Check 'catalog: same version ignored' (@($hits | Where-Object { $_.Id -like 'cat:T-same *' }).Count -eq 0)
    Check 'catalog: unrelated hardware ignored' (@($hits | Where-Object { $_.Id -like 'cat:T-other *' }).Count -eq 0)

    # 3) SHA-256 verification
    $tmp = Join-Path $env:TEMP ('dscan_' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $tmp | Out-Null
    $f1 = Join-Path $tmp 'a.bin'; [IO.File]::WriteAllText($f1, 'hello driver')
    $good = (Get-FileHash $f1 -Algorithm SHA256).Hash
    Check 'sha256: correct hash accepted' ((Test-FileSha256 $f1 $good.ToLower()) -eq $true)
    $threw = $false; try { [void](Test-FileSha256 $f1 ('0' * 64)) } catch { $threw = $true }
    Check 'sha256: wrong hash rejected' $threw
    $threw = $false; try { [void](Test-FileSha256 $f1 $null) } catch { $threw = $true }
    Check 'sha256: missing hash rejected' $threw

    # 4) URL allowlist
    $threw = $false; try { Assert-AllowedUrl 'http://example.com/a.zip' @() } catch { $threw = $true }
    Check 'url: plain http rejected' $threw
    $threw = $false; try { Assert-AllowedUrl 'https://evil.com/a.zip' @('vendor.com') } catch { $threw = $true }
    Check 'url: host outside allowlist rejected' $threw
    $threw = $false; try { Assert-AllowedUrl 'https://dl.vendor.com/a.zip' @('vendor.com') } catch { $threw = $true }
    Check 'url: allowed host accepted' (-not $threw)

    # 5) zip path traversal
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $evil = Join-Path $tmp 'evil.zip'
    $zs = [IO.File]::Create($evil); $za = New-Object IO.Compression.ZipArchive($zs, [IO.Compression.ZipArchiveMode]::Create)
    $en = $za.CreateEntry('../escape.txt'); $sw = New-Object IO.StreamWriter($en.Open()); $sw.Write('x'); $sw.Dispose(); $za.Dispose(); $zs.Dispose()
    $threw = $false; try { Expand-SafeZip $evil (Join-Path $tmp 'out1') } catch { $threw = $true }
    Check 'zip: path traversal rejected' ($threw -and -not (Test-Path (Join-Path $tmp 'escape.txt')))
    $okz = Join-Path $tmp 'ok.zip'
    $zs = [IO.File]::Create($okz); $za = New-Object IO.Compression.ZipArchive($zs, [IO.Compression.ZipArchiveMode]::Create)
    $en = $za.CreateEntry('drv/test.inf'); $sw = New-Object IO.StreamWriter($en.Open()); $sw.Write('[Version]'); $sw.Dispose(); $za.Dispose(); $zs.Dispose()
    Expand-SafeZip $okz (Join-Path $tmp 'out2')
    Check 'zip: safe archive extracted' (Test-Path (Join-Path $tmp 'out2\drv\test.inf'))
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue

    # 5b) rules added for multi-PC catalogs
    $hi = Get-HostInfo
    function New-Fam($os, $arch) { [pscustomobject]@{ name = 'Fam'; match = @('PCI\VEN_10DE&DEV_*'); class = 'DISPLAY'; version = '2.0.0.0'; publisher = 'N'; os = @($os); arch = @($arch) } }
    $gpu = @([pscustomobject]@{ Name = 'GPU'; Class = 'DISPLAY'; Maker = 'NVIDIA'; Version = '1.0.0.1'; HardwareId = @('PCI\VEN_10DE&DEV_2504&SUBSYS_1&REV_A1') })
    $aud = @([pscustomobject]@{ Name = 'HDMI audio'; Class = 'MEDIA'; Maker = 'NVIDIA'; Version = '1.0.0.1'; HardwareId = @('PCI\VEN_10DE&DEV_228E&SUBSYS_1&REV_A1') })
    Check 'family: wildcard + class matches the GPU' (@(Find-CatalogUpdates $gpu @(New-Fam $hi.Os $hi.Arch)).Count -eq 1)
    Check 'family: class filter skips same-vendor audio device' (@(Find-CatalogUpdates $aud @(New-Fam $hi.Os $hi.Arch)).Count -eq 0)
    Check 'os filter: other Windows version skipped' (@(Find-CatalogUpdates $gpu @(New-Fam '7' $hi.Arch)).Count -eq 0)
    Check 'arch filter: other CPU type skipped' (@(Find-CatalogUpdates $gpu @(New-Fam $hi.Os 'arm64x')).Count -eq 0)

    $tmp2 = Join-Path $env:TEMP ('dscan2_' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory $tmp2 | Out-Null
    $lf = Join-Path $tmp2 'setup.exe'; [IO.File]::WriteAllText($lf, 'not a real installer')
    $sha = (Get-FileHash $lf -Algorithm SHA256).Hash
    $base = @{ name = 'T'; url = 'https://us.download.nvidia.com/x/setup.exe'; type = 'installer'; silentArgs = '/s' }
    $r = Install-CatalogItem ([pscustomobject]($base + @{ sha256 = ('0' * 64); signer = 'NVIDIA' })) (Join-Path $tmp2 'w1') $lf
    Check 'install: wrong sha256 stops before running' ((-not $r.Success) -and $r.Message -match 'SHA-256')
    $r = Install-CatalogItem ([pscustomobject]($base + @{ sha256 = $sha })) (Join-Path $tmp2 'w2') $lf
    Check 'install: installer without signer refused' ((-not $r.Success) -and $r.Message -match 'signer')
    $r = Install-CatalogItem ([pscustomobject]($base + @{ sha256 = $sha; signer = 'NVIDIA' })) (Join-Path $tmp2 'w3') $lf
    Check 'install: unsigned file refused even with the right hash' ((-not $r.Success) -and $r.Message -match 'signature')
    $r = Install-CatalogItem ([pscustomobject]@{ name = 'T'; url = 'https://evil.example.com/setup.exe'; type = 'installer'; silentArgs = '/s'; sha256 = $sha; signer = 'NVIDIA' }) (Join-Path $tmp2 'w4') $lf
    Check 'install: host outside the built-in list refused' ((-not $r.Success) -and $r.Message -match 'allowed list')
    Remove-Item $tmp2 -Recurse -Force -ErrorAction SilentlyContinue
    # 5c) Microsoft Update Catalog provider
    Check 'ms key: PCI id trimmed to VEN+DEV' ((Get-CatalogHardwareKey 'PCI\VEN_10EC&DEV_8125&SUBSYS_7D991462&REV_05') -eq 'PCI\VEN_10EC&DEV_8125')
    Check 'ms key: USB id trimmed to VID+PID' ((Get-CatalogHardwareKey 'USB\VID_8087&PID_0033&REV_0000') -eq 'USB\VID_8087&PID_0033')
    Check 'ms key: ACPI id has no key (skipped)' ($null -eq (Get-CatalogHardwareKey 'ACPI\VEN_PNP&DEV_0C0B'))
    function New-CatRow($id, $ver, $class, $prod) { '<tr id="' + $id + '_R0"><td></td><td>Acme NIC (' + $ver + ')</td><td>' + $prod + '</td><td>' + $class + '</td><td>5/7/2026</td><td>' + $ver + '</td><td>320 KB 328320</td><td></td></tr>' }
    $html = (New-CatRow 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee01' '9.0.0.1' 'Drivers (Networking)' 'Windows 11 Client') +
            (New-CatRow 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee02' '9.0.0.5' 'Updates' 'Windows 11 Client') +
            (New-CatRow 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee03' '10.0.0.0' 'Drivers (Networking)' 'Windows 7') +
            (New-CatRow 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee04' '9.0.0.3' 'Drivers (Networking)' 'Windows 10 Client')
    $parsed = @(ConvertFrom-MsCatalogHtml $html)
    Check 'ms html: 4 rows parsed with version and size' (($parsed.Count -eq 4) -and ($parsed[0].Version -eq '9.0.0.1') -and ($parsed[0].SizeMB -eq 0.31))
    $best = Select-BestCatalogRow $parsed (Get-HostInfo)
    Check 'ms best: newest DRIVER row for this Windows (not Updates, not Windows 7)' ($best.Version -eq '9.0.0.3')
    $fakeDev = @([pscustomobject]@{ Name = 'Fake NIC'; Class = 'NET'; Maker = 'Acme'; Version = '1.0.0.0'; HardwareId = @('PCI\VEN_AAAA&DEV_BBBB&SUBSYS_1&REV_01') })
    $fakeRows = @($parsed[3])
    Check 'ms find: newer catalog driver offered' (@(Find-MsCatalogUpdates $fakeDev (Get-HostInfo) 40 { param($k) $fakeRows }).Count -eq 1)
    $fakeDev[0].Version = '9.0.0.3'
    Check 'ms find: same version not offered' (@(Find-MsCatalogUpdates $fakeDev (Get-HostInfo) 40 { param($k) $fakeRows }).Count -eq 0)
    $fakeDev[0].Version = '1.0.0.0'; $fakeDev[0].Maker = 'Microsoft'
    Check 'ms find: Microsoft inbox devices skipped' (@(Find-MsCatalogUpdates $fakeDev (Get-HostInfo) 40 { param($k) $fakeRows }).Count -eq 0)
    try {
        $net = @(Search-MsCatalog 'PCI\VEN_10EC&DEV_8125')
        Check "ms online: catalog search returns rows ($($net.Count))" (($net.Count -gt 0) -and [bool]$net[0].Version)
    } catch { "SKIP  ms online search ($($_.Exception.Message))" }

    # date rule + dedupe
    $dOld = [pscustomobject]@{ Name = 'NIC'; Class = 'NET'; Maker = 'Acme'; Version = '1.0.0.0'; Date = [datetime]'2026-05-08'; HardwareId = @('PCI\VEN_AAAA&DEV_BBBB&SUBSYS_1&REV_01') }
    $rowOlder = [pscustomobject]@{ UpdateId = 'u9'; Title = 'Acme NIC (2.0.0.0)'; Products = 'Windows 11 Client'; Class = 'Drivers (Networking)'; Date = [datetime]'2026-05-07'; Version = '2.0.0.0'; SizeMB = 1 }
    Check 'ms date rule: higher version but OLDER real date is not offered' (@(Find-MsCatalogUpdates @($dOld) (Get-HostInfo) 40 { param($k) @($rowOlder) }).Count -eq 0)
    $dIntel = [pscustomobject]@{ Name = 'SMBus'; Class = 'SYSTEM'; Maker = 'INTEL'; Version = '10.1.46.3'; Date = [datetime]'1968-07-18'; HardwareId = @('PCI\VEN_AAAA&DEV_BBBB&SUBSYS_1&REV_01') }
    $rowIntel = [pscustomobject]@{ UpdateId = 'u8'; Title = 'Intel chipset (10.1.46.5)'; Products = 'Windows 11 Client'; Class = 'Drivers (System)'; Date = [datetime]'1968-07-17'; Version = '10.1.46.5'; SizeMB = 1 }
    Check 'ms date rule: fake 1968 INF dates are ignored, version decides' (@(Find-MsCatalogUpdates @($dIntel) (Get-HostInfo) 40 { param($k) @($rowIntel) }).Count -eq 1)
    $dTwo = @($dIntel, [pscustomobject]@{ Name = 'LPC'; Class = 'SYSTEM'; Maker = 'INTEL'; Version = '10.1.46.3'; Date = [datetime]'1968-07-18'; HardwareId = @('PCI\VEN_AAAA&DEV_CCCC&SUBSYS_1&REV_01') })
    Check 'ms dedupe: same package for two devices is offered once' (@(Find-MsCatalogUpdates $dTwo (Get-HostInfo) 40 { param($k) @($rowIntel) }).Count -eq 1)
    # stale WMI version must not trigger an offer when Plug and Play already has the newer driver
    $real = $inv | Where-Object { $_.InstanceId -and $_.HardwareId.Count -gt 0 -and (ConvertTo-Ver (Get-LiveDriverVersion $_.InstanceId)) } | Select-Object -First 1
    Check 'live: Plug and Play returns a driver version' ([bool]$real)
    $stale = [pscustomobject]@{ InstanceId = $real.InstanceId; Name = $real.Name; Class = $real.Class; Maker = $real.Maker; Version = '0.0.0.1'; Date = $null; HardwareId = $real.HardwareId }
    $liveV = Get-LiveDriverVersion $real.InstanceId
    $entry = [pscustomobject]@{ name = 'Stale'; match = @($real.HardwareId[0]); version = $liveV; publisher = 'T'; date = '2026-01-01'; sizeMB = 1; url = 'https://example.com/x'; sha256 = 'AA'; type = 'installer' }
    Check 'live: offer suppressed when the live version already equals the catalog version' (@(Find-CatalogUpdates @($stale) @($entry)).Count -eq 0)
    # catalog page must be recognised; failures must be loud, not silent
    $okRow = New-CatRow 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeee05' '9.0.0.1' 'Drivers (Networking)' 'Windows 11 Client'
    Check 'ms page: real result rows are accepted' (@(Get-MsCatalogRows $okRow).Count -eq 1)
    Check 'ms page: genuine "no results" page gives an empty list' (@(Get-MsCatalogRows '<div>We did not find any results for "x"</div>').Count -eq 0)
    $threw = $false; try { [void](Get-MsCatalogRows '<html><body>Something completely different</body></html>') } catch { $threw = $true }
    Check 'ms page: unknown layout throws instead of silently returning nothing' $threw
    $threw = $false; try { [void](Find-MsCatalogUpdates $fakeDev (Get-HostInfo) 40 { param($k) throw 'boom' }) } catch { $threw = $true }
    $fakeDev[0].Maker = 'Acme'
    $threw = $false; try { [void](Find-MsCatalogUpdates $fakeDev (Get-HostInfo) 40 { param($k) throw 'boom' }) } catch { $threw = $true }
    Check 'ms find: every query failing throws (the user is told)' $threw
    $two = @($fakeDev[0], [pscustomobject]@{ Name = 'NIC2'; Class = 'NET'; Maker = 'Acme'; Version = '1.0.0.0'; HardwareId = @('PCI\VEN_AAAA&DEV_DDDD&SUBSYS_1&REV_01') })
    $calls = 0
    $r = @(Find-MsCatalogUpdates $two (Get-HostInfo) 40 { param($k) $script:calls++; if ($script:calls -eq 1) { throw 'boom' } else { @($fakeRows) } })
    Check 'ms find: one failed query is counted, the rest still work' (($MsCatalogFailures -eq 1) -and ($r.Count -eq 1))
    $nv = [pscustomobject]@{ name = 'NV'; match = @('PCI\VEN_10DE&DEV_*'); class = 'DISPLAY'; nameLike = @('NVIDIA GeForce RTX*', 'NVIDIA GeForce GTX 16*'); version = '9.0.0.0'; publisher = 'N' }
    $mk = { param($n) @([pscustomobject]@{ Name = $n; Class = 'DISPLAY'; Maker = 'NVIDIA'; Version = '1.0.0.0'; HardwareId = @('PCI\VEN_10DE&DEV_1234&SUBSYS_1&REV_A1') }) }
    Check 'nameLike: GeForce RTX card matches' (@(Find-CatalogUpdates (& $mk 'NVIDIA GeForce RTX 3060') @($nv)).Count -eq 1)
    Check 'nameLike: older GeForce GTX 9xx does not match' (@(Find-CatalogUpdates (& $mk 'NVIDIA GeForce GTX 970') @($nv)).Count -eq 0)
    Check 'nameLike: workstation Quadro does not match' (@(Find-CatalogUpdates (& $mk 'NVIDIA Quadro P2000') @($nv)).Count -eq 0)
    $td = Join-Path $env:TEMP ('dscan3_' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory $td | Out-Null
    $inf = Join-Path $td 'x.inf'; [IO.File]::WriteAllText($inf, "[Version]`r`n; no hardware ids in here`r`n")
    $cab = Join-Path $td 'x.cab'; & makecab.exe /D RptFileName=nul /D InfFileName=nul $inf $cab | Out-Null
    $fs = [IO.File]::OpenRead($cab); try { $dig = [Convert]::ToBase64String([Security.Cryptography.SHA1]::Create().ComputeHash($fs)) } finally { $fs.Dispose() }
    $msItem = [pscustomobject]@{ Raw = [pscustomobject]@{ UpdateId = 'u'; Key = 'PCI\VEN_AAAA&DEV_BBBB'; Name = 'x' } }
    $okUrl = 'https://catalog.s.download.windowsupdate.com/x.cab'
    $r = Install-MsCatalogItem $msItem (Join-Path $td 'w1') ([pscustomobject]@{ Url = $okUrl; Digest = $dig }) $cab
    Check 'ms install: package without this hardware ID refused' ((-not $r.Success) -and $r.Message -match 'does not contain hardware')
    $r = Install-MsCatalogItem $msItem (Join-Path $td 'w2') ([pscustomobject]@{ Url = $okUrl; Digest = 'AAAAAAAAAAAAAAAAAAAAAAAAAAA=' }) $cab
    Check 'ms install: wrong SHA-1 digest refused' ((-not $r.Success) -and $r.Message -match 'SHA-1')
    $r = Install-MsCatalogItem $msItem (Join-Path $td 'w3') ([pscustomobject]@{ Url = 'https://evil.example.com/x.cab'; Digest = $dig }) $cab
    Check 'ms install: host outside the built-in list refused' ((-not $r.Success) -and $r.Message -match 'allowed list')
    Remove-Item $td -Recurse -Force -ErrorAction SilentlyContinue
    # 5b) vendor-direct (NVIDIA): version mapping, lookup with a fake server, signature/host checks
    Check 'nvidia version: 32.0.16.2114 is 621.14' ((ConvertTo-NvidiaVersion '32.0.16.2114') -eq 621.14)
    Check 'nvidia version: 32.0.15.7640 is 576.40' ((ConvertTo-NvidiaVersion '32.0.15.7640') -eq 576.40)
    $fakeGet = { param($u)
        if ($u -match 'TypeID=2') { return '<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 30 Series</Name><Value>120</Value></LookupValue><LookupValue><Name>GeForce RTX 30 Series (Notebooks)</Name><Value>123</Value></LookupValue></LookupValues></LookupValueSearch>' }
        if ($u -match 'ParentID=120') { return '<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 3060</Name><Value>933</Value></LookupValue><LookupValue><Name>GeForce RTX 3060 Ti</Name><Value>934</Value></LookupValue></LookupValues></LookupValueSearch>' }
        if ($u -match 'psid=120&pfid=933') { return '{ "Success":"1", "IDS":[{ "downloadInfo": { "Version":"700.01", "DownloadURL":"https://us.download.nvidia.com/Windows/700.01/x.exe", "DownloadURLFileSize":"1.04 GB", "ReleaseDateTime":"Wed Nov 04, 2026" } }] }' }
        throw "unexpected url $u" }
    $nvOld = @([pscustomobject]@{ InstanceId = 'x'; Name = 'NVIDIA GeForce RTX 3060'; Class = 'DISPLAY'; Maker = 'NVIDIA'; Version = '32.0.16.2114'; HardwareId = @() })
    $o1 = @(Find-NvidiaUpdates $nvOld ([pscustomobject]@{ Arch = 'x64'; Os = '11' }) $fakeGet)
    Check 'nvidia find: newer driver offered with the exact GPU model (not the Ti)' ($o1.Count -eq 1 -and $o1[0].Raw.Version -eq '700.01' -and $o1[0].SizeMB -eq 1065)
    $nvNew = @([pscustomobject]@{ InstanceId = 'x'; Name = 'NVIDIA GeForce RTX 3060'; Class = 'DISPLAY'; Maker = 'NVIDIA'; Version = '32.0.17.0001'; HardwareId = @() })
    Check 'nvidia find: same or newer installed driver is not offered' (@(Find-NvidiaUpdates $nvNew ([pscustomobject]@{ Arch = 'x64'; Os = '11' }) $fakeGet).Count -eq 0)
    Check 'nvidia find: laptop GPU is matched against the notebook series only' (@(Find-NvidiaUpdates @([pscustomobject]@{ InstanceId = 'x'; Name = 'NVIDIA GeForce RTX 3060 Laptop GPU'; Class = 'DISPLAY'; Maker = 'NVIDIA'; Version = '32.0.15.1000'; HardwareId = @() }) ([pscustomobject]@{ Arch = 'x64'; Os = '11' }) { param($u) if ($u -match 'TypeID=2') { '<LookupValueSearch><LookupValues><LookupValue><Name>GeForce RTX 30 Series</Name><Value>120</Value></LookupValue></LookupValues></LookupValueSearch>' } else { throw 'must not query the desktop series' } }).Count -eq 0)
    Check 'nvidia find: non-x64 PC gets no offer' (@(Find-NvidiaUpdates $nvOld ([pscustomobject]@{ Arch = 'arm64'; Os = '11' }) $fakeGet).Count -eq 0)
    $tv = Join-Path $env:TEMP ('ds_vendor_' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $tv | Out-Null
    $unsigned = Join-Path $tv 'setup.exe'; Set-Content $unsigned 'not a real installer'
    $r = Install-VendorItem ([pscustomobject]@{ Raw = [pscustomobject]@{ Url = 'https://us.download.nvidia.com/x/setup.exe'; Signer = 'NVIDIA Corporation'; SilentArgs = '-s'; Version = '1'; InstanceId = $null } }) (Join-Path $tv 'w1') $unsigned
    Check 'vendor install: unsigned file refused' ((-not $r.Success) -and $r.Message -match 'signature')
    $r = Install-VendorItem ([pscustomobject]@{ Raw = [pscustomobject]@{ Url = 'https://evil.example.com/setup.exe'; Signer = 'NVIDIA Corporation'; SilentArgs = '-s'; Version = '1'; InstanceId = $null } }) (Join-Path $tv 'w2') $unsigned
    Check 'vendor install: host outside the built-in list refused' ((-not $r.Success) -and $r.Message -match 'allowed list')
    $winSigned = Join-Path $env:windir 'System32\notepad.exe'
    $r = Install-VendorItem ([pscustomobject]@{ Raw = [pscustomobject]@{ Url = 'https://us.download.nvidia.com/x/notepad.exe'; Signer = 'NVIDIA Corporation'; SilentArgs = '/?'; Version = '1'; InstanceId = $null } }) (Join-Path $tv 'w3') $winSigned
    Check 'vendor install: valid signature from the wrong maker refused' ((-not $r.Success) -and $r.Message -match 'Signer does not match')
    Remove-Item $tv -Recurse -Force -ErrorAction SilentlyContinue
    # 5c) self-update: signed manifest, hash, safe apply with rollback
    $rsaK = New-Object Security.Cryptography.RSACryptoServiceProvider(2048); $pub = $rsaK.ToXmlString($false); $priv = $rsaK.ToXmlString($true)
    $tu = Join-Path $env:TEMP ('ds_upd_' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $tu | Out-Null
    $mkZip = { param($name, $files) $d = Join-Path $tu ($name + '_src'); New-Item -ItemType Directory $d | Out-Null; foreach ($k in $files.Keys) { Set-Content (Join-Path $d $k) $files[$k] -Encoding UTF8 }; $z = Join-Path $tu "$name.zip"; Compress-Archive -Path (Join-Path $d '*') -DestinationPath $z; $z }
    $goodFiles = @{ 'DriverScan.ps1' = '# new main'; 'DriverCore.ps1' = '# new core'; 'DriverUI.ps1' = '# new ui' }
    $zipGood = & $mkZip 'good' $goodFiles
    $mkMan = { param($ver, $zip, $key) $m = [pscustomobject]@{ version = $ver; url = 'https://github.com/o/r/releases/download/v/app.zip'; sha256 = (Get-FileHash $zip -Algorithm SHA256).Hash; signature = '' }; $m.signature = New-UpdateSignature $m $key; $m }
    $man = & $mkMan '2.0.0' $zipGood $priv
    $getMan = { param($u) ($man | ConvertTo-Json) }
    Check 'update: newer signed version is offered' ((Get-AppUpdate 'o/r' '1.0.0' $pub $getMan).version -eq '2.0.0')
    Check 'update: same or older version is not offered' ($null -eq (Get-AppUpdate 'o/r' '2.0.0' $pub $getMan))
    Check 'update: no repo configured means no check' ($null -eq (Get-AppUpdate '' '1.0.0' $pub $getMan))
    $evil = $man.PSObject.Copy(); $evil.sha256 = ('A' * 64)
    $threw = $false; try { [void](Get-AppUpdate 'o/r' '1.0.0' $pub { param($u) ($evil | ConvertTo-Json) }) } catch { $threw = $true }
    Check 'update: manifest changed after signing is refused' $threw
    $other = (New-Object Security.Cryptography.RSACryptoServiceProvider(2048)).ToXmlString($true)
    $threw = $false; try { [void](Get-AppUpdate 'o/r' '1.0.0' $pub { param($u) ((& $mkMan '3.0.0' $zipGood $other) | ConvertTo-Json) }) } catch { $threw = $true }
    Check 'update: manifest signed with a different key is refused' $threw
    $evilUrl = $man.PSObject.Copy(); $evilUrl.url = 'https://evil.example.com/app.zip'; $evilUrl.signature = New-UpdateSignature $evilUrl $priv
    $threw = $false; try { [void](Get-AppUpdate 'o/r' '1.0.0' $pub { param($u) ($evilUrl | ConvertTo-Json) }) } catch { $threw = $true }
    Check 'update: download host outside the allowed list is refused even when signed' $threw
    $inst = Join-Path $tu 'inst'; New-Item -ItemType Directory $inst | Out-Null
    foreach ($k in 'DriverScan.ps1', 'DriverCore.ps1', 'DriverUI.ps1') { Set-Content (Join-Path $inst $k) '# old' }
    Set-Content (Join-Path $inst 'driverdb.json') '{"mine":true}'
    $r = Install-AppUpdate $man $inst (Join-Path $tu 'w1') '1.0.0' $zipGood
    Check 'update apply: files replaced and old ones backed up' ($r.Success -and (Get-Content (Join-Path $inst 'DriverUI.ps1') -Raw) -match 'new ui' -and (Get-Content (Join-Path $inst '_backup\1.0.0\DriverUI.ps1') -Raw) -match 'old')
    $bad = $man.PSObject.Copy(); $bad.sha256 = ('B' * 64)
    $r = Install-AppUpdate $bad $inst (Join-Path $tu 'w2') '2.0.0' $zipGood
    Check 'update apply: wrong zip hash refused' ((-not $r.Success) -and $r.Message -match 'SHA-256')
    $zipExe = & $mkZip 'exe' ($goodFiles + @{ 'evil.exe' = 'MZ' })
    $r = Install-AppUpdate (& $mkMan '2.0.0' $zipExe $priv) $inst (Join-Path $tu 'w3') '2.0.0' $zipExe
    Check 'update apply: package carrying an .exe refused' ((-not $r.Success) -and $r.Message -match 'not allowed')
    $brokenFiles = $goodFiles.Clone(); $brokenFiles['DriverUI.ps1'] = 'function {{{ broken'
    $zipBroken = & $mkZip 'broken' $brokenFiles
    $before = Get-Content (Join-Path $inst 'DriverCore.ps1') -Raw
    $r = Install-AppUpdate (& $mkMan '2.0.1' $zipBroken $priv) $inst (Join-Path $tu 'w4') '2.0.0' $zipBroken
    Check 'update apply: script with a syntax error refused and nothing changed' ((-not $r.Success) -and $r.Message -match 'syntax' -and (Get-Content (Join-Path $inst 'DriverCore.ps1') -Raw) -eq $before)
    Check 'update apply: the user''s driverdb.json is never overwritten' ((Get-Content (Join-Path $inst 'driverdb.json') -Raw) -match 'mine')
    Remove-Item $tu -Recurse -Force -ErrorAction SilentlyContinue
    # 5d) shipped catalog: Intel graphics entries match the right GPUs only
    $shipped = @(Read-DriverCatalog (Join-Path $PSScriptRoot 'driverdb.json'))
    $gpu = { param($n, $id, $v) @([pscustomobject]@{ InstanceId = ''; Name = $n; Class = 'DISPLAY'; Maker = 'Intel Corporation'; Version = $v; HardwareId = @($id) }) }
    $offerNames = { param($devs) (@(Find-CatalogUpdates $devs $shipped ([pscustomobject]@{ Arch = 'x64'; Os = '11' })) | ForEach-Object { $_.Raw.name }) -join ',' }
    Check 'catalog: Arc A770 with an old driver gets the Arc package' ((& $offerNames (& $gpu 'Intel(R) Arc(TM) A770 Graphics' 'PCI\VEN_8086&DEV_56A0&SUBSYS_00000000&REV_08' '31.0.101.4000')) -eq 'Intel Arc Graphics Driver')
    Check 'catalog: UHD Graphics 770 (14th gen) gets the 11th-14th gen package' ((& $offerNames (& $gpu 'Intel(R) UHD Graphics 770' 'PCI\VEN_8086&DEV_4680&SUBSYS_00000000&REV_0C' '31.0.101.4000')) -eq 'Intel Graphics Driver (11th-14th Gen)')
    Check 'catalog: 10th gen UHD 630 (legacy) gets no offer' ((& $offerNames (& $gpu 'Intel(R) UHD Graphics 630' 'PCI\VEN_8086&DEV_9BC8&SUBSYS_00000000&REV_05' '27.20.100.8000')) -eq '')
    Check 'catalog: an up-to-date Intel graphics driver gets no offer' ((& $offerNames (& $gpu 'Intel(R) Arc(TM) A770 Graphics' 'PCI\VEN_8086&DEV_56A0&SUBSYS_00000000&REV_08' '32.0.101.9175')) -eq '')
    Check 'catalog: every shipped installer entry has https url, sha256, signer and silent switches' (@($shipped | Where-Object { $_.type -eq 'installer' -and ($_.url -notmatch '^https://' -or $_.sha256 -notmatch '^[0-9A-F]{64}$' -or -not $_.signer -or -not $_.silentArgs) }).Count -eq 0)
    # 5e) signed online catalog + publisher mirror
    $rk = New-Object Security.Cryptography.RSACryptoServiceProvider(2048); $rpub = $rk.ToXmlString($false); $rpriv = $rk.ToXmlString($true)
    $tc = Join-Path $env:TEMP ('ds_cat_' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $tc | Out-Null
    $goodEntry = [ordered]@{ name = 'Mirror Test Driver'; version = '2.0.0.0'; match = @('PCI\VEN_1234&DEV_*'); type = 'installer'; url = 'https://github.com/kungzazakub-png/tiers-driver-manage/releases/download/drivers/x.exe'; sha256 = ('A' * 64); signer = 'Test'; silentArgs = '/s'; class = 'NET' }
    $mkCat = { param($rev, $entries, $key) $json = ([pscustomobject]@{ version = 1; drivers = @($entries) } | ConvertTo-Json -Depth 6); $b = (New-Object Text.UTF8Encoding($false)).GetBytes($json)
        $m = [pscustomobject]@{ version = "1.$rev"; url = 'https://github.com/o/r/releases/download/catalog/driverdb.json'; sha256 = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($b)).Replace('-', ''); signature = '' }
        $m.signature = New-UpdateSignature $m $key
        @{ Manifest = (New-Object Text.UTF8Encoding($false)).GetBytes(($m | ConvertTo-Json)); File = $b } }
    $fetchFor = { param($c) { param($u) if ($u -like '*catalog.json') { ,$c.Manifest } else { ,$c.File } }.GetNewClosure() }
    $cache = Join-Path $tc 'online.json'
    $c1 = & $mkCat 1 @($goodEntry) $rpriv
    $r = Get-OnlineCatalog 'o/r' $rpub $cache (& $fetchFor $c1)
    Check 'online catalog: signed revision 1 is saved to the cache' ($r.Updated -and $r.Revision -eq 1 -and (Test-Path $cache))
    $r = Get-OnlineCatalog 'o/r' $rpub $cache (& $fetchFor $c1)
    Check 'online catalog: same revision is not downloaded again' (-not $r.Updated)
    $c2 = & $mkCat 2 @($goodEntry) (New-Object Security.Cryptography.RSACryptoServiceProvider(2048)).ToXmlString($true)
    $threw = $false; try { [void](Get-OnlineCatalog 'o/r' $rpub $cache (& $fetchFor $c2)) } catch { $threw = $true }
    Check 'online catalog: signed with another key is refused and the cache stays' ($threw -and (Get-Content $cache -Raw) -match '"revision":\s*1')
    $c3 = & $mkCat 3 @($goodEntry) $rpriv; $c3.File[5] = [byte]($c3.File[5] -bxor 1)
    $threw = $false; try { [void](Get-OnlineCatalog 'o/r' $rpub $cache (& $fetchFor $c3)) } catch { $threw = $true }
    Check 'online catalog: file changed after signing is refused' $threw
    $badEntry = [ordered]@{} + $goodEntry; $badEntry['url'] = 'https://evil.example.com/x.exe'
    $threw = $false; try { [void](Get-OnlineCatalog 'o/r' $rpub $cache (& $fetchFor (& $mkCat 4 @($goodEntry, $badEntry) $rpriv))) } catch { $threw = $true }
    Check 'online catalog: an entry pointing outside the allowed hosts rejects the whole catalog' ($threw -and (Get-Content $cache -Raw) -match '"revision":\s*1')
    $threw = $false; $noHash = [ordered]@{} + $goodEntry; $noHash['sha256'] = ''
    try { [void](Get-OnlineCatalog 'o/r' $rpub $cache (& $fetchFor (& $mkCat 5 @($noHash) $rpriv))) } catch { $threw = $true }
    Check 'online catalog: an entry without sha256 rejects the whole catalog' $threw
    $localCat = Join-Path $tc 'local.json'; Set-Content $localCat '{"version":1,"drivers":[{"name":"Mirror Test Driver","version":"1.0.0.0","match":["x"],"type":"installer","url":"https://downloadmirror.intel.com/1/x.exe","sha256":"B","signer":"Intel","silentArgs":"/s"},{"name":"Mine Only","version":"1.0.0.0","match":["y"],"type":"installer","url":"https://downloadmirror.intel.com/1/y.exe","sha256":"B","signer":"Intel","silentArgs":"/s"}]}'
    $all = @(Read-AllCatalogs $localCat $cache)
    Check 'online catalog: online entry replaces the same-named local one, local-only entries stay' ($all.Count -eq 2 -and (@($all | Where-Object name -eq 'Mirror Test Driver')[0].version -eq '2.0.0.0'))
    Check 'mirror: the publisher''s own release URL is allowed' (-not $(try { Assert-AllowedUrl 'https://github.com/kungzazakub-png/tiers-driver-manage/releases/download/drivers/x.exe'; $false } catch { $true }))
    Check 'mirror: another GitHub repository is NOT allowed' $(try { Assert-AllowedUrl 'https://github.com/someone/else/releases/download/v1/x.exe'; $false } catch { $true })
    Remove-Item $tc -Recurse -Force -ErrorAction SilentlyContinue
    # 5f) update apply must work when %TEMP% is an 8.3 short path (a real PC failed with "folder inside the update" because of this)
    $tl = Join-Path $env:TEMP ('ds_long_directory_name_' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $tl | Out-Null
    $shortRoot = (New-Object -ComObject Scripting.FileSystemObject).GetFolder($tl).ShortPath
    $zipS = Join-Path $tl 's.zip'; $srcS = Join-Path $tl 'src'; New-Item -ItemType Directory $srcS | Out-Null
    foreach ($k in 'DriverScan.ps1', 'DriverCore.ps1', 'DriverUI.ps1') { Set-Content (Join-Path $srcS $k) '# new' }
    Compress-Archive -Path (Join-Path $srcS '*') -DestinationPath $zipS
    $instS = Join-Path $tl 'inst'; New-Item -ItemType Directory $instS | Out-Null
    $manS = [pscustomobject]@{ version = '9.0.0'; url = 'https://github.com/o/r/releases/download/v/a.zip'; sha256 = (Get-FileHash $zipS -Algorithm SHA256).Hash }
    $rs = Install-AppUpdate $manS $instS (Join-Path $shortRoot 'work') '1.0.0' $zipS
    Check ("update apply: works with a short (8.3) work path [$($shortRoot -ne $tl)]: " + $rs.Message) $rs.Success
    Remove-Item $tl -Recurse -Force -ErrorAction SilentlyContinue
    # 5g) devices without a driver: searched like installed ones, and listed when nothing is found
    $ents = @([pscustomobject]@{ PNPDeviceID = 'USB\VID_03F0&PID_5617\X'; Name = 'Unknown USB Printer'; PNPClass = $null; Manufacturer = $null; ConfigManagerErrorCode = 28; HardwareID = @('USB\VID_03F0&PID_5617&REV_0100', 'USB\VID_03F0&PID_5617'); CompatibleID = @('USB\Class_07') },
              [pscustomobject]@{ PNPDeviceID = 'ACPI\XYZ0001\1'; Name = 'Mystery device'; PNPClass = $null; Manufacturer = $null; ConfigManagerErrorCode = 28; HardwareID = @('ACPI\XYZ0001'); CompatibleID = @() })
    $noDrv = @(Get-DevicesWithoutDriver $ents)
    Check 'no-driver: both devices carry their hardware and compatible IDs, without a version' ($noDrv.Count -eq 2 -and $noDrv[0].HardwareId.Count -eq 3 -and $null -eq $noDrv[0].Version)
    $fakeRows = { param($k) @([pscustomobject]@{ UpdateId = '11111111-1111-1111-1111-111111111111'; Title = 'HP printer driver'; Products = 'Windows 11'; Class = 'Drivers (Printers)'; Date = [datetime]'2026-01-01'; Version = '1.2.3.4'; SizeMB = 5 }) }
    $offersND = @(Find-MsCatalogUpdates $noDrv ([pscustomobject]@{ Arch = 'x64'; Os = '11' }) 40 { param($k) if ($k -like 'USB*') { & $fakeRows $k } else { @() } })
    Check 'no-driver: a device with no driver at all is offered the catalog driver' ($offersND.Count -eq 1 -and $offersND[0].Title -match 'no driver yet' -and $offersND[0].Raw.Name -eq 'Unknown USB Printer')
    $unres = @(Get-UnresolvedDevices $noDrv $offersND)
    Check 'no-driver: only the device nothing matched is reported as unresolved' ($unres.Count -eq 1 -and $unres[0].Name -eq 'Mystery device')
    # 5h) driver backup and roll back (pnputil is faked; the real export was run separately on a real device)
    $tb = Join-Path $env:TEMP ('ds_bk_' + [guid]::NewGuid().ToString('N')); New-Item -ItemType Directory $tb | Out-Null
    $calls = [System.Collections.ArrayList]@()
    $fakeRun = { param($a) [void]$calls.Add(($a -join ' ')); if ($a[0] -eq '/export-driver') { Set-Content (Join-Path $a[2] 'old.inf') ';old'; Set-Content (Join-Path $a[2] 'old.sys') 'x' }; $null = cmd /c 'exit 0'; '' }.GetNewClosure()
    $e1 = Backup-DeviceDriver 'USB\VID_1&PID_2\X' 'Test Bluetooth' (Join-Path $tb 'set1') 1 ([pscustomobject]@{ Inf = 'oem34.inf'; Version = '1.0.0.0' }) $fakeRun
    Check 'backup: a third-party driver is exported with pnputil and recorded' ($e1.Folder -eq '1' -and $e1.OldInf -eq 'oem34.inf' -and ($calls -join ';') -match '/export-driver oem34.inf' -and (Test-Path (Join-Path $tb 'set1\1\old.inf')))
    $e2 = Backup-DeviceDriver 'PCI\X' 'Built-in device' (Join-Path $tb 'set1') 2 ([pscustomobject]@{ Inf = 'netrtl64.inf'; Version = '10.0' }) $fakeRun
    Check 'backup: a built-in driver is recorded without files' ($e2.Folder -eq '' -and $e2.OldInf -eq 'netrtl64.inf')
    Check 'backup: a device with no driver gets no entry' ($null -eq (Backup-DeviceDriver 'PCI\Y' 'No driver' (Join-Path $tb 'set1') 3 ([pscustomobject]@{ Inf = ''; Version = '' }) $fakeRun))
    Save-BackupManifest (Join-Path $tb 'set1') @($e1, $e2)
    $listed = @(Get-DriverBackups $tb)
    Check 'backup: saved backups are listed with their folder' ($listed.Count -eq 2 -and $listed[0].SetDir -like '*set1')
    $calls.Clear()
    $rr = Restore-DeviceDriver ($listed | Where-Object Device -eq 'Test Bluetooth') ([pscustomobject]@{ Inf = 'oem99.inf'; Version = '2.0.0.0' }) $fakeRun
    Check 'rollback: old package is added, then the new one removed' ($rr.Success -and $calls.Count -eq 2 -and $calls[0] -match '^/add-driver .*\*\.inf /subdirs /install$' -and $calls[1] -eq '/delete-driver oem99.inf /uninstall /force')
    $calls.Clear()
    $rr = Restore-DeviceDriver ($listed | Where-Object Device -eq 'Test Bluetooth') ([pscustomobject]@{ Inf = 'oem34.inf'; Version = '1.0.0.0' }) $fakeRun
    Check 'rollback: nothing is touched when the device already uses the saved driver' ($rr.Success -and $calls.Count -eq 0)
    $rr = Restore-DeviceDriver ($listed | Where-Object Device -eq 'Test Bluetooth') ([pscustomobject]@{ Inf = 'netrtl64.inf'; Version = '9' }) $fakeRun
    Check 'rollback: a built-in current driver is never deleted' ((-not $rr.Success) -and $rr.Message -match 'built into Windows')
    $failRun = { param($a) $null = cmd /c 'exit 5'; 'access denied' }
    $rr = Restore-DeviceDriver ($listed | Where-Object Device -eq 'Test Bluetooth') ([pscustomobject]@{ Inf = 'oem99.inf'; Version = '2.0.0.0' }) $failRun
    Check 'rollback: a pnputil failure is reported' ((-not $rr.Success) -and $rr.Message -match 'access denied')
    Remove-Item $tb -Recurse -Force -ErrorAction SilentlyContinue
    # 6) full GUI flow in DryRun: scan -> review -> install -> results
    [Windows.Forms.Application]::EnableVisualStyles()
    $script:DryRun = $true
    $script:MaxPasses = 1
    $f = New-MainForm
    $f.Show()
    function Wait-Idle($sec) { $d = (Get-Date).AddSeconds($sec); while ((Get-Date) -lt $d -and $script:isBusy) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 }; [Windows.Forms.Application]::DoEvents() }
    $script:btnScan.PerformClick()
    Wait-Idle 240
    Check 'gui: scan finished' (-not $script:isBusy)
    Check "gui: installed-drivers tab filled ($($script:invGrid.Rows.Count) rows)" ($script:invGrid.Rows.Count -gt 0)
    Check 'gui: review mode after scan' ($script:gridMode -eq 'review')
    "status: $($script:lblStatus.Text)"
    # synthetic list: two ordinary rows + one system row (not ticked), then install the ticked ones
    $script:grid.Rows.Clear(); $script:ctx.ItemsById = @{}; $script:ctx.Results = @(); $script:ctx.RestoreDone = $false
    foreach ($x in @(@('ms:1', 'Realtek Audio', 'MEDIA', 12.5), @('ms:2', 'Intel Wi-Fi 6E', 'NET', 40), @('ms:3', 'Intel Management Engine', 'SYSTEM', 0.4))) {
        $it = [pscustomobject]@{ Id = $x[0]; Title = $x[1]; Class = $x[2]; Source = 'Microsoft Update Catalog'; Date = '2025-09-01'; SizeMB = $x[3] }
        $script:ctx.ItemsById[$it.Id] = $it; Add-UpdateRow $it
    }
    Enter-ReviewMode 0
    Check 'gui: system driver is not ticked by default' ($script:grid.Rows[2].Cells[0].Value -eq $false -and $script:grid.Rows[0].Cells[0].Value -eq $true)
    Check 'gui: install button counts ticked rows' ($script:btnInstall.Enabled -and $script:btnInstall.Text -match '2')
    $script:btnInstall.PerformClick()
    Wait-Idle 20
    Check 'gui: install finished into results mode' ($script:gridMode -eq 'results' -and $script:grid.Rows.Count -eq 2)
    Check 'gui: results kept as ok' (@($script:ctx.Results | Where-Object { $_.Status -eq 'ok' }).Count -eq 2)
    Check 'gui: banner names the install count once, status line is short' (($script:lblBanner.Text -match '2') -and $script:lblStatus.Text -eq 'Done')
    Check 'gui: all-ok ending offers restart and scan again' ((@($script:bannerBtns | Where-Object Visible | ForEach-Object Name) -join ',') -eq 'restart,rescan')
    Check 'gui: summary chip reflects the run, not the old scan' ($script:sumUpdates.Text -match '2')
    Check 'gui: progress bar hidden when idle' (-not $script:progress.Visible)
    Check 'gui: header logo and window icon are loaded' ((@($f.Controls | ForEach-Object { $_.Controls } | Where-Object { $_ -is [Windows.Forms.PictureBox] -and $_.Image }).Count -ge 1) -and $null -ne $f.Icon)
    [Windows.Forms.Application]::DoEvents(); $f.PerformLayout()   # Top values are only final after layout
    $tops = @($f.Controls | Sort-Object TabIndex | ForEach-Object { $_.Top })
    Check 'gui: tab order follows the visual order top to bottom' (($tops -join ',') -eq (($tops | Sort-Object) -join ','))
    Check 'plain reason: SHA failure is explained in plain English' ((Get-PlainReason 'SHA-256 mismatch (got X)') -match 'incomplete')
    Check 'plain reason: unknown text is kept' ((Get-PlainReason 'xyz') -eq 'xyz')
    Check 'gui: unticked driver is not offered again' ($script:ctx.Declined -contains 'ms:3')
    Check 'gui: dates before 1990 show a dash' ((Format-DateText '1968-07-18') -eq '—')
    # failure path: banner offers retry, retry reinstalls only the failed one
    $script:ctx.Results = @([pscustomobject]@{ Id = 'ms:1'; Title = 'Realtek Audio'; Class = 'MEDIA'; Source = 'x'; Date = ''; SizeMB = 1; Status = 'fail'; Reason = 'test' }, [pscustomobject]@{ Id = 'ms:2'; Title = 'Intel Wi-Fi'; Class = 'NET'; Source = 'x'; Date = ''; SizeMB = 1; Status = 'ok'; Reason = '' })
    $script:ctx.NeedReboot = $true
    Complete-Run
    Check 'gui: banner shows restart + retry buttons' (($script:bannerBtns | Where-Object Visible | Measure-Object).Count -eq 3)
    Invoke-BannerClick 'retry'
    Wait-Idle 20
    Check 'gui: retry reinstalled the failed driver only' ((@($script:ctx.Results | Where-Object { $_.Status -eq 'ok' }).Count -eq 2) -and $script:grid.Rows.Count -eq 2)
    # info rows (device with no driver and no match): visible, never counted, never ticked
    $script:grid.Rows.Clear(); $script:gridMode = 'review'; $script:ctx.ItemsById = @{}
    Add-InfoRow ([pscustomobject]@{ InstanceId = 'ACPI\XYZ0001\1'; Name = 'Mystery device'; Class = ''; HardwareId = @('ACPI\XYZ0001') })
    Enter-ReviewMode 0
    Check 'gui: a list with only unresolved devices is shown but offers no install' ($script:grid.Visible -and -not $script:btnInstall.Visible -and -not $script:chkAll.Visible)
    $itX = [pscustomobject]@{ Id = 'ms:9'; Title = 'Some driver'; Class = 'NET'; Source = 'Microsoft Catalog'; Date = ''; SizeMB = 1 }
    $script:ctx.ItemsById[$itX.Id] = $itX; Add-UpdateRow $itX; Enter-ReviewMode 0
    Check 'gui: unresolved rows are not counted in the selection' ($script:lblSel.Text -match '1 of 1')
    $f.Close(); $f.Dispose()
    "FAILED CHECKS: $fail"
    exit $fail
}

[Windows.Forms.Application]::EnableVisualStyles()
# Started hidden (wscript launcher), the first window shown would inherit SW_HIDE and stay invisible: spend that on a 1px invisible form.
$d0 = New-Object Windows.Forms.Form; $d0.FormBorderStyle = 'None'; $d0.ShowInTaskbar = $false; $d0.Opacity = 0; $d0.Size = New-Object Drawing.Size(1, 1); $d0.Show(); $d0.Close()
[void](New-MainForm).ShowDialog()
