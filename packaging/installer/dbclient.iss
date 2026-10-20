; dbclient direct-download installer (Inno Setup 6). Built by packaging\Build-Installer.ps1,
; which passes the defines below.
;
;   /DAppVersion=1.2.3   required
;   /DStageDir=<dir>     required: holds dbclient.exe, MicrosoftEdgeWebview2Setup.exe and the
;                        license files (LICENSE, THIRD_PARTY_NOTICES.md, THIRD_PARTY_LICENSES.txt)
;   /DOutputDir=<dir>    required
;
; Installs per user by default (no UAC prompt; the app needs no elevation). The first page offers
; "Install for all users" for a machine-wide install under Program Files.
; Command line: /ALLUSERS or /CURRENTUSER to pick without the prompt; /ALLOWDOWNGRADE to install
; over a newer version (refused otherwise).

#ifndef AppVersion
  #error AppVersion is not defined; build with packaging\Build-Installer.ps1
#endif

#define AppName "dbclient"
#define AppExe "dbclient.exe"
#define AppIdGuid "{{7D2B5E64-3A9F-4C1B-9E8D-5F6A1C2B3D4E}"

[Setup]
AppId={#AppIdGuid}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Tim Dodd
AppPublisherURL=https://github.com/timothydodd/dbclient
AppSupportURL=https://github.com/timothydodd/dbclient/issues
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=dbclient-Setup-{#AppVersion}
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
SetupIconFile=..\..\src-tauri\icons\icon.ico
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog commandline
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Restart Manager plus the taskkill in PrepareToInstall make sure the app isn't holding files
; during an upgrade.
CloseApplications=force
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#StageDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\THIRD_PARTY_LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\MicrosoftEdgeWebview2Setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: NeedsWebView2

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{tmp}\MicrosoftEdgeWebview2Setup.exe"; Parameters: "/silent /install"; StatusMsg: "Installing the Microsoft Edge WebView2 runtime..."; Flags: waituntilterminated; Check: NeedsWebView2
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Stop a running copy so its files can be removed.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM {#AppExe}"; Flags: runhidden; RunOnceId: "StopApp"

[Code]
const
  WebView2ClientKey = 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

{ WebView2 is present when EdgeUpdate reports a version for its client id, machine-wide (either
  registry view) or for this user. Windows 11 and updated Windows 10 already have it. }
function HasWebView2Version(RootKey: Integer; const Key: String): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(RootKey, Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0');
end;

function NeedsWebView2: Boolean;
begin
  Result := not (HasWebView2Version(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')
    or HasWebView2Version(HKLM, WebView2ClientKey)
    or HasWebView2Version(HKCU, WebView2ClientKey));
end;

{ Numeric dotted-version compare: -1, 0 or 1. }
function CompareVersions(A, B: String): Integer;
var
  PA, PB, NA, NB: Integer;
begin
  Result := 0;
  while (Result = 0) and ((A <> '') or (B <> '')) do
  begin
    PA := Pos('.', A);
    if PA = 0 then PA := Length(A) + 1;
    PB := Pos('.', B);
    if PB = 0 then PB := Length(B) + 1;
    NA := StrToIntDef(Copy(A, 1, PA - 1), 0);
    NB := StrToIntDef(Copy(B, 1, PB - 1), 0);
    Delete(A, 1, PA);
    Delete(B, 1, PB);
    if NA < NB then
      Result := -1
    else if NA > NB then
      Result := 1;
  end;
end;

function HasParam(const Name: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Name) = 0 then
      Result := True;
end;

{ Refuse to replace a newer installed version unless /ALLOWDOWNGRADE is given. }
function InitializeSetup: Boolean;
var
  Installed, Key: String;
begin
  Result := True;
  Key := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{7D2B5E64-3A9F-4C1B-9E8D-5F6A1C2B3D4E}_is1';
  if (RegQueryStringValue(HKCU, Key, 'DisplayVersion', Installed) or RegQueryStringValue(HKLM, Key, 'DisplayVersion', Installed))
     and (CompareVersions(Installed, '{#AppVersion}') > 0) and not HasParam('/ALLOWDOWNGRADE') then
  begin
    SuppressibleMsgBox('A newer version of dbclient (' + Installed + ') is already installed. Run this installer with /ALLOWDOWNGRADE to replace it with {#AppVersion}.', mbError, MB_OK, IDOK);
    Result := False;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  { Stop a running copy so the executable can be replaced. }
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;
