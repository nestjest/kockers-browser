#define AppVersion "0.1.0-alpha.1"
#define AppName "Kockers Browser"
#define AppPublisher "KOCKERS SOCIETY"
#define AppURL "https://github.com/nestjest/kockers-browser"
#define AppExeName "kockers_browser.exe"

[Setup]
AppId={{F4A723A6-4AB7-4AD1-942C-11B4E0B1A06B}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={localappdata}\Programs\Kockers Browser
DefaultGroupName=Kockers Browser
AllowNoIcons=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=KockersBrowser-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\kockers_browser.dll
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=yes
RestartApplications=no
VersionInfoVersion=0.1.0.0
VersionInfoDescription={#AppName} Setup
VersionInfoProductName={#AppName}
VersionInfoProductVersion=0.1.0.0
VersionInfoProductTextVersion={#AppVersion}

[Files]
Source: "..\target\bundle\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs; Excludes: "*.pdb,*.lib,archive.json,CMakeLists.txt,debug.log"

[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; GroupDescription: "Дополнительные ярлыки:"; Flags: unchecked

[Icons]
Name: "{group}\Kockers Browser"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\kockers_browser.dll"
Name: "{group}\Удалить Kockers Browser"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Kockers Browser"; Filename: "{app}\{#AppExeName}"; IconFilename: "{app}\kockers_browser.dll"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Запустить Kockers Browser"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}\debug.log"
