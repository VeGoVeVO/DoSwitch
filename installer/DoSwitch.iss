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
; runs it at the START of the next launch; that throwaway instance then exits
; so the exe can be replaced, and CloseApplications lets the Restart Manager
; close any handle still open on it. The relaunch is the [Run] entry below
; (no skipifsilent), not RestartApplications, which proved unreliable.
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

[Registry]
; The language the player picked, for the app to read at startup: the in-app
; switcher was replaced by the auto-update toggle, so language is chosen here
; now. {language} is the selected [Languages] Name (en/fr/es), which matches
; the app's Lang codes exactly. On a /VERYSILENT self-update Inno reuses the
; previously selected language, so this preserves it rather than resetting.
; Left in place on uninstall (no uninsdeletevalue): the Pro app shares this
; key, and settings.json keeps the language regardless.
Root: HKCU; Subkey: "Software\DoSwitch"; ValueType: string; ValueName: "Language"; ValueData: "{language}"

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

; No skipifsilent: the silent self-update runs this too, which is how the app
; comes back after a staged update applies on startup. Interactive first
; install shows it as the "launch now" box; a /VERYSILENT update relaunches
; the freshly-installed app.
[Run]
Filename: "{app}\DoSwitch.exe"; Description: "{cm:LaunchProgram,DoSwitch}"; Flags: nowait postinstall
