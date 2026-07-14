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

function Get-PeResourceSha256(
    [string]$Path,
    [UInt16]$TypeId,
    [UInt16]$NameId
) {
    if (-not ('AutomaticScreenCamera.PeResourceVerifier' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;

namespace AutomaticScreenCamera
{
    public static class PeResourceVerifier
    {
        private const uint LoadLibraryAsDataFileExclusive = 0x00000040;
        private const uint LoadLibraryAsImageResource = 0x00000020;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr LoadLibraryEx(string fileName, IntPtr file,
                                                   uint flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr FindResource(IntPtr module, IntPtr name,
                                                  IntPtr type);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern IntPtr LoadResource(IntPtr module, IntPtr resource);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint SizeofResource(IntPtr module, IntPtr resource);

        [DllImport("kernel32.dll")]
        private static extern IntPtr LockResource(IntPtr resourceData);

        [DllImport("kernel32.dll")]
        private static extern bool FreeLibrary(IntPtr module);

        private static Win32Exception LastError(string operation)
        {
            return new Win32Exception(Marshal.GetLastWin32Error(), operation);
        }

        public static string Sha256(string path, ushort typeId, ushort nameId)
        {
            IntPtr module = LoadLibraryEx(path, IntPtr.Zero,
                LoadLibraryAsDataFileExclusive | LoadLibraryAsImageResource);
            if (module == IntPtr.Zero)
                throw LastError("Map portable executable as a non-executable resource image");
            try
            {
                IntPtr resource = FindResource(module, new IntPtr(nameId),
                                               new IntPtr(typeId));
                if (resource == IntPtr.Zero)
                    throw LastError("Locate embedded portable payload resource");
                uint rawSize = SizeofResource(module, resource);
                if (rawSize == 0 || rawSize > Int32.MaxValue)
                    throw new InvalidDataException("Embedded portable payload has an invalid size.");
                IntPtr loaded = LoadResource(module, resource);
                if (loaded == IntPtr.Zero)
                    throw LastError("Load embedded portable payload resource");
                IntPtr data = LockResource(loaded);
                if (data == IntPtr.Zero)
                    throw new InvalidDataException("Embedded portable payload could not be locked.");

                byte[] payload = new byte[(int)rawSize];
                Marshal.Copy(data, payload, 0, payload.Length);
                using (SHA256 hash = SHA256.Create())
                {
                    byte[] digest = hash.ComputeHash(payload);
                    return BitConverter.ToString(digest).Replace("-", "").ToLowerInvariant();
                }
            }
            finally
            {
                FreeLibrary(module);
            }
        }
    }
}
'@
    }

    return [AutomaticScreenCamera.PeResourceVerifier]::Sha256(
        (Resolve-Path -LiteralPath $Path).Path,
        $TypeId,
        $NameId)
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
$embeddedPayloadHash = Get-PeResourceSha256 -Path $portableOutput -TypeId 10 -NameId 201
$mediaSourceHash = (Get-FileHash -LiteralPath $mediaSource -Algorithm SHA256).Hash.ToLowerInvariant()
if ($embeddedPayloadHash -ne $mediaSourceHash) {
    throw "Embedded portable payload SHA-256 '$embeddedPayloadHash' does not match '$mediaSourceHash' from '$mediaSource'."
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
