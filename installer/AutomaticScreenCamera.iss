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
DefaultDirName={commonpf64}\Automatic Screen Camera
DefaultGroupName=Automatic Screen Camera
DisableDirPage=yes
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
CloseApplications=no
RestartApplications=no
SetupLogging=yes
VersionInfoVersion={#MyNumericVersion}
VersionInfoProductVersion={#MyNumericVersion}
VersionInfoProductTextVersion={#MyAppVersion}
VersionInfoDescription=Automatic Screen Camera Setup
VersionInfoProductName=Automatic Screen Camera
UsePreviousAppDir=no
UsePreviousTasks=yes

[Tasks]
Name: "startwithwindows"; Description: "Start Automatic Screen Camera with Windows"; GroupDescription: "Additional options:"; Flags: unchecked

[Files]
Source: "{#AppExecutable}"; DestDir: "{app}"; DestName: "AutomaticScreenCamera.exe"; Flags: ignoreversion
Source: "{#SourceDll}"; DestDir: "{app}"; DestName: "AutomaticScreenCameraSource.dll"; Flags: ignoreversion regserver restartreplace
Source: "AutomaticScreenCameraUninstall.cmd"; DestDir: "{app}"; Flags: ignoreversion uninsrestartdelete

[Icons]
Name: "{autoprograms}\Automatic Screen Camera"; Filename: "{app}\AutomaticScreenCamera.exe"; WorkingDir: "{app}"

[Registry]
Root: HKLM64; Subkey: "SOFTWARE\AutomaticScreenCamera\Deployment"; ValueType: string; ValueName: "Mode"; ValueData: "installed"; Flags: uninsdeletekey
Root: HKLM64; Subkey: "SOFTWARE\AutomaticScreenCamera\Deployment"; ValueType: string; ValueName: "Version"; ValueData: "{#MyAppVersion}"
Root: HKLM64; Subkey: "SOFTWARE\AutomaticScreenCamera\Deployment"; ValueType: string; ValueName: "SourcePath"; ValueData: "{app}\AutomaticScreenCameraSource.dll"

[Run]
Filename: "{app}\AutomaticScreenCamera.exe"; Description: "Launch Automatic Screen Camera"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
type
  TSecurityAttributes = record
    Length: LongWord;
    SecurityDescriptor: LongWord;
    InheritHandle: Integer;
  end;

const
  DeploymentMutexName = 'Global\AutomaticScreenCamera.StartupDeployment.v1';
  DeploymentMutexSddl = 'D:(A;;0x00100001;;;AU)(A;;GA;;;BA)(A;;GA;;;SY)';
  MachineApplicationMutexName = 'Global\AutomaticScreenCamera.TrayLifetime.v2';
  LegacyApplicationMutexName = 'Local\AutomaticScreenCamera.TrayInstance.v1';
  DeploymentRegistryKey = 'SOFTWARE\AutomaticScreenCamera\Deployment';
  UninstallRegistryKey = 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{F49244F4-D68C-4CA0-A03A-EAEF00596244}_is1';
  RunRegistryKey = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Run';
  WaitObject0 = 0;
  WaitAbandoned = 128;
  ErrorAlreadyExists = 183;
  CreateMutexInitialOwner = 1;
  MutexSynchronizeAndModify = $00100001;
  DeploymentMutexWaitMilliseconds = 30000;
  ApplicationMutexWaitMilliseconds = 0;
  SddlRevision1 = 1;

var
  DeploymentMutexHandle: LongWord;
  DeploymentMutexOwned: Boolean;
  MachineApplicationMutexHandle: LongWord;
  MachineApplicationMutexOwned: Boolean;
  LegacyApplicationMutexHandle: LongWord;
  LegacyApplicationMutexOwned: Boolean;
  MigratingPortable: Boolean;
  SetupCommitted: Boolean;
  DeploymentSnapshotTaken: Boolean;
  PreviousDeploymentModeExists: Boolean;
  PreviousDeploymentVersionExists: Boolean;
  PreviousDeploymentSourcePathExists: Boolean;
  PreviousDeploymentMode: String;
  PreviousDeploymentVersion: String;
  PreviousDeploymentSourcePath: String;

function ConvertStringSecurityDescriptorToSecurityDescriptorW(
  StringSecurityDescriptor: String; StringSDRevision: LongWord;
  var SecurityDescriptor: LongWord; var SecurityDescriptorSize: LongWord): Integer;
  external 'ConvertStringSecurityDescriptorToSecurityDescriptorW@advapi32.dll stdcall';
function CreateMutexExW(var SecurityAttributes: TSecurityAttributes; Name: String;
  Flags: LongWord; DesiredAccess: LongWord): LongWord;
  external 'CreateMutexExW@kernel32.dll stdcall';
function WaitForSingleObject(Handle: LongWord; Milliseconds: LongWord): LongWord;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function ReleaseMutex(Handle: LongWord): Integer;
  external 'ReleaseMutex@kernel32.dll stdcall';
function CloseHandle(Handle: LongWord): Integer;
  external 'CloseHandle@kernel32.dll stdcall';
function LocalFree(Memory: LongWord): LongWord;
  external 'LocalFree@kernel32.dll stdcall';
procedure SetLastError(ErrorCode: LongWord);
  external 'SetLastError@kernel32.dll stdcall';

function CreateSharedMutex(Name: String; var InitiallyOwned: Boolean;
  var ErrorCode: LongWord): LongWord;
var
  SecurityAttributes: TSecurityAttributes;
  SecurityDescriptor: LongWord;
  SecurityDescriptorSize: LongWord;
begin
  Result := 0;
  InitiallyOwned := False;
  ErrorCode := 0;
  SecurityDescriptor := 0;
  SecurityDescriptorSize := 0;
  if ConvertStringSecurityDescriptorToSecurityDescriptorW(
    DeploymentMutexSddl, SddlRevision1, SecurityDescriptor,
    SecurityDescriptorSize) = 0 then
  begin
    Log('Could not create the deployment mutex security descriptor. Win32 error ' +
      IntToStr(DLLGetLastError()) + '.');
    exit;
  end;

  SecurityAttributes.Length := SizeOf(SecurityAttributes);
  SecurityAttributes.SecurityDescriptor := SecurityDescriptor;
  SecurityAttributes.InheritHandle := 0;
  try
    { CreateMutex only sets ERROR_ALREADY_EXISTS on the existing-object path. }
    SetLastError(0);
    Result := CreateMutexExW(SecurityAttributes, Name,
      CreateMutexInitialOwner, MutexSynchronizeAndModify);
    ErrorCode := DLLGetLastError();
    InitiallyOwned := (Result <> 0) and
      (ErrorCode <> ErrorAlreadyExists);
  finally
    LocalFree(SecurityDescriptor);
  end;
end;

function AcquireSharedMutex(Name: String; Milliseconds: LongWord;
  var Handle: LongWord; var Owned: Boolean): Boolean;
var
  WaitResult: LongWord;
  CreateError: LongWord;
  InitiallyOwned: Boolean;
begin
  if Owned then
  begin
    Result := True;
    exit;
  end;

  Handle := CreateSharedMutex(Name, InitiallyOwned, CreateError);
  if Handle = 0 then
  begin
    Log('Could not create shared mutex ' + Name + '. Win32 error ' +
      IntToStr(CreateError) + '.');
    Result := False;
    exit;
  end;

  if InitiallyOwned then
  begin
    Owned := True;
    Result := True;
    exit;
  end;

  WaitResult := WaitForSingleObject(Handle, Milliseconds);
  Owned := (WaitResult = WaitObject0) or
    (WaitResult = WaitAbandoned);
  if not Owned then
  begin
    Log('Timed out or failed waiting for shared mutex ' + Name +
      '. Wait result ' + IntToStr(WaitResult) + '.');
    CloseHandle(Handle);
    Handle := 0;
  end;
  Result := Owned;
end;

procedure ReleaseSharedMutex(var Handle: LongWord; var Owned: Boolean);
begin
  if Owned then
  begin
    ReleaseMutex(Handle);
    Owned := False;
  end;
  if Handle <> 0 then
  begin
    CloseHandle(Handle);
    Handle := 0;
  end;
end;

function AcquireDeploymentLocks: Boolean;
begin
  Result := AcquireSharedMutex(DeploymentMutexName,
    DeploymentMutexWaitMilliseconds, DeploymentMutexHandle,
    DeploymentMutexOwned);
  if not Result then
    exit;

  Result := AcquireSharedMutex(MachineApplicationMutexName,
    ApplicationMutexWaitMilliseconds, MachineApplicationMutexHandle,
    MachineApplicationMutexOwned);
  if not Result then
  begin
    ReleaseSharedMutex(DeploymentMutexHandle, DeploymentMutexOwned);
    exit;
  end;

  Result := AcquireSharedMutex(LegacyApplicationMutexName,
    ApplicationMutexWaitMilliseconds, LegacyApplicationMutexHandle,
    LegacyApplicationMutexOwned);
  if not Result then
  begin
    ReleaseSharedMutex(MachineApplicationMutexHandle,
      MachineApplicationMutexOwned);
    ReleaseSharedMutex(DeploymentMutexHandle, DeploymentMutexOwned);
  end;
end;

procedure ReleaseDeploymentLocks;
begin
  ReleaseSharedMutex(LegacyApplicationMutexHandle,
    LegacyApplicationMutexOwned);
  ReleaseSharedMutex(MachineApplicationMutexHandle,
    MachineApplicationMutexOwned);
  ReleaseSharedMutex(DeploymentMutexHandle, DeploymentMutexOwned);
end;

function ReadDeploymentMode: String;
begin
  if not RegQueryStringValue(HKEY_LOCAL_MACHINE_64, DeploymentRegistryKey,
    'Mode', Result) then
    Result := '';
end;

procedure SnapshotDeploymentMarker;
begin
  PreviousDeploymentModeExists := RegQueryStringValue(
    HKEY_LOCAL_MACHINE_64, DeploymentRegistryKey, 'Mode',
    PreviousDeploymentMode);
  PreviousDeploymentVersionExists := RegQueryStringValue(
    HKEY_LOCAL_MACHINE_64, DeploymentRegistryKey, 'Version',
    PreviousDeploymentVersion);
  PreviousDeploymentSourcePathExists := RegQueryStringValue(
    HKEY_LOCAL_MACHINE_64, DeploymentRegistryKey, 'SourcePath',
    PreviousDeploymentSourcePath);
  DeploymentSnapshotTaken := True;
end;

procedure RestoreDeploymentMarkerValue(ValueName: String;
  ValueExisted: Boolean; ValueData: String);
begin
  if ValueExisted then
  begin
    if not RegWriteStringValue(HKEY_LOCAL_MACHINE_64,
      DeploymentRegistryKey, ValueName, ValueData) then
      Log('Could not restore deployment marker value ' + ValueName + '.');
  end
  else
    RegDeleteValue(HKEY_LOCAL_MACHINE_64, DeploymentRegistryKey,
      ValueName);
end;

procedure RestorePreviousDeployment;
begin
  if not DeploymentSnapshotTaken then
    exit;

  if PreviousDeploymentSourcePathExists and
    FileExists(PreviousDeploymentSourcePath) and
    ((CompareText(PreviousDeploymentMode, 'portable') = 0) or
     (CompareText(PreviousDeploymentMode, 'installed') = 0)) then
  begin
    try
      RegisterServer(True, PreviousDeploymentSourcePath, False);
      Log('Restored the previous camera-source registration after Setup rollback.');
    except
      Log('Could not restore the previous camera-source registration after Setup rollback: ' +
        GetExceptionMessage + '.');
    end;
  end;

  RestoreDeploymentMarkerValue('Mode', PreviousDeploymentModeExists,
    PreviousDeploymentMode);
  RestoreDeploymentMarkerValue('Version', PreviousDeploymentVersionExists,
    PreviousDeploymentVersion);
  RestoreDeploymentMarkerValue('SourcePath',
    PreviousDeploymentSourcePathExists, PreviousDeploymentSourcePath);
  RegDeleteKeyIfEmpty(HKEY_LOCAL_MACHINE_64, DeploymentRegistryKey);
  Log('Restored the previous deployment marker after Setup rollback.');
end;

function PortableSourcePath: String;
begin
  Result := ExpandConstant(
    '{commonpf64}\Automatic Screen Camera Portable\AutomaticScreenCameraSource.dll');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  DeploymentMode: String;
begin
  Result := '';
  if not AcquireDeploymentLocks then
  begin
    Result := 'Automatic Screen Camera is running or another launch or deployment is still in progress. Exit the tray application, wait for other operations to finish, then run Setup again.';
    exit;
  end;

  SnapshotDeploymentMarker;
  DeploymentMode := ReadDeploymentMode;
  MigratingPortable := CompareText(DeploymentMode, 'portable') = 0;
  if (DeploymentMode <> '') and (not MigratingPortable) and
    (CompareText(DeploymentMode, 'installed') <> 0) then
  begin
    Result := 'The existing Automatic Screen Camera deployment registry data is not recognized.';
    ReleaseDeploymentLocks;
  end;
end;

procedure ApplyOriginalUserState;
var
  ResultCode: Integer;
  ApplicationPath: String;
  RegistryExecutable: String;
  RegistryParameters: String;
begin
  ApplicationPath := ExpandConstant('{app}\AutomaticScreenCamera.exe');
  try
    if not ExecAsOriginalUser(ApplicationPath,
      '--prepare-uninstall-under-lock', '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode) then
      Log('Original-user camera cleanup could not be started.')
    else if ResultCode <> 0 then
      Log('Original-user camera cleanup returned exit code ' +
        IntToStr(ResultCode) + '; continuing with startup configuration.');
  except
    Log('Original-user camera cleanup raised an exception: ' +
      GetExceptionMessage + '. Continuing with startup configuration.');
  end;

  RegistryExecutable := ExpandConstant('{sys}\reg.exe');
  if WizardIsTaskSelected('startwithwindows') then
    RegistryParameters := 'add "' + RunRegistryKey +
      '" /v "AutomaticScreenCamera" /t REG_SZ /d "\"' +
      ApplicationPath + '\"" /f'
  else
    RegistryParameters := 'delete "' + RunRegistryKey +
      '" /v "AutomaticScreenCamera" /f';

  try
    if not ExecAsOriginalUser(RegistryExecutable, RegistryParameters, '',
      SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      Log('Original-user startup configuration could not be started.')
    else if ResultCode <> 0 then
      Log('Original-user startup configuration returned exit code ' +
        IntToStr(ResultCode) + '.');
  except
    Log('Original-user startup configuration raised an exception: ' +
      GetExceptionMessage + '.');
  end;
end;

function RegisterMigratedInstalledSource: Boolean;
var
  InstalledSource: String;
begin
  Result := True;
  if not MigratingPortable then
    exit;

  InstalledSource := ExpandConstant(
    '{app}\AutomaticScreenCameraSource.dll');
  try
    { NeedRestart defers Inno's regserver entry to RunOnce. Register the
      committed file now before removing the source used by portable mode. }
    RegisterServer(True, InstalledSource, False);
    Log('Registered the installed camera source before portable cleanup.');
  except
    Result := False;
    Log('Could not register the installed camera source before portable cleanup: ' +
      GetExceptionMessage + '. The portable source will be retained and the required restart will retry registration.');
  end;
end;

procedure RemoveMigratedPortableFiles;
var
  PortableDirectory: String;
  PortableSource: String;
begin
  if not MigratingPortable then
    exit;

  PortableDirectory := ExpandConstant(
    '{commonpf64}\Automatic Screen Camera Portable');
  PortableSource := PortableSourcePath;
  if DelTree(PortableDirectory, True, True, True) then
  begin
    Log('Removed the previous portable deployment after Setup committed.');
    exit;
  end;

  Log('The previous portable deployment is still in use; scheduling cleanup at restart.');
  try
    if FileExists(PortableSource) then
      RestartReplace(PortableSource, '');
    if DirExists(PortableDirectory) then
      RestartReplace(PortableDirectory, '');
  except
    Log('Could not schedule all portable deployment remnants for removal: ' +
      GetExceptionMessage + '.');
  end;
end;

procedure ConfigureUninstallLauncher;
var
  CommandInterpreter: String;
  LauncherPath: String;
  UninstallerPath: String;
  UninstallCommand: String;
  QuietUninstallCommand: String;
  UninstallRegistryRoot: Integer;
begin
  CommandInterpreter := ExpandConstant('{cmd}');
  LauncherPath := ExpandConstant(
    '{app}\AutomaticScreenCameraUninstall.cmd');
  UninstallerPath := ExpandConstant('{uninstallexe}');
  UninstallCommand := '"' + CommandInterpreter +
    '" /d /s /c ""' + LauncherPath + '" "' + UninstallerPath + '""';
  QuietUninstallCommand := '"' + CommandInterpreter +
    '" /d /s /c ""' + LauncherPath + '" "' + UninstallerPath +
    '" quiet"';

  if RegKeyExists(HKEY_LOCAL_MACHINE_64, UninstallRegistryKey) then
    UninstallRegistryRoot := HKEY_LOCAL_MACHINE_64
  else if RegKeyExists(HKEY_LOCAL_MACHINE_32, UninstallRegistryKey) then
    UninstallRegistryRoot := HKEY_LOCAL_MACHINE_32
  else
  begin
    Log('Could not locate the installed-program registry entry to configure original-user cleanup.');
    exit;
  end;

  if not RegWriteStringValue(UninstallRegistryRoot, UninstallRegistryKey,
    'UninstallString', UninstallCommand) then
    Log('Could not configure original-user uninstall cleanup.');
  if not RegWriteStringValue(UninstallRegistryRoot, UninstallRegistryKey,
    'QuietUninstallString', QuietUninstallCommand) then
    Log('Could not configure quiet original-user uninstall cleanup.');
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  InstalledSourceRegistered: Boolean;
begin
  if CurStep = ssPostInstall then
  begin
    SetupCommitted := True;
    try
      InstalledSourceRegistered := RegisterMigratedInstalledSource;
      ApplyOriginalUserState;
      if InstalledSourceRegistered then
        RemoveMigratedPortableFiles
      else
        Log('Retaining the portable camera source until installed registration succeeds.');
      ConfigureUninstallLauncher;
    finally
      ReleaseDeploymentLocks;
    end;
  end;
end;

procedure DeinitializeSetup;
begin
  if not SetupCommitted then
    RestorePreviousDeployment;
  ReleaseDeploymentLocks;
end;

function NeedRestart: Boolean;
begin
  { Cleanup is deliberately post-commit, after Inno has evaluated this event. }
  Result := MigratingPortable;
end;

function IsSilentUninstall: Boolean;
var
  Index: Integer;
begin
  Result := False;
  for Index := 1 to ParamCount do
    if (CompareText(ParamStr(Index), '/SILENT') = 0) or
      (CompareText(ParamStr(Index), '/VERYSILENT') = 0) then
    begin
      Result := True;
      exit;
    end;
end;

function InitializeUninstall: Boolean;
begin
  Result := AcquireDeploymentLocks;
  if not Result then
  begin
    if IsSilentUninstall then
      Log('Uninstall could not acquire deployment locks; no UI was shown for silent uninstall.')
    else
      MsgBox('Automatic Screen Camera is running or another launch or deployment is still in progress. Exit the tray application, wait for other operations to finish, then try uninstalling again.',
        mbError, MB_OK);
    exit;
  end;
end;

function UninstallNeedRestart: Boolean;
begin
  { The system-lifetime virtual camera or Frame Server may retain the COM source.
    The launcher defers the prompt until original-user cleanup has completed. }
  Result := True;
end;

procedure DeinitializeUninstall;
begin
  ReleaseDeploymentLocks;
end;
