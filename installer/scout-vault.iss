; Scout Vault installer (Inno Setup 6).
;
; Installs for the current user only (no administrator): the app goes to
; %LOCALAPPDATA%\Programs\Scout Vault, with a Start menu shortcut, an entry in
; "Installed apps" and, if chosen, "Start with Windows" (the same registry value
; the app's setting uses). Updating = running a newer installer over it.
;
; Build (from the repository root, after `cargo build --release`):
;   ISCC.exe /DAppVersion=0.7.0 installer\scout-vault.iss
; Output: dist\scout-vault-<version>-setup.exe

#ifndef AppVersion
  #error Pass the version: ISCC /DAppVersion=X.Y.Z installer\scout-vault.iss
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\scout-vault.exe"
#endif

#define AppName "Scout Vault"
#define AppExe "scout-vault.exe"
#define AppUrl "https://github.com/qedrohenrique/scout-vault"

[Setup]
; Never change the AppId: it identifies the installation for updates and uninstalling.
AppId={{1DDFCFAE-DDB3-451C-B403-F457C609A23A}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Scout Vault contributors
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
VersionInfoVersion={#AppVersion}
VersionInfoDescription={#AppName} Setup
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=
DefaultDirName={localappdata}\Programs\{#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
DisableReadyPage=yes
UsePreviousAppDir=yes
UsePreviousTasks=yes
UsePreviousLanguage=yes
ShowLanguageDialog=auto
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
WizardStyle=modern
WizardSizePercent=100
SetupIconFile=..\crates\scout-vault-app\assets\brand\scout-vault.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
OutputDir=..\dist
OutputBaseFilename=scout-vault-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
; The app hides in the tray instead of closing: [Code] asks it to quit with `--quit`.
CloseApplications=no
RestartApplications=no

[Languages]
Name: "ptbr"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
ptbr.AutoStart=Iniciar com o Windows (escondido na bandeja, com os atalhos prontos)
en.AutoStart=Start with Windows (hidden in the tray, with shortcuts ready)
ptbr.Extras=Opções:
en.Extras=Options:
ptbr.RemoveData=Apagar também os dados do Scout Vault neste computador?%n%nSão as preferências, o cofre em cache (criptografado), as fotos de perfil e a sessão lembrada. Sua conta e o cofre no servidor não são afetados.
en.RemoveData=Also delete Scout Vault's data on this computer?%n%nThat's your preferences, the cached (encrypted) vault, profile photos and the remembered session. Your account and the vault on the server are not affected.

[Tasks]
Name: "autostart"; Description: "{cm:AutoStart}"; GroupDescription: "{cm:Extras}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExe}"; Flags: ignoreversion
; The browser extension, to load as "unpacked" (until it is in the stores).
Source: "..\extension\*"; DestDir: "{app}\extension"; Excludes: "test"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; "Start with Windows" (the app's setting reads and writes this same value).
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExe}"" --tray"; Tasks: autostart; Flags: uninsdeletevalue
; Created by the app (browser autofill / start with Windows turned on later):
; only removed when uninstalling.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "{#AppName}"; Flags: uninsdeletevalue dontcreatekey
Root: HKCU; Subkey: "Software\Google\Chrome\NativeMessagingHosts\com.scoutvault.desktop"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\Microsoft\Edge\NativeMessagingHosts\com.scoutvault.desktop"; Flags: uninsdeletekey dontcreatekey
Root: HKCU; Subkey: "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.scoutvault.desktop"; Flags: uninsdeletekey dontcreatekey

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; The browser manifest the app writes next to its preferences.
Type: files; Name: "{userappdata}\{#AppName}\navegador-host.json"

[Code]
{ Asks a running Scout Vault (even hidden in the tray) to quit, so its files can be replaced. }
procedure QuitRunningApp(const Exe: String);
var
  Code: Integer;
begin
  if FileExists(Exe) then
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  { The new executable, from a temporary folder: it finds the running app
    wherever it was started from (an older install or the zip). }
  ExtractTemporaryFile('{#AppExe}');
  QuitRunningApp(ExpandConstant('{tmp}\{#AppExe}'));
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  QuitRunningApp(ExpandConstant('{app}\{#AppExe}'));
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{userappdata}\{#AppName}');
    if DirExists(DataDir) and not UninstallSilent() then
      if MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
