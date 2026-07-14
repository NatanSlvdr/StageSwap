#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\out\build\windows-x64-release'),
    [switch]$StartWithWindows
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
$deploymentRegistryPath = 'HKLM:\SOFTWARE\AutomaticScreenCamera\Deployment'
$priorDeployment = Get-ItemProperty -LiteralPath $deploymentRegistryPath -ErrorAction SilentlyContinue
if ($priorDeployment.Mode -eq 'portable') {
    throw 'The portable edition is active. Run its --cleanup-portable command or use the Setup EXE to migrate it automatically.'
}
$transactionId = [Guid]::NewGuid().ToString('N')
$stagingDirectory = Join-Path $env:ProgramFiles "Automatic Screen Camera.installing-$transactionId"
$backupDirectory = Join-Path $env:ProgramFiles "Automatic Screen Camera.backup-$transactionId"
$sourceDll = Join-Path $BuildDirectory 'src\windows\media_source\Release\AutomaticScreenCameraSource.dll'
$application = Join-Path $BuildDirectory 'src\windows\Release\AutomaticScreenCamera.exe'
if (-not (Test-Path $sourceDll) -or -not (Test-Path $application)) {
    throw "Build outputs were not found under $BuildDirectory. Build the Release configuration first."
}

function Invoke-Registration([string]$DllPath, [switch]$Unregister, [switch]$AllowFailure) {
    $arguments = @('/s')
    if ($Unregister) { $arguments += '/u' }
    $arguments += ('"' + $DllPath + '"')
    $process = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0 -and -not $AllowFailure) {
        $operation = if ($Unregister) { 'unregistration' } else { 'registration' }
        throw "Media source $operation failed with exit code $($process.ExitCode)."
    }
}

function Stop-ApplicationProcess {
    $processes = @(Get-Process AutomaticScreenCamera -ErrorAction SilentlyContinue)
    if ($processes.Count -eq 0) { return }
    $processes | Stop-Process -Force
    foreach ($process in $processes) {
        if (-not $process.WaitForExit(10000)) { throw "Application process $($process.Id) did not exit within 10 seconds." }
    }
}

function Remove-NewInstallationForRollback([string]$Path, [string]$PreservedBackupPath) {
    if (Test-Path -LiteralPath $Path) {
        try { Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop }
        catch { throw "Rollback could not remove the new installation. The prior installation is preserved at '$PreservedBackupPath'. $_" }
    }
    if (Test-Path -LiteralPath $Path) {
        throw "Rollback destination still exists. The prior installation is preserved at '$PreservedBackupPath'."
    }
}

$installedApplication = Join-Path $installDirectory 'AutomaticScreenCamera.exe'
$installedSource = Join-Path $installDirectory 'AutomaticScreenCameraSource.dll'
$installedPreviously = Test-Path -LiteralPath $installDirectory -PathType Container
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$priorRunValue = (Get-ItemProperty -Path $runKey -Name AutomaticScreenCamera -ErrorAction SilentlyContinue).AutomaticScreenCamera
$deploymentMarkerChanged = $false

function Restore-DeploymentMarker {
    Remove-Item -LiteralPath $deploymentRegistryPath -Recurse -Force -ErrorAction SilentlyContinue
    if ($priorDeployment -and $priorDeployment.Mode) {
        New-Item -Path $deploymentRegistryPath -Force | Out-Null
        foreach ($name in @('Mode', 'Version', 'SourcePath')) {
            $value = $priorDeployment.$name
            if ($null -ne $value) {
                New-ItemProperty -LiteralPath $deploymentRegistryPath -Name $name -Value $value -PropertyType String -Force | Out-Null
            }
        }
    }
}

try {
    New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
    Copy-Item -LiteralPath $application -Destination (Join-Path $stagingDirectory 'AutomaticScreenCamera.exe')
    Copy-Item -LiteralPath $sourceDll -Destination (Join-Path $stagingDirectory 'AutomaticScreenCameraSource.dll')
    $applicationSourceHash = (Get-FileHash -LiteralPath $application -Algorithm SHA256).Hash
    $applicationStagedHash = (Get-FileHash -LiteralPath (Join-Path $stagingDirectory 'AutomaticScreenCamera.exe') -Algorithm SHA256).Hash
    if ($applicationSourceHash -ne $applicationStagedHash) {
        throw 'Staged application failed checksum verification.'
    }
    $sourceSourceHash = (Get-FileHash -LiteralPath $sourceDll -Algorithm SHA256).Hash
    $sourceStagedHash = (Get-FileHash -LiteralPath (Join-Path $stagingDirectory 'AutomaticScreenCameraSource.dll') -Algorithm SHA256).Hash
    if ($sourceSourceHash -ne $sourceStagedHash) {
        throw 'Staged media source failed checksum verification.'
    }
} catch {
    Remove-Item -LiteralPath $stagingDirectory -Recurse -Force -ErrorAction SilentlyContinue
    throw
}

