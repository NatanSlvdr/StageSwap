#ifndef MyAppVersion
  #define MyAppVersion GetEnv("ASC_INNO_APP_VERSION")
#endif
#ifndef MyNumericVersion
  #define MyNumericVersion GetEnv("ASC_INNO_NUMERIC_VERSION")
#endif
#ifndef BuildDirectory
  #define BuildDirectory GetEnv("ASC_INNO_BUILD_DIRECTORY")
#endif
#ifndef BuildConfiguration
  #define BuildConfiguration GetEnv("ASC_INNO_BUILD_CONFIGURATION")
#endif
#ifndef PackageOutputDirectory
  #define PackageOutputDirectory GetEnv("ASC_INNO_OUTPUT_DIRECTORY")
#endif
#ifndef SetupBaseFilename
  #define SetupBaseFilename GetEnv("ASC_INNO_SETUP_BASENAME")
#endif

#if MyAppVersion == ""
  #error package.ps1 must provide ASC_INNO_APP_VERSION
#endif
#if MyNumericVersion == ""
  #error package.ps1 must provide ASC_INNO_NUMERIC_VERSION
#endif
#if BuildDirectory == ""
  #error package.ps1 must provide ASC_INNO_BUILD_DIRECTORY
#endif
#if BuildConfiguration == ""
  #error package.ps1 must provide ASC_INNO_BUILD_CONFIGURATION
#endif
#if PackageOutputDirectory == ""
  #error package.ps1 must provide ASC_INNO_OUTPUT_DIRECTORY
#endif
#if SetupBaseFilename == ""
  #error package.ps1 must provide ASC_INNO_SETUP_BASENAME
#endif

#define AppExecutable BuildDirectory + "\src\windows\" + BuildConfiguration + "\AutomaticScreenCamera.exe"
#define SourceDll BuildDirectory + "\src\windows\media_source\" + BuildConfiguration + "\AutomaticScreenCameraSource.dll"

[Setup]
AppId={{F49244F4-D68C-4CA0-A03A-EAEF00596244}
AppName=Automatic Screen Camera
AppVersion={#MyAppVersion}
AppVerName=Automatic Screen Camera {#MyAppVersion}
AppPublisher=Automatic Screen Camera
DefaultDirName={autopf64}\Automatic Screen Camera
DefaultGroupName=Automatic Screen Camera
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\AutomaticScreenCamera.exe
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0.22000
PrivilegesRequired=admin
OutputDir={#PackageOutputDirectory}
OutputBaseFilename={#SetupBaseFilename}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no
SetupLogging=yes
VersionInfoVersion={#MyNumericVersion}
VersionInfoProductVersion={#MyNumericVersion}
VersionInfoProductTextVersion={#MyAppVersion}
VersionInfoDescription=Automatic Screen Camera Setup
VersionInfoProductName=Automatic Screen Camera
UsePreviousAppDir=yes
UsePreviousTasks=yes

[Tasks]
Name: "startwithwindows"; Description: "Start Automatic Screen Camera with Windows"; GroupDescription: "Additional options:"; Flags: unchecked

[Files]
Source: "{#AppExecutable}"; DestName: "AutomaticScreenCameraPrepare.exe"; Flags: dontcopy noencryption
Source: "{#AppExecutable}"; DestDir: "{app}"; DestName: "AutomaticScreenCamera.exe"; Flags: ignoreversion
Source: "{#SourceDll}"; DestDir: "{app}"; DestName: "AutomaticScreenCameraSource.dll"; Flags: ignoreversion regserver restartreplace

[Icons]
Name: "{autoprograms}\Automatic Screen Camera"; Filename: "{app}\AutomaticScreenCamera.exe"; WorkingDir: "{app}"

[Registry]
Root: HKLM64; Subkey: "SOFTWARE\AutomaticScreenCamera\Deployment"; ValueType: string; ValueName: "Mode"; ValueData: "installed"; Flags: uninsdeletekey
Root: HKLM64; Subkey: "SOFTWARE\AutomaticScreenCamera\Deployment"; ValueType: string; ValueName: "Version"; ValueData: "{#MyAppVersion}"
Root: HKLM64; Subkey: "SOFTWARE\AutomaticScreenCamera\Deployment"; ValueType: string; ValueName: "SourcePath"; ValueData: "{app}\AutomaticScreenCameraSource.dll"
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "AutomaticScreenCamera"; ValueData: """{app}\AutomaticScreenCamera.exe"""; Tasks: startwithwindows; Flags: uninsdeletevalue

[Run]
Filename: "{app}\AutomaticScreenCamera.exe"; Description: "Launch Automatic Screen Camera"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
  PrepareExecutable: String;
begin
  Result := '';
  ExtractTemporaryFile('AutomaticScreenCameraPrepare.exe');
  PrepareExecutable := ExpandConstant('{tmp}\AutomaticScreenCameraPrepare.exe');
  if not Exec(PrepareExecutable, '--prepare-install', '', SW_HIDE,
              ewWaitUntilTerminated, ResultCode) then
  begin
    Result := 'Could not prepare the existing Automatic Screen Camera deployment for installation.';
    exit;
  end;
  if ResultCode <> 0 then
    Result := 'The existing Automatic Screen Camera deployment could not be stopped or removed. Exit the tray application and try again.';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
  ApplicationPath: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    ApplicationPath := ExpandConstant('{app}\AutomaticScreenCamera.exe');
    if FileExists(ApplicationPath) then
    begin
      if (not Exec(ApplicationPath, '--prepare-uninstall', '', SW_HIDE,
                   ewWaitUntilTerminated, ResultCode)) or (ResultCode <> 0) then
        RaiseException('The virtual camera could not be removed. Exit Automatic Screen Camera and retry the uninstall.');
    end;
    RegDeleteValue(HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run',
      'AutomaticScreenCamera');
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and (not WizardIsTaskSelected('startwithwindows')) then
    RegDeleteValue(HKEY_CURRENT_USER,
      'Software\Microsoft\Windows\CurrentVersion\Run',
      'AutomaticScreenCamera');
end;
