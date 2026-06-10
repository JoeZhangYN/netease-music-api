$ErrorActionPreference = "Stop"

Write-Host "`n=== Building Netease Music API ===" -ForegroundColor Cyan

# dist arm64 已退役（cfdfd7f）——dist/ 只保留 windows-x64 + linux-x64。
# 原 [3/3] Linux ARM64 musl 步骤 + arm64 拷贝已删，对齐 dist/ 现实（改行为同步对齐脚本）。

# 1. Windows x64 (native)
Write-Host "`n[1/2] Windows x64 (native)..." -ForegroundColor Yellow
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "Windows build failed" }

# 2. Linux x64 musl (via cross + Docker)
Write-Host "`n[2/2] Linux x64 musl (cross)..." -ForegroundColor Yellow
cross build --release --target x86_64-unknown-linux-musl
if ($LASTEXITCODE -ne 0) { throw "Linux x64 build failed" }

# 3. Collect artifacts
Write-Host "`nCollecting artifacts to dist/..." -ForegroundColor Yellow
New-Item -ItemType Directory -Force -Path dist | Out-Null
Copy-Item target/release/netease-music-api.exe dist/netease-music-api-windows-x64.exe
Copy-Item target/x86_64-unknown-linux-musl/release/netease-music-api dist/netease-music-api-linux-x64

Write-Host "`n=== Build complete ===" -ForegroundColor Green
Write-Host "Artifacts:"
Get-ChildItem dist/ | ForEach-Object {
    $size = "{0:N2} MB" -f ($_.Length / 1MB)
    Write-Host "  $($_.Name)  ($size)"
}
