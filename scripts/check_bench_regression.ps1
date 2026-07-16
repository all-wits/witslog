# FR-P7-002: compare the just-run Criterion benches against the committed
# baseline under bench/baseline/ and fail if any mean time regressed > 20%.
#
# Run after `cargo bench -p witslog-bench -- --save-baseline ci`.
# To refresh the baseline after an intentional perf change:
#   pwsh scripts/check_bench_regression.ps1 -UpdateBaseline
param(
    [switch]$UpdateBaseline,
    [double]$ThresholdPct = 20.0
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$criterionDir = Join-Path $root "target\criterion"
$baselineDir = Join-Path $root "bench\baseline"

if (-not (Test-Path $criterionDir)) {
    throw "no target/criterion dir found - run 'cargo bench -p witslog-bench -- --save-baseline ci' first"
}

if ($UpdateBaseline) {
    New-Item -ItemType Directory -Force -Path $baselineDir | Out-Null
}

$failed = $false
$benchDirs = Get-ChildItem -Path $criterionDir -Recurse -Filter "estimates.json" |
    Where-Object { $_.FullName -match "\\ci\\estimates\.json$" }

foreach ($file in $benchDirs) {
    # target/criterion/<group>/<bench>/ci/estimates.json
    $benchName = Split-Path (Split-Path (Split-Path $file.FullName -Parent) -Parent) -Leaf
    $groupName = Split-Path (Split-Path (Split-Path (Split-Path $file.FullName -Parent) -Parent) -Parent) -Leaf
    $key = "$groupName--$benchName"

    $estimates = Get-Content $file.FullName -Raw | ConvertFrom-Json
    $meanNs = $estimates.mean.point_estimate

    $baselineFile = Join-Path $baselineDir "$key.json"

    if ($UpdateBaseline) {
        @{ mean_ns = $meanNs } | ConvertTo-Json | Set-Content $baselineFile
        Write-Output "Updated baseline: $key -> $meanNs ns"
        continue
    }

    if (-not (Test-Path $baselineFile)) {
        Write-Output "No baseline for $key yet (skipping regression check; run -UpdateBaseline to create one)"
        continue
    }

    $baseline = Get-Content $baselineFile -Raw | ConvertFrom-Json
    $baselineNs = $baseline.mean_ns
    $regressionPct = (($meanNs - $baselineNs) / $baselineNs) * 100

    if ($regressionPct -gt $ThresholdPct) {
        Write-Output "REGRESSION: $key mean $meanNs ns vs baseline $baselineNs ns (+$([math]::Round($regressionPct,1))%, threshold $ThresholdPct%)"
        $failed = $true
    } else {
        Write-Output "OK: $key mean $meanNs ns vs baseline $baselineNs ns ($([math]::Round($regressionPct,1))%)"
    }
}

if ($failed) {
    throw "one or more benchmarks regressed beyond $ThresholdPct%"
}
