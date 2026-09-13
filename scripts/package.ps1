param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    if (-not $SkipBuild) {
        cargo build --workspace --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    # The development screenshot feature drives the GUI from environment variables and must never
    # ship. -SkipBuild can otherwise package whatever a previous `--features screenshot` build left.
    $desktop = Join-Path $root 'target/release/duckie.exe'
    $image = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($desktop))
    if ($image.Contains('DUCKIE_CAPTURE_PATH')) {
        throw "$desktop was built with the screenshot feature. Rebuild without it before packaging."
    }
    $destination = Join-Path $root 'dist/Duckie-0.1.0-windows-x64'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    foreach ($file in @('duckie.exe', 'duckie-test-worker.exe')) {
        Copy-Item -LiteralPath (Join-Path $root "target/release/$file") -Destination $destination -Force
    }
    foreach ($file in @('README.md', 'IMPLEMENTATION_STATUS.md', 'PERFORMANCE.md')) {
        Copy-Item -LiteralPath (Join-Path $root $file) -Destination $destination -Force
    }
    $metadataText = cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata failed' }
    $metadata = $metadataText | ConvertFrom-Json
    $notices = [System.Text.StringBuilder]::new()
    [void]$notices.AppendLine('# Third-party dependencies')
    [void]$notices.AppendLine('Dependency metadata and available packaged license/notice files. Review before public redistribution.')
    foreach ($package in ($metadata.packages | Where-Object { $_.source } | Sort-Object name, version)) {
        [void]$notices.AppendLine("`n## $($package.name) $($package.version)")
        [void]$notices.AppendLine("License expression: $($package.license)")
        [void]$notices.AppendLine("Source: $($package.repository)")
        $packageRoot = Split-Path -Parent $package.manifest_path
        $licenseFiles = Get-ChildItem -LiteralPath $packageRoot -File | Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE)' }
        foreach ($license in $licenseFiles) {
            [void]$notices.AppendLine("`n### $($license.Name)`n")
            [void]$notices.AppendLine((Get-Content -LiteralPath $license.FullName -Raw))
        }
    }
    [System.IO.File]::WriteAllText((Join-Path $destination 'THIRD_PARTY_NOTICES.md'), $notices.ToString(), [System.Text.UTF8Encoding]::new($false))
    $zip = Join-Path $root 'dist/Duckie-0.1.0-windows-x64.zip'
    Compress-Archive -Path $destination -DestinationPath $zip -Force
    Get-Item -LiteralPath $zip | Select-Object FullName, Length
} finally { Pop-Location }
