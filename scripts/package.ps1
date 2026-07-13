[CmdletBinding()]
param(
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\out\build\windows-x64-release'),
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\out\package'),
    [string]$TestResultsDirectory,
    [string]$Version = '0.1.0',
    [string]$SourceRevision = $env:GITHUB_SHA
)

$ErrorActionPreference = 'Stop'
$resolvedSourceRevision = if ($SourceRevision) { $SourceRevision } else { 'unknown' }
$packageName = "AutomaticScreenCamera-$Version-windows-x64-unsigned"
$packageDirectory = Join-Path $OutputDirectory $packageName
$archive = Join-Path $OutputDirectory "$packageName.zip"

function Require-File([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required release artifact was not found: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

$applicationDirectory = Join-Path $BuildDirectory "src\windows\$Configuration"
$sourceDirectory = Join-Path $BuildDirectory "src\windows\media_source\$Configuration"
$artifacts = @(
    (Require-File (Join-Path $applicationDirectory 'AutomaticScreenCamera.exe')),
    (Require-File (Join-Path $applicationDirectory 'AutomaticScreenCamera.pdb')),
    (Require-File (Join-Path $sourceDirectory 'AutomaticScreenCameraSource.dll')),
    (Require-File (Join-Path $sourceDirectory 'AutomaticScreenCameraSource.pdb'))
)

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
if (Test-Path -LiteralPath $packageDirectory) {
    Remove-Item -LiteralPath $packageDirectory -Recurse -Force
}
if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}
New-Item -ItemType Directory -Path $packageDirectory | Out-Null
$resolvedPackageDirectory = (Resolve-Path -LiteralPath $packageDirectory).Path

foreach ($artifact in $artifacts) {
    Copy-Item -LiteralPath $artifact -Destination $packageDirectory
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install.ps1') -Destination $packageDirectory
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'uninstall.ps1') -Destination $packageDirectory
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\README.md') -Destination $packageDirectory

$releaseMetadata = [ordered]@{
    applicationVersion = $Version
    sourceRevision = $resolvedSourceRevision
    architecture = 'x64'
    configuration = $Configuration
    generator = 'Visual Studio 17 2022'
    windowsSdk = '10.0.22621.0'
    createdUtc = [DateTime]::UtcNow.ToString('o')
}
$releaseMetadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageDirectory 'release-metadata.json') -Encoding UTF8

if ($TestResultsDirectory) {
    if (-not (Test-Path -LiteralPath $TestResultsDirectory -PathType Container)) {
        throw "Test-results directory was not found: $TestResultsDirectory"
    }
    Copy-Item -LiteralPath $TestResultsDirectory -Destination (Join-Path $packageDirectory 'test-results') -Recurse
}

$hashLines = Get-ChildItem -LiteralPath $resolvedPackageDirectory -File -Recurse |
    Sort-Object FullName |
    ForEach-Object {
        $relativePath = $_.FullName.Substring($resolvedPackageDirectory.Length + 1).Replace('\', '/')
        $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash *$relativePath"
    }
$utf8NoBom = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText(
    (Join-Path $resolvedPackageDirectory 'SHA256SUMS.txt'),
    (($hashLines -join "`n") + "`n"),
    $utf8NoBom
)

Compress-Archive -LiteralPath $packageDirectory -DestinationPath $archive -CompressionLevel Optimal
$archiveHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$archive.sha256", "$archiveHash *$([IO.Path]::GetFileName($archive))`n", $utf8NoBom)

Write-Host "Packaged $archive"
