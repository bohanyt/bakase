# Pengecekan cepat sebelum commit: format, lint, test, dan build.
# Jalankan dari root repo: ./scripts/check.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Step([string]$name, [scriptblock]$cmd) {
    Write-Host "==> $name" -ForegroundColor Cyan
    & $cmd
    if ($LASTEXITCODE -ne 0) {
        Write-Host "GAGAL: $name" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

Push-Location (Join-Path $root 'server')
try {
    Step 'cargo fmt' { cargo fmt --check }
    Step 'cargo clippy' { cargo clippy --all-targets -q -- -D warnings }
    Step 'cargo test' { cargo test -q }
} finally { Pop-Location }

Push-Location (Join-Path $root 'web')
try {
    if (-not (Test-Path 'node_modules')) { Step 'npm ci' { npm ci --silent } }
    Step 'lint frontend' { npm run lint --silent }
    Step 'test frontend' { npm test --silent }
    Step 'build frontend' { npm run build --silent }
    Step 'build extension' { npm run build:extension --silent }
} finally { Pop-Location }

Write-Host 'Semua cek lulus.' -ForegroundColor Green
