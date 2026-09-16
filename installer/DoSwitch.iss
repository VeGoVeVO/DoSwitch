; DoSwitch (free) installer.
;
; Per-user, no admin - the app needs nothing installed and no elevation. It
; lays down the exe and its shortcuts under the user's profile. The same
; installer is what the app's silent self-update runs, which is why it can
; close and relaunch the running app (CloseApplications / RestartApplications).
;
; Built by CI (.github/workflows/publish-free.yml), which passes the version
; with /DAppVersion=.

#ifndef AppVersion
  #define AppVersion "0.0.0.0"
#endif

[Setup]
AppId={{2C1B9E55-7D3A-4F81-9E22-8B4A1F0C7E92}}
AppName=DoSwitch
AppVersion={#AppVersion}
AppPublisher=VeGoVeVO
AppPublisherURL=https://doswitchpro.com
DefaultDirName={localappdata}\Programs\DoSwitch
DefaultGroupName=DoSwitch
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
; For the silent self-update. The app stages the installer while it runs and
; launches it only as it quits, so the exe is already being freed - but the
; process may not be fully gone the instant the installer starts copying, so
; CloseApplications lets the Restart Manager mop up any handle still open.
; RestartApplications is OFF: the update is left in place for the next launch
; rather than relaunched, which is the seamless, never-interrupt behaviour.
CloseApplications=yes
RestartApplications=no
OutputDir=dist
OutputBaseFilename=DoSwitch-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\DoSwitch.exe
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startup"; Description: "Start DoSwitch when Windows starts"; GroupDescription: "Startup:"; Flags: unchecked

[Files]
Source: "DoSwitch.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\DoSwitch"; Filename: "{app}\DoSwitch.exe"
Name: "{group}\{cm:UninstallProgram,DoSwitch}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\DoSwitch"; Filename: "{app}\DoSwitch.exe"; Tasks: desktopicon
Name: "{userstartup}\DoSwitch"; Filename: "{app}\DoSwitch.exe"; Tasks: startup

[Run]
Filename: "{app}\DoSwitch.exe"; Description: "{cm:LaunchProgram,DoSwitch}"; Flags: nowait postinstall skipifsilent
