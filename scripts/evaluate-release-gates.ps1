[CmdletBinding()]
param([Parameter(Mandatory)][string]$EvidenceManifest, [string]$OutputPath)
$ErrorActionPreference = 'Stop'
$manifest = Get-Content -LiteralPath $EvidenceManifest -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.schema_version -ne 2) { throw "Unsupported evidence manifest schema: $($manifest.schema_version)" }
$results = [Collections.Generic.List[object]]::new()
function Add-Gate([string]$Name, [bool]$Passed, [string]$Evidence) {
    $script:results.Add([ordered]@{ name = $Name; passed = $Passed; evidence = $Evidence })
}
foreach ($architecture in @('x64')) {
    $entry = $manifest.architectures.$architecture
    Add-Gate "$architecture Debug build and tests" ([bool]$entry.debug_tests_passed) "passed=$($entry.debug_tests_passed)"
    Add-Gate "$architecture Release build and tests" ([bool]$entry.release_tests_passed) "passed=$($entry.release_tests_passed)"
    Add-Gate "$architecture package validation" ([bool]$entry.package_validation_passed) "passed=$($entry.package_validation_passed)"
    Add-Gate "$architecture interactive Windows tests" ([bool]$entry.interactive_windows_tests_passed) "passed=$($entry.interactive_windows_tests_passed)"
    Add-Gate "$architecture Windows 11 VM acceptance" ([bool]$entry.vm_acceptance_passed) "passed=$($entry.vm_acceptance_passed)"
    Add-Gate "$architecture physical smoke test" ([bool]$entry.physical_smoke_passed) "passed=$($entry.physical_smoke_passed)"
}
Add-Gate 'C++ reference screenshots at 100% and 150% DPI captured' ([bool]$manifest.reference_screenshots_100_150_captured) "passed=$($manifest.reference_screenshots_100_150_captured)"
Add-Gate 'Rust UI visual comparison at 100% and 150% DPI' ([bool]$manifest.ui_visual_comparison_passed) "passed=$($manifest.ui_visual_comparison_passed)"
Add-Gate 'Windows Camera negotiation' ([bool]$manifest.windows_camera_formats_passed) "passed=$($manifest.windows_camera_formats_passed)"
Add-Gate 'Zoom negotiation' ([bool]$manifest.zoom_formats_passed) "passed=$($manifest.zoom_formats_passed)"
Add-Gate 'No unresolved retained-workflow failures' (@($manifest.unresolved_product_failures).Count -eq 0) "$(@($manifest.unresolved_product_failures).Count) unresolved"
$passed = @($results | Where-Object { -not $_.passed }).Count -eq 0
$report = [ordered]@{ schema_version = 2; evaluated_at_utc = [DateTime]::UtcNow.ToString('o'); passed = $passed; gates = $results }
$json = $report | ConvertTo-Json -Depth 6
if ($OutputPath) { [IO.File]::WriteAllText([IO.Path]::GetFullPath($OutputPath), $json, [Text.UTF8Encoding]::new($false)) }
$json
if (-not $passed) { exit 1 }
