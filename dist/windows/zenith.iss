; The Windows setup program: Zenith for one user, without asking for
; administrator rights. Built by dist/windows/build.ps1, which passes the
; version and the folder holding the files.
;
; Run again as an update, without a window of its own (/VERYSILENT), by
; `zenithd update`, which shows Zenith's own window meanwhile: it ends
; Zenith, replaces the files and opens it again.

[Setup]
AppId={{6D0B6E1C-52A4-4F7E-9A3B-0E6C1F7B9D24}
AppName=Zenith
AppVersion={#AppVersion}
AppPublisher=Zenith
DefaultDirName={localappdata}\Programs\Zenith
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={#OutDir}
OutputBaseFilename=Zenith-{#AppVersion}-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\zenith-clips.exe
UninstallDisplayName=Zenith
WizardStyle=modern
SetupIconFile=..\..\app\icons\zenith.ico

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
; The id is what Windows files Zenith's notifications under: with it on
; this shortcut they carry Zenith's name and icon. It must match
; `APP_ID` in crates/daemon/src/feedback.rs.
Name: "{autoprograms}\Zenith"; Filename: "{app}\zenith-clips.exe"; AppUserModelID: "Zenith.Clips"
Name: "{autoprograms}\Zenith recorder"; Filename: "{app}\zenith-clips.exe"; Parameters: "--start-recorder"; Comment: "Start Zenith's background recorder"

[Registry]
; The recorder starts when this user signs in. Written by a first install
; only: an update must not switch back on what was switched off in
; Settings ("Start with Windows").
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Zenith"; ValueData: """{app}\zenith-clips.exe"" --start-recorder"; Flags: uninsdeletevalue; Check: FirstInstall

[Run]
; The recorder now, without waiting for the next sign-in; then the app.
Filename: "{app}\zenith-clips.exe"; Parameters: "--start-recorder"; Flags: nowait
Filename: "{app}\zenith-clips.exe"; Description: "Open Zenith"; Flags: nowait postinstall

; What Zenith made for itself goes with it: the buffer, the logs, the
; thumbnails. The clips, the library index and the settings are the
; user's, and stay.
[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\Zenith\ring"
Type: filesandordirs; Name: "{localappdata}\Zenith\logs"
Type: filesandordirs; Name: "{localappdata}\Zenith\cache"
Type: filesandordirs; Name: "{localappdata}\Zenith\update"

; The recorder is asked first, so a clip being saved is finished; then
; everything of Zenith's that still runs is ended, the window included,
; or its files would stay behind.
[UninstallRun]
Filename: "{app}\zenithd.exe"; Parameters: "quit"; Flags: runhidden skipifdoesntexist; RunOnceId: "QuitRecorder"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM zenithd.exe /IM zenith-rec.exe /IM zenith-clips.exe"; Flags: runhidden; RunOnceId: "StopZenith"

[Code]
var
  // Whether Zenith was here before this setup ran: decided before any
  // file is copied, asked after.
  WasInstalled: Boolean;

function FirstInstall: Boolean;
begin
  Result := not WasInstalled;
end;

// The recorder runs in the background and holds its files open: it is
// stopped before they are replaced, and started again afterwards ([Run]).
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  WasInstalled := FileExists(ExpandConstant('{app}\zenith-clips.exe'));
  // Asked first, so a clip being saved is finished; then made to.
  if FileExists(ExpandConstant('{app}\zenithd.exe')) then
    Exec(ExpandConstant('{app}\zenithd.exe'), 'quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM zenithd.exe /IM zenith-rec.exe /IM zenith-clips.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;
