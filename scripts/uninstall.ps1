#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\out\build\windows-x64-release')
)

$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitOperatingSystem -or -not [Environment]::Is64BitProcess) {
    throw 'Automatic Screen Camera requires 64-bit Windows and a 64-bit PowerShell process.'
}

function Wait-NamedMutex([Threading.Mutex]$Mutex, [int]$Milliseconds) {
    try { return $Mutex.WaitOne($Milliseconds) }
    catch [Threading.AbandonedMutexException] { return $true }
}

$deploymentMutex = [Threading.Mutex]::new($false, 'Global\AutomaticScreenCamera.StartupDeployment.v1')
$machineApplicationMutex = $null
$legacyApplicationMutex = $null
$deploymentMutexOwned = Wait-NamedMutex $deploymentMutex 30000
$machineApplicationMutexOwned = $false
$legacyApplicationMutexOwned = $false
try {
    if (-not $deploymentMutexOwned) {
        throw 'Another Automatic Screen Camera launch or deployment is still in progress.'
    }

$installDirectory = Join-Path $env:ProgramFiles 'Automatic Screen Camera'
$application = Join-Path $installDirectory 'AutomaticScreenCamera.exe'
$sourceDll = Join-Path $installDirectory 'AutomaticScreenCameraSource.dll'
$cleanupApplication = Join-Path $BuildDirectory 'src\windows\Release\AutomaticScreenCamera.exe'
if (-not (Test-Path -LiteralPath $cleanupApplication -PathType Leaf)) {
    throw "The Release cleanup helper was not found under $BuildDirectory. Build the Release configuration first."
}

$processes = @(Get-Process AutomaticScreenCamera -ErrorAction SilentlyContinue)
if ($processes.Count -ne 0) {
    $processes | Stop-Process -Force
    foreach ($process in $processes) {
        if (-not $process.WaitForExit(10000)) { throw "Application process $($process.Id) did not exit within 10 seconds." }
    }
}
$machineApplicationMutex = [Threading.Mutex]::new($false, 'Global\AutomaticScreenCamera.TrayLifetime.v2')
$machineApplicationMutexOwned = Wait-NamedMutex $machineApplicationMutex 0
if (-not $machineApplicationMutexOwned) {
    throw 'Automatic Screen Camera is still running in another Windows session.'
}
$legacyApplicationMutex = [Threading.Mutex]::new($false, 'Local\AutomaticScreenCamera.TrayInstance.v1')
$legacyApplicationMutexOwned = Wait-NamedMutex $legacyApplicationMutex 0
if (-not $legacyApplicationMutexOwned) {
    throw 'Automatic Screen Camera is still running in this Windows session.'
}
if (Test-Path -LiteralPath $application -PathType Leaf) {
    $removal = Start-Process -FilePath $cleanupApplication -ArgumentList '--remove-virtual-camera-under-lock' -Wait -PassThru
    if ($removal.ExitCode -ne 0) {
        throw "Virtual-camera removal failed with exit code $($removal.ExitCode)."
    }
}
if (Test-Path -LiteralPath $sourceDll -PathType Leaf) {
    $arguments = @('/s', '/u', ('"' + $sourceDll + '"'))
    $process = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "Media source unregistration failed with exit code $($process.ExitCode)."
    }
}
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name AutomaticScreenCamera -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Automatic Screen Camera.lnk') -Force -ErrorAction SilentlyContinue
if (Test-Path -LiteralPath $installDirectory) {
    Remove-Item -LiteralPath $installDirectory -Recurse -Force -ErrorAction Stop
}
if (Test-Path -LiteralPath $installDirectory) { throw "Install directory could not be removed: $installDirectory" }
$deploymentRegistryPath = 'HKLM:\SOFTWARE\AutomaticScreenCamera\Deployment'
$deployment = Get-ItemProperty -LiteralPath $deploymentRegistryPath -ErrorAction SilentlyContinue
if ($deployment.Mode -eq 'installed') {
    Remove-Item -LiteralPath $deploymentRegistryPath -Recurse -Force -ErrorAction Stop
}
Write-Host 'Automatic Screen Camera uninstalled. User configuration and logs were retained in LocalAppData.'
} finally {
    if ($legacyApplicationMutexOwned) { $legacyApplicationMutex.ReleaseMutex() }
    if ($legacyApplicationMutex) { $legacyApplicationMutex.Dispose() }
    if ($machineApplicationMutexOwned) { $machineApplicationMutex.ReleaseMutex() }
    if ($machineApplicationMutex) { $machineApplicationMutex.Dispose() }
    if ($deploymentMutexOwned) { $deploymentMutex.ReleaseMutex() }
    $deploymentMutex.Dispose()
}
