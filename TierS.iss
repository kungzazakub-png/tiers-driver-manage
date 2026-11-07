; TierS Driver Manage - Inno Setup script. Built by Build-Setup.ps1 (pass /DAppVersion=x.y.z).
#define AppName "TierS Driver Manage"
#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif

[Setup]
AppId={{B6F1C0D2-5E0A-4B7B-9E2C-7A3D1F4C8E21}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=TierS
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
SetupIconFile=DriverScan\TierS.ico
UninstallDisplayIcon={app}\TierS.ico
UninstallDisplayName={#AppName}
OutputDir=.
OutputBaseFilename=TierS-Driver-Manage-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "DriverScan\TierS Driver Manage.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\DriverScan.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\DriverCore.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\DriverUI.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\New-CatalogEntry.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\README.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\TierS.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\tiers-logo.png"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\UpdatePublicKey.xml"; DestDir: "{app}"; Flags: ignoreversion
; the user's own catalog is kept when it is newer
Source: "DriverScan\driverdb.json"; DestDir: "{app}"; Flags: comparetimestamp
Source: "DriverScan\driverdb.example.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "DriverScan\update.json"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\TierS Driver Manage.exe"; IconFilename: "{app}\TierS.ico"; Comment: "Scan and install drivers"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\TierS Driver Manage.exe"; IconFilename: "{app}\TierS.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\TierS Driver Manage.exe"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent runascurrentuser

[InstallDelete]
; leftovers from the earlier one-file installer / pre-rename versions
Type: files; Name: "{autodesktop}\DriverScan.lnk"
Type: files; Name: "{autoprograms}\DriverScan.lnk"
Type: files; Name: "{userdesktop}\{#AppName}.lnk"
Type: files; Name: "{userprograms}\{#AppName}.lnk"
Type: files; Name: "{app}\TierS.vbs"
Type: files; Name: "{app}\DriverScan.cmd"
Type: files; Name: "{app}\Setup-Run.exe"

[Registry]
; the Apps entry written by the earlier installer is replaced by Inno's own
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\TierSDriverManage"; ValueType: none; Flags: deletekey

[UninstallDelete]
Type: filesandordirs; Name: "{app}\_backup"
Type: filesandordirs; Name: "{commonappdata}\{#AppName}"
