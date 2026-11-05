; Sondev Terminal インストーラー（Inno Setup 6）
; （開発中の名前は wimux。コードのフォルダ名・環境変数は wimux のまま）
; ※ このファイルは UTF-8（BOM 付き）で保存すること。BOM がないと日本語の環境で文字化けする
;
; 作り方は installer\build.ps1 を実行するだけ（リリースビルド → インストーラー作成）。
; 手動で作る場合: ISCC.exe /DAppVersion=0.1.0 installer\wimux.iss
;
; - 管理者権限なしで、ユーザーごとに %LOCALAPPDATA%\Programs\Sondev Terminal へインストール
; - スタートメニューに登録、デスクトップのアイコンは任意
; - 設定・セッション（%APPDATA%\SondevTerminal）はアンインストールしても残す

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

#define AppName "Sondev Terminal"
#define AppExe "sondev.exe"

[Setup]
; AppId はアップデートを同じアプリとして扱うための ID。変更しないこと
AppId={{6F3C2A1E-8B7D-4E59-9A3F-2C1D5E7B9A40}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Sondev
DefaultDirName={localappdata}\Programs\Sondev Terminal
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename=SondevTerminal-setup-{#AppVersion}
SetupIconFile=..\assets\wimux.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; 起動中の wimux があれば閉じてから上書きする
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\THIRD_PARTY_LICENSES.html"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
; cdi で使う fzf（MIT ライセンス）。build.ps1 が installer\vendor に用意する
Source: "vendor\fzf.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "vendor\fzf-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[InstallDelete]
; 以前の名前（wimux）で入れた exe を消す
Type: files; Name: "{app}\wimux.exe"

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; アプリ内の［更新］から /SILENT /relaunch=1 で呼ばれたときは、終わったら新しい版を起動する
Filename: "{app}\{#AppExe}"; Flags: nowait; Check: RelaunchRequested

[Code]
function RelaunchRequested: Boolean;
begin
  Result := ExpandConstant('{param:relaunch|0}') = '1';
end;
