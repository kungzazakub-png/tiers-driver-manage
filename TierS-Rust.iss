; TierS Driver Manage 2.x (Rust) installer. Same AppId as 1.x, so it upgrades the PowerShell version in place.
#define AppName "TierS Driver Manage"
#ifndef AppVersion
  #define AppVersion "2.0.0"
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
SetupIconFile=app\assets\TierS.ico
UninstallDisplayIcon={app}\tiers-driver-manage.exe
UninstallDisplayName={#AppName}
OutputDir=dist
OutputBaseFilename=TierS-Driver-Manage-Setup-{#AppVersion}
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
Source: "app\target\release\tiers-driver-manage.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\tiers-driver-manage.exe"; Comment: "Scan and install drivers"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\tiers-driver-manage.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\tiers-driver-manage.exe"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent runascurrentuser

[UninstallRun]
; removes the "start with Windows" task if the user turned it on (harmless when it does not exist)
Filename: "{sys}\schtasks.exe"; Parameters: "/Delete /TN ""TierS Driver Manage"" /F"; Flags: runhidden; RunOnceId: "RemoveAutostartTask"

[InstallDelete]
; the 1.x PowerShell version's files
Type: files; Name: "{app}\TierS Driver Manage.exe"
Type: files; Name: "{app}\*.ps1"
Type: files; Name: "{app}\update.json"
Type: files; Name: "{app}\UpdatePublicKey.xml"
Type: files; Name: "{app}\driverdb*.json"
Type: files; Name: "{app}\tiers-logo.png"
Type: files; Name: "{app}\TierS.ico"
Type: files; Name: "{app}\README.txt"

[UninstallDelete]
Type: filesandordirs; Name: "{commonappdata}\{#AppName}"
