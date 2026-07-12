#Requires -RunAsAdministrator
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitOperatingSystem -or -not [Environment]::Is64BitProcess) {
    throw 'Automatic Screen Camera requires 64-bit Windows and a 64-bit PowerShell process.'
}

$installDirectory = Join-Path $env:ProgramFiles 'Automatic Screen Camera'
$application = Join-Path $installDirectory 'AutomaticScreenCamera.exe'
$sourceDll = Join-Path $installDirectory 'AutomaticScreenCameraSource.dll'

$processes = @(Get-Process AutomaticScreenCamera -ErrorAction SilentlyContinue)
if ($processes.Count -ne 0) {
    $processes | Stop-Process -Force
    foreach ($process in $processes) {
        if (-not $process.WaitForExit(10000)) { throw "Application process $($process.Id) did not exit within 10 seconds." }
    }
}
if (Test-Path -LiteralPath $application -PathType Leaf) {
    $removal = Start-Process -FilePath $application -ArgumentList '--remove-virtual-camera' -Wait -PassThru
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
Write-Host 'Automatic Screen Camera uninstalled. User configuration and logs were retained in LocalAppData.'
