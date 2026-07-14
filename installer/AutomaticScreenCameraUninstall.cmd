@echo off
setlocal EnableExtensions DisableDelayedExpansion

rem Preserve an unelevated helper before the real uninstaller removes Program Files.
rem A private per-principal LocalAppData directory avoids loading the full app from a
rem shared temporary directory when enterprise software runs this launcher as SYSTEM.
set "CLEANUP_DIR="
set "CLEANUP_HELPER="
rem A high/System caller cannot safely execute the full app from user-writable
rem storage. Normal Windows Settings launches this coordinator at medium integrity;
rem already-elevated callers fall back to machine uninstall without this helper.
"%SystemRoot%\System32\whoami.exe" /groups 2>nul | "%SystemRoot%\System32\findstr.exe" /c:"S-1-16-12288" /c:"S-1-16-16384" >nul
if not errorlevel 1 goto helper_ready
if not defined LOCALAPPDATA goto helper_ready
:allocate_helper
set "CLEANUP_DIR=%LOCALAPPDATA%\AutomaticScreenCamera\Uninstall-%RANDOM%-%RANDOM%-%RANDOM%"
if exist "%CLEANUP_DIR%" goto allocate_helper
md "%CLEANUP_DIR%" >nul 2>&1
if errorlevel 1 goto helper_ready
set "CLEANUP_HELPER=%CLEANUP_DIR%\AutomaticScreenCameraCleanup.exe"
copy /b /y "%~dp0AutomaticScreenCamera.exe" "%CLEANUP_HELPER%" >nul 2>&1
if errorlevel 1 set "CLEANUP_HELPER="
:helper_ready

rem This command only stops and briefly reserves the tray; it is non-destructive.
start "" /wait "%~dp0AutomaticScreenCamera.exe" --stop-for-uninstall >nul 2>&1

if /i "%~2"=="quiet" goto quiet_uninstall

rem Defer any required restart until current-user cleanup below has completed.
start "" /wait "%~1" /NORESTART
goto uninstall_finished

:quiet_uninstall
  start "" /wait "%~1" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART

:uninstall_finished
set "UNINSTALL_EXIT=%ERRORLEVEL%"
if not "%UNINSTALL_EXIT%"=="0" goto cleanup_helper
set "UNINSTALL_EXIT=3010"
if not defined CLEANUP_HELPER goto cleanup_helper
if /i "%~2"=="quiet" (
  "%CLEANUP_HELPER%" --cleanup-user-after-uninstall >nul 2>&1
) else (
  "%CLEANUP_HELPER%" --cleanup-user-after-uninstall-and-prompt-restart >nul 2>&1
)
rem /NORESTART lets current-user cleanup finish first. Conservatively report the
rem standard Windows reboot-required code because Inno does not expose its pending
rem deletion result to this launcher.

:cleanup_helper
if defined CLEANUP_DIR rd /s /q "%CLEANUP_DIR%" >nul 2>&1
exit /b %UNINSTALL_EXIT%
