$ErrorActionPreference = "Stop"

Write-Host "`n=== Building Netease Music API ===" -ForegroundColor Cyan

# 1. Windows x64 (native)
Write-Host "`n[1/3] Windows x64 (native)..." -ForegroundColor Yellow
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "Windows build failed" }

# 2. Linux x64 musl (via cross + Docker)
Write-Host "`n[2/3] Linux x64 musl (cross)..." -ForegroundColor Yellow
cross build --release --target x86_64-unknown-linux-musl
if ($LASTEXITCODE -ne 0) { throw "Linux x64 build failed" }

# 3. Linux ARM64 musl (via cross + Docker)
Write-Host "`n[3/3] Linux ARM64 musl (cross)..." -ForegroundColor Yellow
cross build --release --target aarch64-unknown-linux-musl
if ($LASTEXITCODE -ne 0) { throw "Linux ARM64 build failed" }

# 4. Collect artifacts
Write-Host "`nCollecting artifacts to dist/..." -ForegroundColor Yellow
New-Item -ItemType Directory -Force -Path dist | Out-Null
Copy-Item target/release/netease-music-api.exe dist/netease-music-api-windows-x64.exe
Copy-Item target/x86_64-unknown-linux-musl/release/netease-music-api dist/netease-music-api-linux-x64
Copy-Item target/aarch64-unknown-linux-musl/release/netease-music-api dist/netease-music-api-linux-arm64

Write-Host "`n=== Build complete ===" -ForegroundColor Green
Write-Host "Artifacts:"
Get-ChildItem dist/ | ForEach-Object {
    $size = "{0:N2} MB" -f ($_.Length / 1MB)
    Write-Host "  $($_.Name)  ($size)"
}
