#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\build'),
    [switch]$StartWithWindows
)

$ErrorActionPreference = 'Stop'
$installDirectory = Join-Path $env:ProgramFiles 'Automatic Screen Camera'
$packagedSource = Join-Path $PSScriptRoot 'AutomaticScreenCameraSource.dll'
$packagedApplication = Join-Path $PSScriptRoot 'AutomaticScreenCamera.exe'
if ((Test-Path $packagedSource) -and (Test-Path $packagedApplication)) {
    $sourceDll = $packagedSource
    $application = $packagedApplication
} else {
    $sourceDll = Join-Path $BuildDirectory 'src\windows\media_source\Release\AutomaticScreenCameraSource.dll'
    $application = Join-Path $BuildDirectory 'src\windows\Release\AutomaticScreenCamera.exe'
}
if (-not (Test-Path $sourceDll) -or -not (Test-Path $application)) {
    throw "Build outputs were not found under $BuildDirectory. Build the Release configuration first."
}

Get-Process AutomaticScreenCamera -ErrorAction SilentlyContinue | Stop-Process -Force

New-Item -ItemType Directory -Force -Path $installDirectory | Out-Null
Copy-Item $application (Join-Path $installDirectory 'AutomaticScreenCamera.exe') -Force
Copy-Item $sourceDll (Join-Path $installDirectory 'AutomaticScreenCameraSource.dll') -Force

& "$env:SystemRoot\System32\regsvr32.exe" /s (Join-Path $installDirectory 'AutomaticScreenCameraSource.dll')
if ($LASTEXITCODE -ne 0) { throw "Media source registration failed with exit code $LASTEXITCODE." }

$shell = New-Object -ComObject WScript.Shell
$startMenu = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Automatic Screen Camera.lnk'
$shortcut = $shell.CreateShortcut($startMenu)
$shortcut.TargetPath = Join-Path $installDirectory 'AutomaticScreenCamera.exe'
$shortcut.WorkingDirectory = $installDirectory
$shortcut.Save()

$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if ($StartWithWindows) {
    New-ItemProperty -Path $runKey -Name AutomaticScreenCamera -Value ('"' + (Join-Path $installDirectory 'AutomaticScreenCamera.exe') + '"') -PropertyType String -Force | Out-Null
}

Write-Host "Automatic Screen Camera installed to $installDirectory"
Write-Host 'Launch it from the Start menu. Windows camera privacy access must be enabled.'
