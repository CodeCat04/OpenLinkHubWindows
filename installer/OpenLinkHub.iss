; OpenLinkHub for Windows — Inno Setup script.
;
; Build with build.ps1 (recommended), or directly:
;   iscc /DAppVersion=0.1.0 installer\OpenLinkHub.iss
; after `cargo build --release`. Output: dist\OpenLinkHub-Setup-<version>.exe
;
; Installs per user (like VS Code or Discord): %LOCALAPPDATA%\Programs\OpenLinkHub, Start
; menu shortcut, optional desktop shortcut, optional start-with-Windows, and an entry in
; Settings > Apps for uninstalling. The only step that needs administrator rights is the
; optional CPU temperature sensor service, which is elevated on its own (one UAC prompt).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Never change AppId: Windows uses it to recognise upgrades of the same app.
AppId={{CB2D02BD-3032-4436-80BE-D265BD2D8549}
AppName=OpenLinkHub
AppVersion={#AppVersion}
AppVerName=OpenLinkHub {#AppVersion}
AppPublisher=OpenLinkHub contributors
AppPublisherURL=https://github.com/CodeCat04/OpenLinkHubWindows
AppSupportURL=https://github.com/CodeCat04/OpenLinkHubWindows
VersionInfoVersion={#AppVersion}
VersionInfoDescription=OpenLinkHub Setup
DefaultDirName={autopf}\OpenLinkHub
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=OpenLinkHub-Setup-{#AppVersion}
SetupIconFile=..\assets\OpenLinkHub.ico
UninstallDisplayIcon={app}\OpenLinkHub.exe
UninstallDisplayName=OpenLinkHub
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; We stop the running app ourselves (see PrepareToInstall), which also returns the hubs
; to hardware mode cleanly; Restart Manager would just kill it.
CloseApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start OpenLinkHub automatically when I sign in (recommended)"
Name: "cpusensor"; Description: "CPU temperature sensor (asks for administrator permission; needs the PawnIO driver)"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\OpenLinkHub.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\OpenLinkHubSensors.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\OpenLinkHub"; Filename: "{app}\OpenLinkHub.exe"; Comment: "Corsair iCUE LINK fan and lighting control"
Name: "{autodesktop}\OpenLinkHub"; Filename: "{app}\OpenLinkHub.exe"; Tasks: desktopicon

[Registry]
; Same value the app's own "Start with Windows" toggle uses, so the two stay in sync.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "OpenLinkHub"; ValueData: """{app}\OpenLinkHub.exe"" --background"; Flags: uninsdeletevalue; Tasks: autostart
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "OpenLinkHub"; Flags: deletevalue uninsdeletevalue; Tasks: not autostart

[Run]
; Registers the sensor service (copies it to Program Files and runs it as SYSTEM). This is
; the only elevated step.
Filename: "{app}\OpenLinkHubSensors.exe"; Parameters: "install"; Verb: "runas"; Flags: shellexec waituntilterminated runhidden; Tasks: cpusensor; StatusMsg: "Installing the CPU temperature sensor service..."
Filename: "{app}\OpenLinkHub.exe"; Description: "Launch OpenLinkHub"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Quit the running app first; this returns the hubs to hardware (firmware) control.
Filename: "{app}\OpenLinkHub.exe"; Parameters: "--stop"; Flags: runhidden waituntilterminated; RunOnceId: "StopOpenLinkHub"

[Code]
// Before installing or upgrading, stop any running OpenLinkHub (installed or portable)
// so its files can be replaced and the new copy can take over the hubs.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  ExtractTemporaryFile('OpenLinkHub.exe');
  Exec(ExpandConstant('{tmp}\OpenLinkHub.exe'), '--stop', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;

const
  SensorServiceKey = 'SYSTEM\CurrentControlSet\Services\OpenLinkHubSensors';

// Remove the sensor service (elevated) if it was installed, then offer to remove settings
// (fan curves, lighting, logs) when uninstalling interactively.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
  ResultCode: Integer;
begin
  if (CurUninstallStep = usUninstall) and RegKeyExists(HKLM, SensorServiceKey) then
    ShellExec('runas', ExpandConstant('{app}\OpenLinkHubSensors.exe'), 'uninstall', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{userappdata}\OpenLinkHub');
    if DirExists(DataDir) and not UninstallSilent then
      if MsgBox('Also delete your OpenLinkHub settings (fan curves, lighting and logs)?' + #13#10 + #13#10 +
                'Choose No to keep them for a future reinstall.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
