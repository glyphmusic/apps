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

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"

[Icons]
Name: "{userprograms}\Glyph"; Filename: "{app}\glyph.exe"; WorkingDir: "{app}"; AppUserModelID: "com.glyphmusic.glyph"

[Run]
Filename: "{app}\glyph.exe"; Description: "Launch Glyph"; Flags: nowait postinstall skipifsilent
