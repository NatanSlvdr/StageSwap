[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ArtifactPath,
    [string]$OutputPath = (Join-Path $PWD 'release-environment.json'),
    [string]$ZoomPath,
    [ValidateSet('stable', 'enterprise', 'other')]
    [string]$ZoomChannel = 'stable',
    [string]$VmImageId = $env:ASC_VM_IMAGE_ID,
    [string]$UsbPassthroughId = $env:ASC_USB_PASSTHROUGH_ID
)

$ErrorActionPreference = 'Stop'

$artifact = Get-Item -LiteralPath $ArtifactPath -ErrorAction Stop
$artifactHash = Get-FileHash -LiteralPath $artifact.FullName -Algorithm SHA256
$artifactVersion = $artifact.VersionInfo.FileVersion
$artifactSourceRevision = $null
$artifactConfiguration = $null
$artifactArchitecture = $null
$artifactWindowsSdk = $null
if ($artifact.Extension -eq '.zip') {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($artifact.FullName)
    try {
        $metadataEntry = $archive.Entries | Where-Object { $_.FullName -match '(^|/)release-metadata\.json$' } | Select-Object -First 1
        if ($metadataEntry) {
            $reader = [System.IO.StreamReader]::new($metadataEntry.Open())
            try { $packageMetadata = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
            $artifactVersion = $packageMetadata.applicationVersion
            $artifactSourceRevision = $packageMetadata.sourceRevision
            $artifactConfiguration = $packageMetadata.configuration
            $artifactArchitecture = $packageMetadata.architecture
            $artifactWindowsSdk = $packageMetadata.windowsSdk
        }
    } finally {
        $archive.Dispose()
    }
    if (-not $metadataEntry) { throw 'Release ZIP does not contain release-metadata.json.' }
    $externalHashPath = $artifact.FullName + '.sha256'
    if (-not (Test-Path -LiteralPath $externalHashPath -PathType Leaf)) {
        throw "External archive checksum is missing: $externalHashPath"
    }
    $externalHashPattern = '^([0-9a-fA-F]{64}) \*' + [Regex]::Escape($artifact.Name) + '$'
    $externalHashLines = @(Get-Content -LiteralPath $externalHashPath -Encoding UTF8 | Where-Object { $_ -match $externalHashPattern })
    $externalHashMatch = if ($externalHashLines.Count -eq 1) { [Regex]::Match($externalHashLines[0], $externalHashPattern) } else { $null }
    if (-not $externalHashMatch -or -not $externalHashMatch.Success -or
        $externalHashMatch.Groups[1].Value -ne $artifactHash.Hash) {
        throw 'Release ZIP does not match its external SHA-256 checksum.'
    }
}

if (-not $ZoomPath) {
    $zoomCandidates = @(
        (Join-Path $env:APPDATA 'Zoom\bin\Zoom.exe'),
        (Join-Path $env:ProgramFiles 'Zoom\bin\Zoom.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Zoom\bin\Zoom.exe')
    )
    $ZoomPath = $zoomCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
}
$zoom = if ($ZoomPath -and (Test-Path -LiteralPath $ZoomPath)) { Get-Item -LiteralPath $ZoomPath } else { $null }

$operatingSystem = Get-CimInstance Win32_OperatingSystem
$computerSystem = Get-CimInstance Win32_ComputerSystem
$processors = @(Get-CimInstance Win32_Processor)
$installedUpdates = @(Get-HotFix -ErrorAction SilentlyContinue | Sort-Object HotFixID | ForEach-Object {
    [ordered]@{
        hotfix_id = $_.HotFixID
        installed_on = if ($_.InstalledOn) { $_.InstalledOn.ToString('yyyy-MM-dd') } else { $null }
    }
})
$videoControllers = @(Get-CimInstance Win32_VideoController | ForEach-Object {
    [ordered]@{
        name = $_.Name
        driver_version = $_.DriverVersion
        pnp_device_id = $_.PNPDeviceID
    }
})
$webcams = @(Get-CimInstance Win32_PnPSignedDriver | Where-Object {
    $_.DeviceClass -in @('CAMERA', 'IMAGE') -or $_.DeviceName -match '(?i)camera|webcam'
} | ForEach-Object {
    [ordered]@{
        name = $_.DeviceName
        device_id = $_.DeviceID
        manufacturer = $_.Manufacturer
        driver_provider = $_.DriverProviderName
        driver_version = $_.DriverVersion
        driver_date = if ($_.DriverDate) { $_.DriverDate.ToString('o') } else { $null }
    }
})

$visualStudio = $null
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path -LiteralPath $vswhere) {
    $vsJson = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json
    if ($LASTEXITCODE -eq 0 -and $vsJson) {
        $visualStudio = @($vsJson | ConvertFrom-Json) | Select-Object -First 1
    }
}

