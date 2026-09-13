param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    if (-not $SkipBuild) {
        cargo build --workspace --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    # The screenshot and bench features drive the GUI from environment variables and must never
    # ship. -SkipBuild can otherwise package whatever a previous featured build left behind.
    $desktop = Join-Path $root 'target/release/duckie.exe'
    $image = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($desktop))
    foreach ($marker in @('DUCKIE_CAPTURE_PATH', 'DUCKIE_BENCH_PATH')) {
        if ($image.Contains($marker)) {
            throw "$desktop contains $marker, so it was built with a development feature. Rebuild without it before packaging."
        }
    }
    $metadataText = cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata failed' }
    $metadata = $metadataText | ConvertFrom-Json
    # Read the version from Cargo.toml via cargo metadata rather than hardcoding it here, so the
    # package name cannot silently drift from the workspace version after a bump.
    $version = ($metadata.packages | Where-Object { $_.name -eq 'duckie-desktop' }).version
    $destination = Join-Path $root "dist/Duckie-$version-windows-x64"
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    foreach ($file in @('duckie.exe', 'duckie-test-worker.exe')) {
        Copy-Item -LiteralPath (Join-Path $root "target/release/$file") -Destination $destination -Force
    }
    foreach ($file in @('LICENSE', 'README.md', 'IMPLEMENTATION_STATUS.md', 'PERFORMANCE.md')) {
        Copy-Item -LiteralPath (Join-Path $root $file) -Destination $destination -Force
    }
    $notices = [System.Text.StringBuilder]::new()
    [void]$notices.AppendLine('# Third-party dependencies')
    [void]$notices.AppendLine('Duckie itself is MIT licensed; see LICENSE. Below is the metadata and available license/notice files for every crate linked into these binaries.')
    [void]$notices.AppendLine("`nWhere a crate offers a choice of licenses, Duckie takes the permissive option — notably Apache-2.0 for ``self_cell``, which is offered as Apache-2.0 OR GPL-2.0-only. ``epaint_default_fonts`` embeds typefaces under OFL-1.1 and the Ubuntu Font Licence, which permit redistribution inside an application but not sale of the fonts on their own. Re-run this script and re-read this file whenever Cargo.lock changes.")
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
    $zip = Join-Path $root "dist/Duckie-$version-windows-x64.zip"
    Compress-Archive -Path $destination -DestinationPath $zip -Force
    Get-Item -LiteralPath $zip | Select-Object FullName, Length
} finally { Pop-Location }
