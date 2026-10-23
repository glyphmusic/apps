#ifndef AppVersion
  #define AppVersion "0.2.4"
#endif
#ifndef BinaryPath
  #define BinaryPath "..\..\target\release\glyph.exe"
#endif

[Setup]
AppId={{BC67FD4D-D87E-4FA7-AE16-3702E59B6155}
AppName=Glyph
AppVersion={#AppVersion}
AppPublisher=glyphmusic
AppPublisherURL=https://glyph.my
DefaultDirName={localappdata}\Programs\Glyph
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=glyph-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\icons\glyph.ico
UninstallDisplayIcon={app}\glyph.exe
UninstallDisplayName=Glyph
CloseApplications=yes
CloseApplicationsFilter=glyph.exe
RestartApplications=no
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; English, or German on a German Windows (no language dialog).
ShowLanguageDialog=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"

[Icons]
Name: "{userprograms}\Glyph"; Filename: "{app}\glyph.exe"; WorkingDir: "{app}"; AppUserModelID: "com.glyphmusic.glyph"

[Run]
Filename: "{app}\glyph.exe"; Description: "{cm:LaunchProgram,Glyph}"; Flags: nowait postinstall skipifsilent

[Registry]
; "Start with your computer" (Settings -> System) writes this value; the
; installer never creates it, the uninstaller removes it.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Glyph"; Flags: uninsdeletevalue dontcreatekey
; The glyph:// link handler the app registers (src/integrate.rs).
Root: HKCU; Subkey: "Software\Classes\glyph"; ValueType: none; Flags: uninsdeletekey dontcreatekey

[CustomMessages]
en.DataTitle=Delete Glyph's data on this computer?
en.DataText=Glyph keeps your library, playlists, settings, downloaded songs and caches in your user folder. Uninstalling leaves them there unless you choose otherwise.
en.DataAll=Delete all app data%nDownloaded songs, library, playlists, settings, sign-in and caches. This cannot be undone.
en.DataDownloads=Delete downloaded songs and caches only%nKeeps your library, playlists and settings for a later install.
en.DataKeep=Keep everything
de.DataTitle=Glyphs Daten auf diesem Computer löschen?
de.DataText=Glyph speichert deine Bibliothek, Playlists, Einstellungen, heruntergeladenen Songs und Caches in deinem Benutzerordner. Beim Deinstallieren bleiben sie dort, außer du wählst etwas anderes.
de.DataAll=Alle App-Daten löschen%nHeruntergeladene Songs, Bibliothek, Playlists, Einstellungen, Anmeldung und Caches. Das lässt sich nicht rückgängig machen.
de.DataDownloads=Nur heruntergeladene Songs und Caches löschen%nBibliothek, Playlists und Einstellungen bleiben für eine spätere Installation.
de.DataKeep=Alles behalten

[Code]
const
  DataKeep = 0;
  DataDownloads = 1;
  DataAll = 2;

var
  DataChoice: Integer;

procedure DeleteDir(const Dir: String);
begin
  if DirExists(Dir) then
    DelTree(Dir, True, True, True);
end;

{ Asked once the uninstall is confirmed. Closing the dialog keeps the data;
  silent uninstalls (scripts, package managers) always keep it. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Labels: TArrayOfString;
  ResultCode: Integer;
  Data: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    DataChoice := DataKeep;
    if not UninstallSilent then
    begin
      SetArrayLength(Labels, 3);
      Labels[0] := CustomMessage('DataAll');
      Labels[1] := CustomMessage('DataDownloads');
      Labels[2] := CustomMessage('DataKeep');
      case TaskDialogMsgBox(CustomMessage('DataTitle'), CustomMessage('DataText'), mbConfirmation, MB_YESNOCANCEL or MB_DEFBUTTON3, Labels, 0) of
        IDYES: DataChoice := DataAll;
        IDNO: DataChoice := DataDownloads;
      end;
    end;
    { Sign-in tokens live in the Windows credential store, not in a folder;
      the app removes its own entries while it is still installed. }
    if DataChoice = DataAll then
      Exec(ExpandConstant('{app}\glyph.exe'), '--uninstall-cleanup', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;
  if CurUninstallStep = usPostUninstall then
  begin
    { Same folders as glyph_core::paths on Windows: data and settings in
      Roaming, caches in Local. }
    Data := ExpandConstant('{userappdata}\glyph');
    if DataChoice = DataAll then
    begin
      DeleteDir(Data);
      DeleteDir(ExpandConstant('{localappdata}\glyph'));
    end
    else if DataChoice = DataDownloads then
    begin
      DeleteDir(Data + '\downloads');
      DeleteDir(Data + '\lyrics');
      DeleteDir(Data + '\synced_covers');
      DeleteDir(Data + '\bin');
      DeleteDir(Data + '\update');
      DeleteFile(Data + '\resolver_cache.json');
      DeleteFile(Data + '\youtube_quota.json');
      DeleteDir(ExpandConstant('{localappdata}\glyph'));
    end;
  end;
end;
