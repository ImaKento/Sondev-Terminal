# Sondev Terminal installer build script (ASCII only, works on Windows PowerShell 5.1)
#
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1
#
# 1. cargo build --release
# 2. THIRD_PARTY_LICENSES.html (if cargo-about is installed)
# 3. fzf.exe to bundle (installer\vendor; downloaded once, used by cdi)
# 4. Inno Setup -> dist\SondevTerminal-setup-<version>.exe
#
# Requirements: Rust, Visual Studio Build Tools, Inno Setup 6
#   winget install JRSoftware.InnoSetup
#   cargo install cargo-about --locked --features cli   (optional, for the license list)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# version from Cargo.toml ([workspace.package] version = "x.y.z")
$version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Sondev Terminal $version" -ForegroundColor Cyan

Write-Host "== cargo build --release" -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

New-Item -ItemType Directory -Force -Path "dist" | Out-Null

if (Get-Command cargo-about -ErrorAction SilentlyContinue) {
    Write-Host "== third-party licenses" -ForegroundColor Cyan
    cargo about generate --workspace installer\about.hbs -o dist\THIRD_PARTY_LICENSES.html
    if ($LASTEXITCODE -ne 0) { Write-Warning "cargo about failed; the license list is skipped" }
} else {
    Write-Warning "cargo-about is not installed; the license list is skipped (cargo install cargo-about --locked --features cli)"
}

# fzf.exe for cdi (MIT license). Downloaded once into installer\vendor.
$vendor = Join-Path $root "installer\vendor"
New-Item -ItemType Directory -Force -Path $vendor | Out-Null
$fzfExe = Join-Path $vendor "fzf.exe"
if (-not (Test-Path $fzfExe)) {
    Write-Host "== fzf (download)" -ForegroundColor Cyan
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        # On CI, use the token to avoid the API rate limit for anonymous requests
        $headers = @{}
        if ($env:GITHUB_TOKEN) { $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN" }
        $rel = Invoke-RestMethod -UseBasicParsing -Headers $headers "https://api.github.com/repos/junegunn/fzf/releases/latest"
        $asset = $rel.assets | Where-Object { $_.name -like "*windows_amd64.zip" } | Select-Object -First 1
        $zip = Join-Path $env:TEMP $asset.name
        Invoke-WebRequest -UseBasicParsing $asset.browser_download_url -OutFile $zip
        Expand-Archive -Force $zip -DestinationPath $vendor
        Remove-Item $zip
        Write-Host "fzf $($rel.tag_name)"
    } catch {
        $local = Get-Command fzf -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($local) {
            Write-Warning "fzf download failed; bundling the installed copy: $($local.Source)"
            Copy-Item $local.Source $fzfExe
        } else {
            Write-Warning "fzf download failed; the installer is built without fzf (cdi uses its built-in picker)"
        }
    }
}
$fzfLicense = Join-Path $vendor "fzf-LICENSE.txt"
if ((Test-Path $fzfExe) -and -not (Test-Path $fzfLicense)) {
    try {
        Invoke-WebRequest -UseBasicParsing "https://raw.githubusercontent.com/junegunn/fzf/master/LICENSE" -OutFile $fzfLicense
    } catch {
        Write-Warning "could not download the fzf license text"
    }
}

$iscc = @(
    (Get-Command iscc -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup" }

Write-Host "== Inno Setup" -ForegroundColor Cyan
& $iscc "/DAppVersion=$version" "installer\wimux.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }

Write-Host "done: dist\SondevTerminal-setup-$version.exe" -ForegroundColor Green