$sdkRoot = $null
try {
    $sdkRoot = (Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots').KitsRoot10
} catch {
    $sdkRoot = $null
}
$sdkVersions = @()
if ($sdkRoot) {
    $includeRoot = Join-Path $sdkRoot 'Include'
    if (Test-Path -LiteralPath $includeRoot) {
        $sdkVersions = @(Get-ChildItem -LiteralPath $includeRoot -Directory | Select-Object -ExpandProperty Name | Sort-Object)
    }
}

$checkoutCommit = $null
if (Get-Command git -ErrorAction SilentlyContinue) {
    $checkoutCommit = (& git -C (Split-Path -Parent $PSScriptRoot) rev-parse HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) { $checkoutCommit = $null }
}
$gitCommit = $checkoutCommit
if ($artifactSourceRevision -and $artifactSourceRevision -ne 'unknown') {
    if ($checkoutCommit -and $checkoutCommit -ne $artifactSourceRevision) {
        throw "Artifact revision $artifactSourceRevision does not match checkout revision $checkoutCommit."
    }
    $gitCommit = $artifactSourceRevision
}

$environment = [ordered]@{
    schema_version = 1
    captured_at_utc = [DateTime]::UtcNow.ToString('o')
    git_commit = $gitCommit
    artifact = [ordered]@{
        path = $artifact.FullName
        filename = $artifact.Name
        version = $artifactVersion
        source_revision = $artifactSourceRevision
        configuration = $artifactConfiguration
        architecture = $artifactArchitecture
        windows_sdk = $artifactWindowsSdk
        size_bytes = $artifact.Length
        sha256 = $artifactHash.Hash.ToLowerInvariant()
    }
    windows = [ordered]@{
        caption = $operatingSystem.Caption
        version = $operatingSystem.Version
        build_number = $operatingSystem.BuildNumber
        architecture = $operatingSystem.OSArchitecture
        install_date = if ($operatingSystem.InstallDate) { $operatingSystem.InstallDate.ToString('o') } else { $null }
        installed_updates = $installedUpdates
    }
    zoom = [ordered]@{
        found = [bool]$zoom
        path = if ($zoom) { $zoom.FullName } else { $null }
        channel = $ZoomChannel
        file_version = if ($zoom) { $zoom.VersionInfo.FileVersion } else { $null }
        product_version = if ($zoom) { $zoom.VersionInfo.ProductVersion } else { $null }
    }
    machine = [ordered]@{
        manufacturer = $computerSystem.Manufacturer
        model = $computerSystem.Model
        logical_processors = ($processors | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum
        total_memory_bytes = [uint64]$computerSystem.TotalPhysicalMemory
        hypervisor_present = [bool]$computerSystem.HypervisorPresent
        vm_image_id = $VmImageId
        usb_passthrough_id = $UsbPassthroughId
    }
    visual_studio = if ($visualStudio) {
        [ordered]@{
            display_name = $visualStudio.displayName
            version = $visualStudio.installationVersion
            path = $visualStudio.installationPath
        }
    } else { $null }
    windows_sdk = [ordered]@{
        root = $sdkRoot
        installed_versions = $sdkVersions
        required_version_present = $sdkVersions -contains '10.0.22621.0'
    }
    webcams = $webcams
    video_controllers = $videoControllers
}

$parent = Split-Path -Parent ([System.IO.Path]::GetFullPath($OutputPath))
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
$environmentJson = $environment | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($OutputPath), $environmentJson, [System.Text.UTF8Encoding]::new($false))
Write-Host "Release environment written to $OutputPath"
if (-not $zoom) { Write-Warning 'Zoom was not found; the release environment is incomplete.' }
if ($webcams.Count -eq 0) { Write-Warning 'No physical webcam driver was found; the release environment is incomplete.' }
if (-not $VmImageId) { Write-Warning 'VM image ID is missing; pass -VmImageId or set ASC_VM_IMAGE_ID.' }
if (-not $UsbPassthroughId) { Write-Warning 'USB passthrough ID is missing; pass -UsbPassthroughId or set ASC_USB_PASSTHROUGH_ID.' }
