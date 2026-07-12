#Requires -RunAsAdministrator
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$installDirectory = Join-Path $env:ProgramFiles 'Automatic Screen Camera'
$application = Join-Path $installDirectory 'AutomaticScreenCamera.exe'
$sourceDll = Join-Path $installDirectory 'AutomaticScreenCameraSource.dll'

Get-Process AutomaticScreenCamera -ErrorAction SilentlyContinue | Stop-Process -Force
if (Test-Path $application) { & $application --remove-virtual-camera }
if (Test-Path $sourceDll) { & "$env:SystemRoot\System32\regsvr32.exe" /s /u $sourceDll }
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name AutomaticScreenCamera -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Automatic Screen Camera.lnk') -Force -ErrorAction SilentlyContinue
Remove-Item $installDirectory -Recurse -Force -ErrorAction SilentlyContinue
Write-Host 'Automatic Screen Camera uninstalled. User configuration and logs were retained in LocalAppData.'
