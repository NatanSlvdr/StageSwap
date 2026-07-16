[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string]$Architecture = 'x64',
    [string]$BuildDirectory,
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\out\package'),
    [string]$Version = '0.1.0',
    [string]$SourceRevision = $env:GITHUB_SHA
)

$ErrorActionPreference = 'Stop'
if (-not $BuildDirectory) {
    $BuildDirectory = Join-Path $PSScriptRoot "..\out\build\windows-$Architecture-$($Configuration.ToLowerInvariant())"
}
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') {
    throw 'Version must be semantic, for example 1.2.3 or 1.2.3-beta.1.'
}
$resolvedSourceRevision = if ($SourceRevision) { $SourceRevision } else { 'unknown' }
$artifactName = "windows-$Architecture-portable.exe"
$buildArchitecture = if ($Architecture -eq 'arm64') { 'ARM64' } else { 'x64' }

function Require-File([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Required artifact not found: $Path" }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Get-PeMachine([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    try {
        $reader = [IO.BinaryReader]::new($stream)
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "'$Path' is not a PE file." }
        $stream.Position = 0x3c
        $peOffset = $reader.ReadUInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw "'$Path' has an invalid PE signature." }
        return $reader.ReadUInt16()
    } finally { $stream.Dispose() }
}

function Assert-NativeArchitecture([string]$Path) {
    $expected = if ($Architecture -eq 'arm64') { 0xaa64 } else { 0x8664 }
    $actual = Get-PeMachine $Path
    if ($actual -ne $expected) {
        throw ("Architecture mismatch for '{0}': expected {1} (0x{2:x4}), found 0x{3:x4}." -f $Path, $Architecture, $expected, $actual)
    }
}

$applicationDirectory = Join-Path $BuildDirectory "src\windows\$Configuration"
$sourceDirectory = Join-Path $BuildDirectory "src\windows\media_source\$Configuration"
$portable = Require-File (Join-Path $applicationDirectory "windows-$Architecture-portable.exe")
$mediaSource = Require-File (Join-Path $sourceDirectory 'AutomaticScreenCameraSource.dll')
Assert-NativeArchitecture $portable
Assert-NativeArchitecture $mediaSource

foreach ($binary in @($portable, $mediaSource)) {
    $productVersion = (Get-Item -LiteralPath $binary).VersionInfo.ProductVersion.Trim()
    if ($productVersion -ne $Version) { throw "ProductVersion '$productVersion' in '$binary' does not match '$Version'." }
    $comments = (Get-Item -LiteralPath $binary).VersionInfo.Comments
    if ($comments -notlike "*; $buildArchitecture; Windows SDK 10.0.22621.0*") {
        throw "Build metadata in '$binary' does not identify $buildArchitecture and SDK 10.0.22621.0."
    }
    if ($resolvedSourceRevision -ne 'unknown' -and $comments -notmatch [Regex]::Escape("Source revision $resolvedSourceRevision")) {
        throw "Source revision metadata in '$binary' is incorrect."
    }
}

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$output = Join-Path (Resolve-Path -LiteralPath $OutputDirectory).Path $artifactName
Copy-Item -LiteralPath $portable -Destination $output -Force
$hash = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
$metadata = @(
    "# applicationVersion=$Version",
    "# sourceRevision=$resolvedSourceRevision",
    "# architecture=$Architecture",
    "# configuration=$Configuration",
    '# windowsSdk=10.0.22621.0',
    "$hash *$artifactName"
) -join "`n"
[IO.File]::WriteAllText("$output.sha256", "$metadata`n", [Text.UTF8Encoding]::new($false))
Write-Host "Packaged $output"
