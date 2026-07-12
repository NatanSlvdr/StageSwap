[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$EvidenceManifest,
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'

function Add-GateResult {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][bool]$Passed,
        [Parameter(Mandatory)][string]$Evidence
    )
    $script:results.Add([ordered]@{ name = $Name; passed = $Passed; evidence = $Evidence })
}

function Test-TrueBoolean($Value) {
    return $Value -is [bool] -and $Value
}

function Test-JsonInteger($Value) {
    return $Value -is [sbyte] -or $Value -is [byte] -or $Value -is [int16] -or
        $Value -is [uint16] -or $Value -is [int32] -or $Value -is [uint32] -or
        $Value -is [int64] -or $Value -is [uint64]
}

function Test-NonNegativeInteger($Value) {
    return (Test-JsonInteger $Value) -and $Value -ge 0
}

function Test-FiniteNumber($Value) {
    if (Test-JsonInteger $Value) { return $true }
    if ($Value -is [decimal]) { return $true }
    if ($Value -is [single] -or $Value -is [double]) {
        return -not [double]::IsNaN([double]$Value) -and -not [double]::IsInfinity([double]$Value)
    }
    return $false
}

function Add-CountGate([string]$Name, $Value, [int64]$Required) {
    $valid = Test-NonNegativeInteger $Value
    Add-GateResult $Name ($valid -and $Value -ge $Required) "$Value / $Required"
}

function Convert-EvidenceDate($Value) {
    if (-not ($Value -is [string]) -or -not $Value) { return $null }
    try {
        $parsed = [DateTimeOffset]::ParseExact(
            $Value, 'o', [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::None)
        if ($parsed.Offset -eq [TimeSpan]::Zero) { return $parsed }
        return $null
    }
    catch { return $null }
}

$manifestPath = (Resolve-Path -LiteralPath $EvidenceManifest).Path
$manifestDirectory = Split-Path -Parent $manifestPath
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.schema_version -ne 1) { throw "Unsupported evidence manifest schema: $($manifest.schema_version)" }

