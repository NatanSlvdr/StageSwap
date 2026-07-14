[CmdletBinding()]
param(
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\out\build\windows-x64-release'),
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\out\package'),
    [string]$Version = '0.1.0',
    [string]$SourceRevision = $env:GITHUB_SHA,
    [string]$IsccPath = $env:ISCC_PATH
)

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^(?<major>\d+)\.(?<minor>\d+)\.(?<patch>\d+)(?:-[0-9A-Za-z.-]+)?$') {
    throw 'Version must be semantic, for example 1.2.3 or 1.2.3-beta.1.'
}
$numericVersion = "$($Matches.major).$($Matches.minor).$($Matches.patch).0"
$resolvedSourceRevision = if ($SourceRevision) { $SourceRevision } else { 'unknown' }
$baseName = "AutomaticScreenCamera-$Version-windows-x64"
$portableName = "$baseName-portable.exe"
$setupName = "$baseName-setup.exe"

function Require-File([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required release artifact was not found: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Resolve-Iscc([string]$RequestedPath) {
    if ($RequestedPath) { return (Require-File $RequestedPath) }
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
    ) | Where-Object { $_ }
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return (Resolve-Path -LiteralPath $candidate).Path }
    }
    throw 'Inno Setup 6 compiler was not found. Set ISCC_PATH to ISCC.exe.'
}

function Write-ArtifactChecksum([string]$ArtifactPath) {
    $artifact = Get-Item -LiteralPath $ArtifactPath
    $hash = (Get-FileHash -LiteralPath $artifact.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $lines = @(
        "# applicationVersion=$Version",
        "# sourceRevision=$resolvedSourceRevision",
        '# architecture=x64',
        "# configuration=$Configuration",
        '# windowsSdk=10.0.22621.0',
        "$hash *$($artifact.Name)"
    )
    $utf8NoBom = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText("$($artifact.FullName).sha256", (($lines -join "`n") + "`n"), $utf8NoBom)
}

$applicationDirectory = Join-Path $BuildDirectory "src\windows\$Configuration"
$sourceDirectory = Join-Path $BuildDirectory "src\windows\media_source\$Configuration"
$portableSource = Require-File (Join-Path $applicationDirectory 'AutomaticScreenCameraPortable.exe')
$applicationSource = Require-File (Join-Path $applicationDirectory 'AutomaticScreenCamera.exe')
$mediaSource = Require-File (Join-Path $sourceDirectory 'AutomaticScreenCameraSource.dll')
$iscc = Resolve-Iscc $IsccPath
$installerScript = Require-File (Join-Path $PSScriptRoot '..\installer\AutomaticScreenCamera.iss')

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$resolvedOutput = (Resolve-Path -LiteralPath $OutputDirectory).Path
$portableOutput = Join-Path $resolvedOutput $portableName
$setupOutput = Join-Path $resolvedOutput $setupName
Get-ChildItem -LiteralPath $resolvedOutput -File |
    Where-Object { $_.Name -like "$baseName-*" } |
    Remove-Item -Force

Copy-Item -LiteralPath $portableSource -Destination $portableOutput
$portableProcess = Start-Process -FilePath $portableOutput -ArgumentList '--verify-portable-payload' -Wait -PassThru
if ($portableProcess.ExitCode -ne 0) {
    throw "Portable payload verification failed with exit code $($portableProcess.ExitCode)."
}

$resolvedBuild = (Resolve-Path -LiteralPath $BuildDirectory).Path
$innoEnvironment = @{
    ASC_INNO_APP_VERSION = $Version
    ASC_INNO_NUMERIC_VERSION = $numericVersion
    ASC_INNO_BUILD_DIRECTORY = $resolvedBuild
    ASC_INNO_BUILD_CONFIGURATION = $Configuration
    ASC_INNO_OUTPUT_DIRECTORY = $resolvedOutput
    ASC_INNO_SETUP_BASENAME = [IO.Path]::GetFileNameWithoutExtension($setupName)
}
$priorInnoEnvironment = @{}
foreach ($entry in $innoEnvironment.GetEnumerator()) {
    $priorInnoEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, 'Process')
    [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process')
}
try {
    & $iscc $installerScript
    if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE." }
} finally {
    foreach ($entry in $priorInnoEnvironment.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process')
    }
}
$null = Require-File $setupOutput

$portableVersion = (Get-Item -LiteralPath $portableOutput).VersionInfo.ProductVersion
$setupVersion = (Get-Item -LiteralPath $setupOutput).VersionInfo.ProductVersion
if ($portableVersion -ne $Version) { throw "Portable ProductVersion '$portableVersion' does not match '$Version'. Reconfigure CMake with ASC_PACKAGE_VERSION." }
if ($setupVersion -ne $Version) { throw "Setup ProductVersion '$setupVersion' does not match '$Version'." }
foreach ($binary in @($applicationSource, $mediaSource)) {
    $binaryVersion = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion
    if ($binaryVersion -ne $Version) { throw "ProductVersion '$binaryVersion' in '$binary' does not match '$Version'." }
}
foreach ($binary in @($portableOutput, $applicationSource, $mediaSource)) {
    $comments = (Get-Item -LiteralPath $binary).VersionInfo.Comments
    $expectedBuildMetadata = "; $Configuration; x64; Windows SDK 10.0.22621.0"
    if ($comments -notlike "*$expectedBuildMetadata*") {
        throw "Build metadata in '$binary' does not identify $Configuration x64 with Windows SDK 10.0.22621.0."
    }
}
if ($resolvedSourceRevision -ne 'unknown') {
    foreach ($binary in @($portableOutput, $applicationSource, $mediaSource)) {
        $comments = (Get-Item -LiteralPath $binary).VersionInfo.Comments
        if ($comments -notmatch ([Regex]::Escape("Source revision $resolvedSourceRevision"))) {
            throw "Source revision metadata in '$binary' does not match '$resolvedSourceRevision'. Reconfigure CMake with ASC_SOURCE_REVISION."
        }
    }
}

Write-ArtifactChecksum $portableOutput
Write-ArtifactChecksum $setupOutput

$expectedNames = @($portableName, "$portableName.sha256", $setupName, "$setupName.sha256") | Sort-Object
$actualNames = @(Get-ChildItem -LiteralPath $resolvedOutput -File | Where-Object { $_.Name -like "$baseName-*" } | Select-Object -ExpandProperty Name | Sort-Object)
if (Compare-Object $expectedNames $actualNames) {
    throw "Package output contains unexpected files for version ${Version}: $($actualNames -join ', ')"
}

Write-Host "Packaged $portableOutput"
Write-Host "Packaged $setupOutput"
