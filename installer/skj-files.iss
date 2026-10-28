; SKJ Files — Windows installer (Inno Setup 6)
;
; Build it with scripts/package-windows.ps1, which passes the version from Cargo.toml:
;   iscc /DAppVersion=0.4.0 /DExePath=..\target\release\skj-files.exe installer\skj-files.iss
;
; Per-user install (no admin prompt): it goes under %LOCALAPPDATA%, which is also what lets the
; app's own "default file manager" setting work, since that writes per-user registry keys.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef ExePath
  #define ExePath "..\target\release\skj-files.exe"
#endif
; passed in from Cargo.toml's `repository` by scripts/package-windows.ps1, so moving the
; project never means editing this file
#ifndef AppUrl
  #define AppUrl "https://github.com/jakub362/skj-files"
#endif

#define AppName "SKJ Files"
#define AppPublisher "SKJ Tech"
#define ExeName "skj-files.exe"

[Setup]
AppId={{8F3B1C27-5E4A-4D19-9C62-7A1E0B5D83F4}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=SKJ-Files-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\skj-files.ico
UninstallDisplayIcon={app}\{#ExeName}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Branding, drawn by scripts/make-installer-art.py from assets/skj-files.png.
; Several sizes so it stays sharp at 125/150/200% display scaling.
WizardImageFile=art\wizard.bmp,art\wizard-2.bmp,art\wizard-3.bmp,art\wizard-4.bmp
WizardSmallImageFile=art\wizard-small.bmp,art\wizard-small-2.bmp,art\wizard-small-3.bmp,art\wizard-small-4.bmp
WizardImageStretch=yes
; modern style hides the welcome page by default, but that is the page the artwork lives on
DisableWelcomePage=no
DisableReadyPage=yes
ShowLanguageDialog=auto
AppMutex=SKJFilesSingleInstance
; x64 only, matching the binary we ship
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; per-user: no admin prompt, and it matches the per-user registry the app writes
PrivilegesRequired=lowest
; the app must not be running while its .exe is replaced
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "pl"; MessagesFile: "compiler:Languages\Polish.isl"

[CustomMessages]
en.OpenWithSKJ=Add "Open with SKJ Files" to the folder right-click menu
pl.OpenWithSKJ=Dodaj „Otwórz w SKJ Files” do menu prawego przycisku myszy na folderach
en.LaunchApp=Start {#AppName}
pl.LaunchApp=Uruchom {#AppName}
en.DesktopIcon=Create a desktop shortcut
pl.DesktopIcon=Utwórz skrót na pulpicie
en.KeepSettings=Your settings and tabs are kept in %APPDATA%\skj-files and are not removed.
pl.KeepSettings=Twoje ustawienia i karty pozostają w %APPDATA%\skj-files i nie są usuwane.

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "foldermenu"; Description: "{cm:OpenWithSKJ}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#ExeName}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#ExeName}"; Tasks: desktopicon

[Registry]
; "Open with SKJ Files" on folders, on the folder background, and on drives.
; This is an extra right-click entry, NOT the default action - double-clicking a folder still
; opens File Explorer. Making SKJ Files the default is a separate, experimental setting in the app.
Root: HKCU; Subkey: "Software\Classes\Directory\shell\skjfilesopen"; ValueType: string; ValueName: ""; ValueData: "Open with {#AppName}"; Tasks: foldermenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\skjfilesopen"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#ExeName}"; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\shell\skjfilesopen\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#ExeName}"" ""%1"""; Tasks: foldermenu; Flags: uninsdeletekey

Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\skjfilesopen"; ValueType: string; ValueName: ""; ValueData: "Open with {#AppName}"; Tasks: foldermenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\skjfilesopen"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#ExeName}"; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\skjfilesopen\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#ExeName}"" ""%V"""; Tasks: foldermenu; Flags: uninsdeletekey

Root: HKCU; Subkey: "Software\Classes\Drive\shell\skjfilesopen"; ValueType: string; ValueName: ""; ValueData: "Open with {#AppName}"; Tasks: foldermenu; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Drive\shell\skjfilesopen"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#ExeName}"; Tasks: foldermenu
Root: HKCU; Subkey: "Software\Classes\Drive\shell\skjfilesopen\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#ExeName}"" ""%1"""; Tasks: foldermenu; Flags: uninsdeletekey

; so the app can find where it was installed (used by the update check's "Get" button)
Root: HKCU; Subkey: "Software\{#AppPublisher}\{#AppName}"; ValueType: string; ValueName: "InstallDir"; ValueData: "{app}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\{#AppPublisher}\{#AppName}"; ValueType: string; ValueName: "Version"; ValueData: "{#AppVersion}"

[Run]
Filename: "{app}\{#ExeName}"; Description: "{cm:LaunchApp}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; nothing: settings in %APPDATA%\skj-files are the user's and are deliberately left alone
