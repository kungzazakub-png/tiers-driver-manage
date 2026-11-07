# Builds the Setup wizard: C:\1push\TierS-Driver-Manage-Setup.exe (Inno Setup). Usage: .\Build-Setup.ps1 [-Version 1.0.1]
param([string]$Version)
$src = Join-Path $PSScriptRoot 'DriverScan'
if (-not $Version) { $Version = [regex]::Match((Get-Content (Join-Path $src 'DriverScan.ps1') -Raw), "AppVersion = '([\d\.]+)'").Groups[1].Value }

# Native launcher (own process name, icon, UAC shield): compiled with the C# compiler that ships with Windows.
$csc = "$env:windir\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$sma = Get-ChildItem "$env:windir\Microsoft.NET\assembly\GAC_MSIL\System.Management.Automation" -Recurse -Filter System.Management.Automation.dll | Select-Object -First 1 -ExpandProperty FullName
& $csc /nologo /target:winexe /platform:x64 "/reference:$sma" /reference:System.Windows.Forms.dll "/win32icon:$src\TierS.ico" "/win32manifest:$src\Launcher.manifest" "/out:$src\TierS Driver Manage.exe" "$src\Launcher.cs" | Out-Null
if (-not (Test-Path "$src\TierS Driver Manage.exe")) { throw 'launcher did not compile' }

$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup not found (winget install JRSoftware.InnoSetup)' }
Push-Location $PSScriptRoot
try { & $iscc /Q "/DAppVersion=$Version" (Join-Path $PSScriptRoot 'TierS.iss') } finally { Pop-Location }
$out = Join-Path $PSScriptRoot 'TierS-Driver-Manage-Setup.exe'
if ((Get-Item $out).LastWriteTime -lt (Get-Date).AddMinutes(-2)) { throw 'Setup was not rebuilt' }
"built $out ($([int]((Get-Item $out).Length / 1KB)) KB) version $Version"
