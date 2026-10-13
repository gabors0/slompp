; The Windows installer, built with Inno Setup 6.3 or newer from the
; release build:
;
;   cargo build --release -p slopp -p slopp-core
;   iscc crates\slopp\assets\windows\slopp.iss
;
; It writes target\release\slopp-<version>-windows-x86_64-setup.exe. The
; version comes from slopp.exe's version resource (build.rs); /DAppVersion=…
; overrides it, /O<dir> the output folder.
;
; Installs for the current user (%LOCALAPPDATA%\Programs\SLOPP, no UAC
; prompt) unless the user picks all users. Uninstalling stops the servers
; but keeps the stack (%LOCALAPPDATA%\slopp: sites, databases, configs) and
; the settings.

#define Root "..\..\..\.."
#define Bin Root + "\target\release"
#ifndef AppVersion
  #define AppVersion GetStringFileInfo(Bin + "\slopp.exe", "ProductVersion")
#endif

[Setup]
; Identifies the installation for upgrades and uninstalling: never change it.
AppId={{21EC18CB-2262-45D5-848B-E315EB0C19B2}
AppName=SLOPP
AppVersion={#AppVersion}
AppPublisher=SLOPP
AppPublisherURL=https://github.com/gabors0/slopp
AppSupportURL=https://github.com/gabors0/slopp/issues
AppUpdatesURL=https://github.com/gabors0/slopp/releases
DefaultDirName={autopf}\SLOPP
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#Bin}
OutputBaseFilename=slopp-{#AppVersion}-windows-x86_64-setup
SetupIconFile=slopp.ico
UninstallDisplayIcon={app}\slopp.exe
UninstallDisplayName=SLOPP
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; Closes a running panel (Restart Manager), forcefully if it doesn't go.
CloseApplications=force
RestartApplications=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Bin}\slopp.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\sloppctl.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\README.md"; DestDir: "{app}"; Flags: ignoreversion

; No AppUserModelID: slopp.exe doesn't set one, and a shortcut whose ID
; differs from its window's splits a pinned taskbar button from the
; running window.
[Icons]
Name: "{autoprograms}\SLOPP"; Filename: "{app}\slopp.exe"
Name: "{autodesktop}\SLOPP"; Filename: "{app}\slopp.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\slopp.exe"; Description: "{cm:LaunchProgram,SLOPP}"; Flags: nowait postinstall skipifsilent

; The servers outlive the panel by design; without it nothing would stop
; them.
[UninstallRun]
Filename: "{app}\sloppctl.exe"; Parameters: "stop"; Flags: runhidden waituntilterminated; RunOnceId: "StopServers"
