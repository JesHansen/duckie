# Runs the benchmark protocol from ARCHITECTURE.md against release binaries.
# Requires a build with the bench feature:
#   cargo build --workspace --release --features duckie-desktop/bench
param(
    [int]$WarmTrials = 30,
    [int]$ColdTrials = 20,
    [string]$Collection = 'fixtures/collection-1k',
    [int]$LargeBytes = 50MB,
    [string]$Out = 'artifacts/benchmark.json'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    $exe = Join-Path $root 'target/release/duckie.exe'
    if (-not (Test-Path $exe)) { throw "$exe is missing. Build with --features duckie-desktop/bench first." }
    $image = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($exe))
    if (-not $image.Contains('DUCKIE_BENCH_PATH')) {
        throw "$exe was built without the bench feature; it cannot report milestones."
    }
    $scratch = Join-Path ([System.IO.Path]::GetTempPath()) ("duckie-bench-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Force -Path $scratch | Out-Null

    # Each trial runs a fresh process. The app writes epoch milestones; process creation time
    # comes from the OS, so loader cost before main() is included rather than silently dropped.
    function Invoke-Trial([string[]]$AppArgs, [hashtable]$Env) {
        $report = Join-Path $scratch ("trial-" + [guid]::NewGuid() + ".json")
        $previous = @{}
        $Env['DUCKIE_BENCH_PATH'] = $report
        foreach ($key in $Env.Keys) {
            $previous[$key] = [Environment]::GetEnvironmentVariable($key)
            [Environment]::SetEnvironmentVariable($key, $Env[$key])
        }
        try {
            $process = Start-Process -FilePath $exe -ArgumentList $AppArgs -PassThru
            $started = $process.StartTime
            if (-not $process.WaitForExit(120000)) {
                $process.Kill(); $process.WaitForExit()
                throw 'Trial did not exit within 120 s'
            }
            if (-not (Test-Path $report)) { throw 'Trial wrote no report' }
            $r = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
            # [DateTimeOffset]::new honours the DateTime's Kind. Do NOT subtract a
            # [datetime]'1970-01-01T00:00:00Z' literal: PowerShell casts it to local time, which
            # silently shifts every measurement by the UTC offset.
            $epoch = [DateTimeOffset]::new($started).ToUnixTimeMilliseconds()
            [pscustomobject]@{
                FirstFrameMs = if ($r.firstFrameEpochMs) { $r.firstFrameEpochMs - $epoch } else { $null }
                SearchableMs = if ($r.searchableEpochMs) { $r.searchableEpochMs - $epoch } else { $null }
                RespondedMs  = if ($r.respondedEpochMs) { $r.respondedEpochMs - $epoch } else { $null }
                Requests     = $r.requests
                SettledBytes = $r.settledPrivateBytes
                PeakBytes    = $r.peakPrivateBytes
                PeakWsBytes  = $r.peakWorkingSetBytes
            }
        } finally {
            foreach ($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key]) }
        }
    }
    # Nearest-rank p95: with 20-30 trials an interpolating percentile would invent a value
    # between samples, and the gates are stated against observed launches.
    function Get-P95($values) {
        $sorted = @($values | Where-Object { $null -ne $_ } | Sort-Object)
        if ($sorted.Count -eq 0) { return $null }
        $sorted[[Math]::Min($sorted.Count - 1, [Math]::Ceiling(0.95 * $sorted.Count) - 1)]
    }
    function Summarize($name, $values, $target) {
        [pscustomobject]@{
            Measurement = $name
            Trials      = @($values | Where-Object { $null -ne $_ }).Count
            MinMs       = ($values | Measure-Object -Minimum).Minimum
            MedianMs    = (@($values | Sort-Object))[[int](@($values).Count / 2)]
            P95Ms       = Get-P95 $values
            TargetMs    = $target
        }
    }

    Write-Host "Warm launch, empty workspace ($WarmTrials trials)"
    $warm = 1..$WarmTrials | ForEach-Object { (Invoke-Trial @() @{}).FirstFrameMs }

    Write-Host "Restored collection, $Collection ($WarmTrials trials)"
    $restored = @()
    $restoredFirst = @()
    if (Test-Path $Collection) {
        $full = (Resolve-Path $Collection).Path
        1..$WarmTrials | ForEach-Object {
            $t = Invoke-Trial @($full) @{}
            $restored += $t.SearchableMs
            $restoredFirst += $t.FirstFrameMs
        }
    } else {
        Write-Warning "$Collection not found; run scripts/make-fixtures.mjs. Skipping restore measurement."
    }

    # The 50 MiB budget is a delta over the settled baseline, so both come from one process.
    Write-Host "50 MiB response, tests disabled"
    $port = 8799
    $server = Start-Process -FilePath 'node.exe' -ArgumentList (Join-Path $root 'scripts/dev-server.mjs') `
        -PassThru -WindowStyle Hidden
    $large = $null
    try {
        $deadline = (Get-Date).AddSeconds(15)
        while ((Get-Date) -lt $deadline) {
            try { (New-Object Net.Sockets.TcpClient('127.0.0.1', 8787)).Close(); break } catch { Start-Sleep -Milliseconds 50 }
        }
        $large = Invoke-Trial @() @{ 'DUCKIE_BENCH_URL' = "http://127.0.0.1:8787/large?bytes=$LargeBytes" }
    } finally {
        if (-not $server.HasExited) { $server.Kill(); $server.WaitForExit() }
    }
    $port | Out-Null

    $result = [pscustomobject]@{
        CapturedUtc   = (Get-Date).ToUniversalTime().ToString('o')
        Machine       = [pscustomobject]@{
            OS        = (Get-CimInstance Win32_OperatingSystem).Caption + ' ' + [Environment]::OSVersion.Version
            CPU       = (Get-CimInstance Win32_Processor).Name
            Cores     = [Environment]::ProcessorCount
            RAMBytes  = (Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
            GPU       = (Get-CimInstance Win32_VideoController | Select-Object -First 1).Name
            PowerPlan = (powercfg /getactivescheme)
            Renderer  = 'glow'
        }
        Launch        = Summarize 'Warm launch to first frame' $warm 300
        RestoreFirst  = Summarize 'Restored collection, first frame' $restoredFirst 300
        Restore       = Summarize 'Restored collection searchable' $restored 500
        LargeResponse = $large
        Notes         = @(
            "First-frame milestones are recorded inside the app at the end of its first frame; they exclude compositor present time, so treat them as a lower bound.",
            "Cold launch ($ColdTrials trials) is NOT measured here: it needs controlled cache eviction between trials.",
            "Input-to-paint latency and GPU allocations are NOT measured here.",
            "Peak private bytes come from the kernel's PeakPagefileUsage for the GUI process only; test workers are separate processes and are not included."
        )
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent (Join-Path $root $Out)) | Out-Null
    $result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $root $Out) -Encoding utf8
    Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
    $result.Launch, $result.RestoreFirst, $result.Restore | Format-Table -AutoSize
    if ($large) {
        "50 MiB response: settled {0:N0} B, peak {1:N0} B, delta {2:N2} MiB, elapsed {3} ms" -f `
            $large.SettledBytes, $large.PeakBytes, (($large.PeakBytes - $large.SettledBytes) / 1MB), $large.RespondedMs
    }
    "Written to $Out"
} finally { Pop-Location }
