; Build with scripts/package-release.py. Per-user installation; never installs a driver or starts a daemon.
[Setup]
AppId={{836CD3AA-31A9-4D03-A7F2-E996D0892DD7}
AppName=LoRa Mesh Tools
AppVersion={#Version}
VersionInfoVersion={#NumericVersion}
AppPublisher=LoRa Mesh contributors
AppPublisherURL=https://github.com/crockpotveggies/lora-mesh-rs
DefaultDirName={localappdata}\Programs\LoRaMesh
DefaultGroupName=LoRa Mesh
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
LicenseFile={#SourceDir}\LICENSE
InfoBeforeFile={#SourceDir}\INSTALL.txt
UninstallDisplayName=LoRa Mesh Tools
CloseApplications=yes
SetupLogging=yes
[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
[Icons]
Name: "{group}\LoRa Mesh Command Prompt"; Filename: "{cmd}"; Parameters: "/K cd /d ""{app}\bin"""; WorkingDir: "{app}\bin"
Name: "{group}\Installation guide"; Filename: "{app}\INSTALL.txt"
Name: "{group}\Uninstall LoRa Mesh"; Filename: "{uninstallexe}"
