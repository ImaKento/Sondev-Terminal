# Write the winget manifests for a release (ASCII only).
#
#   pwsh installer/winget.ps1 -Version 0.2.0 -Installer dist\SondevTerminal-setup-0.2.0.exe
#
# Fills {{VERSION}} / {{URL}} / {{SHA256}} / {{DATE}} in installer/winget/*.yaml and writes
# dist/winget/<files>. The Release workflow submits them to microsoft/winget-pkgs with
# wingetcreate (when the WINGET_TOKEN secret is set).

param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Installer,
    [string]$Repo = "ImaKento/Sondev-Terminal",
    [string]$OutDir = "dist/winget"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

if (-not (Test-Path -LiteralPath $Installer)) { throw "installer not found: $Installer" }
$name = Split-Path -Leaf $Installer
$url = "https://github.com/$Repo/releases/download/v$Version/$name"
$sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $Installer).Hash.ToUpperInvariant()
$date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-dd")

$out = Join-Path $root $OutDir
New-Item -ItemType Directory -Force -Path $out | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding $false

$schemas = @{
    "version"       = "https://aka.ms/winget-manifest.version.1.6.0.schema.json"
    "installer"     = "https://aka.ms/winget-manifest.installer.1.6.0.schema.json"
    "defaultLocale" = "https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json"
    "locale"        = "https://aka.ms/winget-manifest.locale.1.6.0.schema.json"
}

foreach ($tpl in Get-ChildItem (Join-Path $root "installer/winget") -Filter *.yaml) {
    $text = [IO.File]::ReadAllText($tpl.FullName, $utf8)
    $text = $text.Replace("{{VERSION}}", $Version)
    $text = $text.Replace("{{URL}}", $url)
    $text = $text.Replace("{{SHA256}}", $sha)
    $text = $text.Replace("{{DATE}}", $date)
    # Drop the template's own comment lines (they are notes for this repository)
    $lines = @($text -split "`r?`n" | Where-Object { $_ -notmatch '^\s*#' })
    $type = ($lines | Where-Object { $_ -match '^ManifestType:\s*(\S+)' } | Select-Object -First 1) -replace '^ManifestType:\s*', ''
    $header = "# yaml-language-server: `$schema=$($schemas[$type.Trim()])"
    $body = (@($header) + $lines) -join "`n"
    [IO.File]::WriteAllText((Join-Path $out $tpl.Name), $body.TrimEnd() + "`n", $utf8)
}

Write-Host "winget manifests: $out (v$Version, $sha)"
