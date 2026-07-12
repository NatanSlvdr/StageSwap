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

$installDirectory = Join-Path $env:ProgramFiles 'Automatic Screen Camera'
$transactionId = [Guid]::NewGuid().ToString('N')
$stagingDirectory = Join-Path $env:ProgramFiles "Automatic Screen Camera.installing-$transactionId"
$backupDirectory = Join-Path $env:ProgramFiles "Automatic Screen Camera.backup-$transactionId"
$packagedSource = Join-Path $PSScriptRoot 'AutomaticScreenCameraSource.dll'
$packagedApplication = Join-Path $PSScriptRoot 'AutomaticScreenCamera.exe'
$usingPackagedArtifacts = (
    (Test-Path -LiteralPath $packagedSource -PathType Leaf) -and
    (Test-Path -LiteralPath $packagedApplication -PathType Leaf)
)
if ($usingPackagedArtifacts) {
    $sourceDll = $packagedSource
    $application = $packagedApplication
} else {
    $sourceDll = Join-Path $BuildDirectory 'src\windows\media_source\Release\AutomaticScreenCameraSource.dll'
    $application = Join-Path $BuildDirectory 'src\windows\Release\AutomaticScreenCamera.exe'
}
if (-not (Test-Path $sourceDll) -or -not (Test-Path $application)) {
    throw "Build outputs were not found under $BuildDirectory. Build the Release configuration first."
}

$checksumManifest = Join-Path $PSScriptRoot 'SHA256SUMS.txt'
if ($usingPackagedArtifacts -and -not (Test-Path -LiteralPath $checksumManifest -PathType Leaf)) {
    throw 'Packaged artifacts require SHA256SUMS.txt; refusing an unverifiable install.'
}
if ($usingPackagedArtifacts) {
    $manifestLines = Get-Content -LiteralPath $checksumManifest
    foreach ($artifact in @($application, $sourceDll)) {
        $fileName = [IO.Path]::GetFileName($artifact)
        $pattern = '^[0-9a-fA-F]{64} \*' + [Regex]::Escape($fileName) + '$'
        $entries = @($manifestLines | Where-Object { $_ -match $pattern })
        if ($entries.Count -ne 1) {
            throw "Checksum manifest does not contain exactly one entry for $fileName."
        }
        $expectedHash = ($entries[0] -split ' ')[0]
        $actualHash = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash
        if ($expectedHash -ne $actualHash) {
            throw "Package checksum verification failed for $fileName."
        }
    }
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
    if (Test-Path -LiteralPath $installedApplication -PathType Leaf) {
        $removal = Start-Process -FilePath $installedApplication -ArgumentList '--remove-virtual-camera' -Wait -PassThru
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