$results = [System.Collections.Generic.List[object]]::new()
Add-CountGate 'Five consecutive clean hosted builds' $manifest.hosted_clean_builds 5
Add-CountGate 'Install/uninstall or upgrade cycles' $manifest.install_cycles 20
Add-CountGate 'Application cold-start/exit cycles' $manifest.application_cycles 100
Add-CountGate 'Virtual-camera open/close cycles' $manifest.virtual_camera_cycles 100
Add-CountGate 'Automatic camera/screen switches' $manifest.automatic_switches 300
Add-CountGate 'Webcam disconnect/reconnect cycles' $manifest.webcam_reconnect_cycles 50
Add-CountGate 'Lifecycle recovery cycles' $manifest.lifecycle_recovery_cycles 20
Add-CountGate 'Zoom cold launches' $manifest.zoom_cold_launches 30
$failurePropertyPresent = $null -ne $manifest.PSObject.Properties['unresolved_product_failures']
$failureArrayPresent = $failurePropertyPresent -and $manifest.unresolved_product_failures -is [System.Array]
Add-GateResult 'No unresolved product failures' ($failureArrayPresent -and $manifest.unresolved_product_failures.Count -eq 0) "$(@($manifest.unresolved_product_failures).Count) unresolved"
$environmentPath = if ($manifest.environment_report -and [System.IO.Path]::IsPathRooted($manifest.environment_report)) {
    $manifest.environment_report
} elseif ($manifest.environment_report) {
    Join-Path $manifestDirectory $manifest.environment_report
} else { $null }
$environmentExists = $environmentPath -and (Test-Path -LiteralPath $environmentPath)
$environment = if ($environmentExists) { Get-Content -LiteralPath $environmentPath -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
$environmentComplete = $environment -and $environment.git_commit -and $environment.artifact.version -and
    $environment.artifact.source_revision -and $environment.git_commit -eq $environment.artifact.source_revision -and
    $environment.artifact.configuration -eq 'Release' -and $environment.artifact.architecture -eq 'x64' -and
    $environment.artifact.windows_sdk -eq '10.0.22621.0' -and $environment.artifact.sha256 -and
    $environment.windows.build_number -and (Test-TrueBoolean $environment.zoom.found) -and $environment.zoom.channel -and
    @($environment.webcams).Count -gt 0 -and (Test-TrueBoolean $environment.windows_sdk.required_version_present) -and
    $environment.machine.vm_image_id -and $environment.machine.usb_passthrough_id
Add-GateResult 'Complete environment report attached' ([bool]$environmentComplete) ([string]$environmentPath)

$soaks = @($manifest.soaks)
Add-GateResult 'Two consecutive 24-hour Zoom soaks recorded' ($soaks.Count -ge 2) "$($soaks.Count) / 2"
$seenProbePaths = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$seenRunIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$soakPeriods = [System.Collections.Generic.List[object]]::new()
for ($index = 0; $index -lt [Math]::Min(2, $soaks.Count); $index++) {
    $soak = $soaks[$index]
    $probePath = if ([System.IO.Path]::IsPathRooted($soak.probe_json)) {
        $soak.probe_json
    } else {
        Join-Path $manifestDirectory $soak.probe_json
    }
    $probePath = [System.IO.Path]::GetFullPath($probePath)
    $probeUnique = $seenProbePaths.Add($probePath)
    $probeExists = Test-Path -LiteralPath $probePath
    $probe = if ($probeExists) { Get-Content -LiteralPath $probePath -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
    $prefix = "Soak $($index + 1)"
    Add-GateResult "$prefix probe evidence exists" $probeExists $probePath
    Add-GateResult "$prefix probe evidence is distinct" $probeUnique $probePath
    $runIdValid = $soak.run_id -is [string] -and [bool]$soak.run_id -and $seenRunIds.Add($soak.run_id)
    Add-GateResult "$prefix run ID is distinct" $runIdValid ([string]$soak.run_id)
    $startedAt = Convert-EvidenceDate $soak.started_at_utc
    $completedAt = Convert-EvidenceDate $soak.completed_at_utc
    $periodValid = $startedAt -and $completedAt -and $completedAt -gt $startedAt
    Add-GateResult "$prefix timestamps are valid" ([bool]$periodValid) "$startedAt .. $completedAt"
    if ($periodValid) { $soakPeriods.Add([ordered]@{ started = $startedAt; completed = $completedAt }) }
    $artifactBound = $environment -and $soak.artifact_sha256 -is [string] -and
        $soak.artifact_sha256 -eq $environment.artifact.sha256
    Add-GateResult "$prefix artifact binding" ([bool]$artifactBound) ([string]$soak.artifact_sha256)
    if ($probe) {
        Add-GateResult "$prefix probe verdict" (Test-TrueBoolean $probe.passed) "passed=$($probe.passed)"
        $durationValid = Test-FiniteNumber $probe.duration_seconds
        $fpsValid = Test-FiniteNumber $probe.measured_fps
        Add-GateResult "$prefix duration" ($durationValid -and $probe.duration_seconds -ge 86400) "$($probe.duration_seconds) seconds"
        Add-GateResult "$prefix cadence" ($fpsValid -and $probe.measured_fps -ge 29 -and $probe.measured_fps -le 31) "$($probe.measured_fps) fps"
        $formatAccepted = (Test-NonNegativeInteger $probe.media_type.width) -and
            (Test-NonNegativeInteger $probe.media_type.height) -and
            $probe.media_type.subtype_name -in @('RGB32', 'NV12') -and
            (($probe.media_type.width -eq 1920 -and $probe.media_type.height -eq 1080) -or
             ($probe.media_type.width -eq 1280 -and $probe.media_type.height -eq 720))
        Add-GateResult "$prefix negotiated media type" $formatAccepted "$($probe.media_type.subtype_name) $($probe.media_type.width)x$($probe.media_type.height)"
        $staleValid = Test-FiniteNumber $probe.stale_frame_duration_ms
        Add-GateResult "$prefix stale-frame duration" ($staleValid -and $probe.stale_frame_duration_ms -ge 0 -and $probe.stale_frame_duration_ms -le 2000) "$($probe.stale_frame_duration_ms) ms"
        $timestampIntegrity = (Test-NonNegativeInteger $probe.timestamp_regressions) -and
            (Test-NonNegativeInteger $probe.read_failures) -and
            $probe.timestamp_regressions -eq 0 -and $probe.read_failures -eq 0
        Add-GateResult "$prefix timestamp/read integrity" $timestampIntegrity "regressions=$($probe.timestamp_regressions), failures=$($probe.read_failures)"
        $warmupValid = Test-NonNegativeInteger $probe.producer_process.warmup_seconds
        $requiredCoverage = if ($durationValid -and $warmupValid) { [Math]::Max(0, $probe.duration_seconds - $probe.producer_process.warmup_seconds - 5) } else { 0 }
        $producerFound = Test-TrueBoolean $probe.producer_process.found
        $coverageValid = Test-FiniteNumber $probe.producer_process.measurement_coverage_seconds
        $averageCpuValid = Test-FiniteNumber $probe.producer_process.average_cpu_percent
        $p95CpuValid = Test-FiniteNumber $probe.producer_process.p95_cpu_percent
        $memoryValid = Test-NonNegativeInteger $probe.producer_process.private_memory_growth_bytes
        $handleValid = Test-JsonInteger $probe.producer_process.handle_growth
        Add-GateResult "$prefix producer metric coverage" ($producerFound -and $durationValid -and $warmupValid -and $coverageValid -and $probe.producer_process.measurement_coverage_seconds -ge $requiredCoverage) "$($probe.producer_process.measurement_coverage_seconds) / $requiredCoverage seconds"
        Add-GateResult "$prefix average CPU" ($producerFound -and $averageCpuValid -and $probe.producer_process.average_cpu_percent -ge 0 -and $probe.producer_process.average_cpu_percent -le 25) "$($probe.producer_process.average_cpu_percent)%"
        Add-GateResult "$prefix p95 CPU" ($producerFound -and $p95CpuValid -and $probe.producer_process.p95_cpu_percent -ge 0 -and $probe.producer_process.p95_cpu_percent -le 50) "$($probe.producer_process.p95_cpu_percent)%"
        Add-GateResult "$prefix private-memory growth" ($producerFound -and $memoryValid -and $probe.producer_process.private_memory_growth_bytes -lt 52428800) "$($probe.producer_process.private_memory_growth_bytes) bytes"
        Add-GateResult "$prefix handle growth" ($producerFound -and $handleValid -and $probe.producer_process.handle_growth -lt 20) "$($probe.producer_process.handle_growth) handles"
    }
    Add-GateResult "$prefix Zoom/UI verification" (Test-TrueBoolean $soak.zoom_expected_frames_verified) "verified=$($soak.zoom_expected_frames_verified)"
    Add-GateResult "$prefix no crash, hang, deadlock, or device loss" (Test-TrueBoolean $soak.no_product_failure) "verified=$($soak.no_product_failure)"
    Add-GateResult "$prefix privacy/stale-frame review" (Test-TrueBoolean $soak.privacy_review_passed) "verified=$($soak.privacy_review_passed)"
}
Add-GateResult 'Soaks are sequential and non-overlapping' ($soakPeriods.Count -eq 2 -and $soakPeriods[0].completed -le $soakPeriods[1].started) "$($soakPeriods.Count) valid periods"

$passed = @($results | Where-Object { -not $_.passed }).Count -eq 0
$report = [ordered]@{
    schema_version = 1
    evaluated_at_utc = [DateTime]::UtcNow.ToString('o')
    passed = $passed
    confidence_claim_allowed = $passed
    manifest = $manifestPath
    gates = $results
}
$json = $report | ConvertTo-Json -Depth 8
if ($OutputPath) {
    $parent = Split-Path -Parent ([System.IO.Path]::GetFullPath($OutputPath))
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($OutputPath), $json, [System.Text.UTF8Encoding]::new($false))
}
$json
if (-not $passed) { exit 1 }
