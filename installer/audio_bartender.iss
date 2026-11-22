; Compile with build.ps1, which passes /DAppVersion.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
; Where build.ps1 put the portable exes, and where the installer goes.
#ifndef DistDir
  #define DistDir "..\dist"
#endif

#define AppGuid "AAC1CE9E-BB1A-486E-94D6-1018655731B6"
#define AppName "Audio Bartender"
; Both interfaces install under this one name, so the Run entry, the elevated task
; and the tray icon's identity survive switching between them.
#define AppExe  "AudioBartender.exe"
#define AppUrl  "https://github.com/Araxeus/Audio-Bartender"

[Setup]
AppId={{{#AppGuid}}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Araxeus
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
VersionInfoVersion={#AppVersion}
VersionInfoDescription={#AppName} Setup

; Setup asks whether to install for all users, which needs admin and adds the
; elevated task, or just for the current user.
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog commandline

; The elevated task must launch an exe that only admins can modify, so the folder stays fixed.
DefaultDirName={autopf}\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes

ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

; [Code] stops the app, so Restart Manager is not needed.
CloseApplications=no

SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern dynamic
WizardSizePercent=110
; Drawn by scripts/make-wizard-images.ps1 at 100%, 150% and 200% scale.
WizardImageFile=wizard-164.png,wizard-246.png,wizard-328.png
WizardSmallImageFile=wizard-small-55.png,wizard-small-82.png,wizard-small-110.png
Compression=lzma2/max
SolidCompression=yes
OutputDir={#DistDir}
OutputBaseFilename=audio_bartender_setup

[Messages]
WelcomeLabel2=This will install [name/ver] on your computer.%n%nAudio Bartender replaces the Windows volume icon with a mixer for every app, live level meters and media controls, and it lets you choose which app your keyboard's media keys control.

[Tasks]
Name: "startup"; Description: "Start {#AppName} with Windows"
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#DistDir}\audio_bartender_gpui.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Check: UseNative; Flags: ignoreversion
Source: "{#DistDir}\audio_bartender_tauri.exe"; DestDir: "{app}"; DestName: "{#AppExe}"; Check: UseWeb; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; Comment: "Per-app volume, level meters and media key routing"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

; The app owns its Run entry, so setup asks it to add or remove the entry. Running as
; the original user puts it in that user's registry rather than the admin's.
[Run]
Filename: "{app}\{#AppExe}"; Parameters: "--set-autostart=on"; Tasks: startup; Flags: runasoriginaluser runhidden waituntilterminated
Filename: "{app}\{#AppExe}"; Parameters: "--set-autostart=off"; Tasks: not startup; Flags: runasoriginaluser runhidden waituntilterminated
; An unelevated launch hands off to the elevated task when there is one.
Filename: "{app}\{#AppExe}"; Description: "Start {#AppName}"; Flags: nowait postinstall runasoriginaluser

[Code]
const
  TaskName = 'AudioBartender';
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{{#AppGuid}}_is1';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';
  RunValue = 'AudioBartender';
  WebView2Key = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

  TASK_CREATE_OR_UPDATE = 6;
  TASK_LOGON_INTERACTIVE_TOKEN = 3;

var
  // Install, Update, Repair, or Downgrade. Set from the version already installed in this mode.
  InstallAction: String;
  InstalledVersion: String;
  InterfacePage: TInputOptionWizardPage;

function UseNative: Boolean;
begin
  Result := InterfacePage.SelectedValueIndex = 0;
end;

function UseWeb: Boolean;
begin
  Result := InterfacePage.SelectedValueIndex = 1;
end;

// Windows 11 ships the WebView2 Runtime; some Windows 10 machines lack it.
function WebView2Installed: Boolean;
var
  Version: String;
begin
  Result :=
    (RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\' + WebView2Key, 'pv', Version) or
     RegQueryStringValue(HKCU, 'Software\' + WebView2Key, 'pv', Version)) and
    (Version <> '') and (Version <> '0.0.0.0');
end;

function GetInstallAction: String;
var
  Installed, Current: Int64;
  Order: Integer;
begin
  Result := 'Install';
  if not RegQueryStringValue(HKA, UninstallKey, 'DisplayVersion', InstalledVersion) then
    Exit;

  Result := 'Update';
  if not (StrToVersion(InstalledVersion, Installed) and StrToVersion('{#AppVersion}', Current)) then
    Exit;

  Order := ComparePackedVersion(Current, Installed);
  if Order = 0 then
    Result := 'Repair'
  else if Order < 0 then
    Result := 'Downgrade';
end;

function ActionGerund: String;
begin
  if InstallAction = 'Repair' then
    Result := 'Repairing'
  else if InstallAction = 'Update' then
    Result := 'Updating'
  else
    Result := 'Downgrading';
end;

procedure InitializeWizard;
begin
  InstallAction := GetInstallAction;
  Log('Install action: ' + InstallAction + ' (installed: ' + InstalledVersion + ', setup: {#AppVersion})');

  InterfacePage := CreateInputOptionPage(wpWelcome,
    'Choose your interface', 'Both work the same way. You can switch later by running setup again.',
    'Which interface should {#AppName} use?', True, False);
  InterfacePage.Add('Native (recommended)' + #13#10 + 'Fast and light, with the smallest memory use.');
  InterfacePage.Add('Web' + #13#10 + 'Built on Microsoft Edge WebView2.');
  InterfacePage.SelectedValueIndex := StrToIntDef(GetPreviousData('Interface', '0'), 0);
  // /INTERFACE=native or /INTERFACE=web picks one for silent installs.
  if CompareText(ExpandConstant('{param:interface|}'), 'web') = 0 then
    InterfacePage.SelectedValueIndex := 1
  else if CompareText(ExpandConstant('{param:interface|}'), 'native') = 0 then
    InterfacePage.SelectedValueIndex := 0;
end;

procedure RegisterPreviousData(PreviousDataKey: Integer);
begin
  SetPreviousData(PreviousDataKey, 'Interface', IntToStr(InterfacePage.SelectedValueIndex));
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = InterfacePage.ID) and UseWeb and not WebView2Installed then
    Result := SuppressibleMsgBox(
      'The Web interface needs the Microsoft Edge WebView2 Runtime, which isn''t installed on this computer.' + #13#10#13#10 +
      'You can get it from microsoft.com/edge/webview2, or go back and choose Native, which needs nothing extra.' + #13#10#13#10 +
      'Install the Web interface anyway?',
      mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES;
end;

// Relabels the wizard when this mode already has a copy installed.
procedure CurPageChanged(CurPageID: Integer);
begin
  if InstallAction = 'Install' then
    Exit;

  if CurPageID = wpReady then begin
    WizardForm.PageNameLabel.Caption := 'Ready to ' + InstallAction;
    WizardForm.PageDescriptionLabel.Caption := 'Setup is ready to ' + Lowercase(InstallAction) + ' {#AppName}.';

    if InstallAction = 'Repair' then
      WizardForm.ReadyLabel.Caption := 'Version ' + InstalledVersion + ' is already installed. ' +
        'Setup will reinstall it, restoring its files, startup entry and elevated task.'
    else if InstallAction = 'Update' then
      WizardForm.ReadyLabel.Caption := 'Setup will update {#AppName} from version ' +
        InstalledVersion + ' to {#AppVersion}.'
    else
      WizardForm.ReadyLabel.Caption := 'A newer version (' + InstalledVersion + ') is installed. ' +
        'Setup will replace it with version {#AppVersion}.';

    WizardForm.NextButton.Caption := '&' + InstallAction;
  end
  else if CurPageID = wpInstalling then begin
    WizardForm.PageNameLabel.Caption := ActionGerund;
    WizardForm.PageDescriptionLabel.Caption := 'Please wait while Setup ' + Lowercase(InstallAction) + 's {#AppName}.';
  end
  else if CurPageID = wpFinished then
    WizardForm.FinishedLabel.Caption := 'Setup has finished ' + Lowercase(ActionGerund) + ' {#AppName}.';
end;

procedure StopApp;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Sleep(500);
end;

function XmlEscape(S: String): String;
begin
  Result := S;
  StringChangeEx(Result, '&', '&amp;', True);
  StringChangeEx(Result, '<', '&lt;', True);
  StringChangeEx(Result, '>', '&gt;', True);
end;

// Registers an on-demand task that runs the exe elevated for the installing user.
// The exe hands off to it when launched unelevated. The settings override defaults that would
// kill the app after 72 hours, stop it on battery, and run it at below-normal priority.
procedure RegisterElevatedTask;
var
  Service, Folder: Variant;
  Dir, Xml: String;
begin
  Dir := XmlEscape(ExpandConstant('{app}'));
  Xml :=
    '<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">' +
    '<RegistrationInfo><Description>Runs {#AppName} elevated, so media keys also work while an admin window has focus.</Description></RegistrationInfo>' +
    '<Principals><Principal id="Author">' +
      '<LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel>' +
    '</Principal></Principals>' +
    '<Settings>' +
      '<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>' +
      '<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>' +
      '<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>' +
      '<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>' +
      '<Priority>4</Priority>' +
    '</Settings>' +
    '<Actions Context="Author"><Exec>' +
      '<Command>"' + Dir + '\{#AppExe}"</Command>' +
      '<Arguments>--task</Arguments>' +
      '<WorkingDirectory>' + Dir + '</WorkingDirectory>' +
    '</Exec></Actions>' +
    '</Task>';

  try
    Service := CreateOleObject('Schedule.Service');
    Service.Connect();
    Folder := Service.GetFolder('\');
    Folder.RegisterTask(TaskName, Xml, TASK_CREATE_OR_UPDATE, '', '', TASK_LOGON_INTERACTIVE_TOKEN);
  except
    SuppressibleMsgBox(
      'Could not register the elevated task, so the app will run without elevation.' + #13#10#13#10 +
      GetExceptionMessage, mbError, MB_OK, IDOK);
  end;
end;

// Keeps one install per user. An all-users install replaces a just-me install,
// and a just-me install refuses to run over an all-users install.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Uninstaller: String;
  ResultCode: Integer;
begin
  Result := '';

  if not IsAdminInstallMode and RegKeyExists(HKLM, UninstallKey) then begin
    Result := '{#AppName} is already installed for all users. ' +
      'Run this setup for all users to update it, or uninstall it first.';
    Exit;
  end;

  StopApp;

  if IsAdminInstallMode and RegQueryStringValue(HKCU, UninstallKey, 'UninstallString', Uninstaller) then
    Exec(RemoveQuotes(Uninstaller), '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and IsAdminInstallMode then
    RegisterElevatedTask;
end;

// Every cleanup step runs in either mode. A step that has nothing to remove, or lacks access, does nothing.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
begin
  if CurUninstallStep = usUninstall then begin
    Exec(ExpandConstant('{sys}\schtasks.exe'), '/delete /tn ' + TaskName + ' /f', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    StopApp;
  end;

  if CurUninstallStep = usPostUninstall then begin
    RegDeleteValue(HKCU, RunKey, RunValue);
    RegDeleteValue(HKCU, ApprovedKey, RunValue);
    RegDeleteKeyIncludingSubkeys(HKCU, 'Software\AudioBartender');
    if SuppressibleMsgBox('Also remove your {#AppName} settings?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
      DelTree(ExpandConstant('{userappdata}\AudioBartender'), True, True, True);
  end;
end;
