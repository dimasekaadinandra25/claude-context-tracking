#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

#define AppName "Claude Usage Monitor"
#define AppExe "claude-usage-monitor.exe"

[Setup]
AppId={{A02EDB01-0246-490B-889D-15AD58B06E58}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Dimas Eka Adinandra
AppPublisherURL=https://github.com/dimasekaadinandra25/claude-context-tracking
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#AppExe}
OutputDir=..\target\installer
OutputBaseFilename=claude-usage-monitor-setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=force

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "ClaudeUsageMonitor"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch now"; Flags: nowait postinstall skipifsilent