$previousInstallationMoved = $false
$newInstallationPlaced = $false
$previousVirtualCameraRemoved = $false
try {
    Stop-ApplicationProcess
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
    if (Test-Path -LiteralPath $installedApplication -PathType Leaf) {
        $removal = Start-Process -FilePath $application -ArgumentList '--remove-virtual-camera-under-lock' -Wait -PassThru
        if ($removal.ExitCode -ne 0) {
            throw "Existing virtual-camera removal failed with exit code $($removal.ExitCode)."
        }
        $previousVirtualCameraRemoved = $true
    }
    if (Test-Path -LiteralPath $installedSource -PathType Leaf) {
        Invoke-Registration -DllPath $installedSource -Unregister -AllowFailure
    }

    if ($installedPreviously) {
        Move-Item -LiteralPath $installDirectory -Destination $backupDirectory
        $previousInstallationMoved = $true
    }
    Move-Item -LiteralPath $stagingDirectory -Destination $installDirectory
    $newInstallationPlaced = $true
    Invoke-Registration -DllPath $installedSource
} catch {
    $installError = $_
    if ($newInstallationPlaced -and (Test-Path -LiteralPath $installDirectory -PathType Container)) {
        Remove-NewInstallationForRollback -Path $installDirectory -PreservedBackupPath $backupDirectory
    }
    if ($previousInstallationMoved -and (Test-Path -LiteralPath $backupDirectory -PathType Container)) {
        Move-Item -LiteralPath $backupDirectory -Destination $installDirectory
    }
    if (Test-Path -LiteralPath $installedSource -PathType Leaf) {
        Invoke-Registration -DllPath $installedSource -AllowFailure
    }
    if ($installedPreviously -and $previousVirtualCameraRemoved -and
        (Test-Path -LiteralPath $installedApplication -PathType Leaf)) {
        try {
            $restoredApplication = Start-Process -FilePath $installedApplication -PassThru
            if ($restoredApplication.WaitForExit(3000)) {
                Write-Warning "The prior application exited while restoring its virtual camera (exit code $($restoredApplication.ExitCode))."
            }
        } catch {
            Write-Warning "The prior files and COM server were restored, but its virtual camera could not be restarted: $_"
        }
    }
    throw $installError
} finally {
    Remove-Item -LiteralPath $stagingDirectory -Recurse -Force -ErrorAction SilentlyContinue
}

$startMenu = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Automatic Screen Camera.lnk'
try {
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($startMenu)
    $shortcut.TargetPath = $installedApplication
    $shortcut.WorkingDirectory = $installDirectory
    $shortcut.Save()

    if ($StartWithWindows) {
        New-ItemProperty -Path $runKey -Name AutomaticScreenCamera -Value ('"' + $installedApplication + '"') -PropertyType String -Force | Out-Null
    } else {
        Remove-ItemProperty -Path $runKey -Name AutomaticScreenCamera -ErrorAction SilentlyContinue
    }

    $deploymentMarkerChanged = $true
    New-Item -Path $deploymentRegistryPath -Force | Out-Null
    New-ItemProperty -LiteralPath $deploymentRegistryPath -Name Mode -Value installed -PropertyType String -Force | Out-Null
    $installedVersion = (Get-Item -LiteralPath $installedApplication).VersionInfo.ProductVersion
    if (-not $installedVersion) { $installedVersion = 'development' }
    New-ItemProperty -LiteralPath $deploymentRegistryPath -Name Version -Value $installedVersion -PropertyType String -Force | Out-Null
    New-ItemProperty -LiteralPath $deploymentRegistryPath -Name SourcePath -Value $installedSource -PropertyType String -Force | Out-Null
} catch {
    $installError = $_
    if (Test-Path -LiteralPath $installedSource -PathType Leaf) {
        Invoke-Registration -DllPath $installedSource -Unregister -AllowFailure
    }
    Remove-NewInstallationForRollback -Path $installDirectory -PreservedBackupPath $backupDirectory
    if ($previousInstallationMoved -and (Test-Path -LiteralPath $backupDirectory -PathType Container)) {
        Move-Item -LiteralPath $backupDirectory -Destination $installDirectory
        if (Test-Path -LiteralPath $installedSource -PathType Leaf) {
            Invoke-Registration -DllPath $installedSource -AllowFailure
        }
    }
    if ($priorRunValue) {
        New-ItemProperty -Path $runKey -Name AutomaticScreenCamera -Value $priorRunValue -PropertyType String -Force | Out-Null
    } else {
        Remove-ItemProperty -Path $runKey -Name AutomaticScreenCamera -ErrorAction SilentlyContinue
    }
    if (-not $installedPreviously) { Remove-Item -LiteralPath $startMenu -Force -ErrorAction SilentlyContinue }
    if ($deploymentMarkerChanged) { Restore-DeploymentMarker }
    if ($installedPreviously -and $previousVirtualCameraRemoved -and
        (Test-Path -LiteralPath $installedApplication -PathType Leaf)) {
        try { Start-Process -FilePath $installedApplication | Out-Null }
        catch { Write-Warning "The prior install was restored but its virtual camera could not be restarted: $_" }
    }
    throw $installError
}

Remove-Item -LiteralPath $backupDirectory -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "Automatic Screen Camera installed to $installDirectory"
Write-Host 'Launch it from the Start menu. Windows camera privacy access must be enabled.'
} finally {
    if ($legacyApplicationMutexOwned) { $legacyApplicationMutex.ReleaseMutex() }
    if ($legacyApplicationMutex) { $legacyApplicationMutex.Dispose() }
    if ($machineApplicationMutexOwned) { $machineApplicationMutex.ReleaseMutex() }
    if ($machineApplicationMutex) { $machineApplicationMutex.Dispose() }
    if ($deploymentMutexOwned) { $deploymentMutex.ReleaseMutex() }
    $deploymentMutex.Dispose()
}
