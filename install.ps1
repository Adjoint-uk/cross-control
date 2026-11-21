<#
.SYNOPSIS
  cross-control installer for Windows.

.DESCRIPTION
  Downloads a release archive, checks it against the release's SHA256SUMS,
  installs cross-control.exe and cross-controlw.exe with the bundled
  CrossControl PowerShell module, and runs `cross-control doctor`.

    irm https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.ps1 | iex

  Or, to choose options:

    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/Adjoint-uk/cross-control/main/install.ps1))) -Version 0.3.0 -Autostart

.PARAMETER Version
  Version to install (default: the newest release).
.PARAMETER Destination
  Install directory (default: %LOCALAPPDATA%\Programs\cross-control).
.PARAMETER Autostart
  Also register the logon task and start cross-control now.
.PARAMETER DownloadBase
  URL holding the archives and SHA256SUMS, for an internal mirror (default:
  the GitHub release).
#>
param(
    [string]$Version = $env:CROSS_CONTROL_VERSION,
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'Programs\cross-control'),
    [switch]$Autostart,
    [string]$DownloadBase = $env:CROSS_CONTROL_DOWNLOAD_BASE
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # much faster downloads on 5.1
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$repo = 'Adjoint-uk/cross-control'

if ([Environment]::OSVersion.Platform -ne 'Win32NT') { throw 'This installer is for Windows; on Linux use install.sh.' }
if (-not [Environment]::Is64BitOperatingSystem) { throw 'cross-control needs 64-bit Windows.' }
$target = 'x86_64-pc-windows-msvc'

if (-not $Version) {
    # The newest release, including pre-releases.
    $releases = Invoke-RestMethod "https://api.github.com/repos/$repo/releases?per_page=1"
    $Version = @($releases)[0].tag_name
    if (-not $Version) { throw 'Could not find the latest release; pass -Version.' }
}
$Version = $Version.TrimStart('v')

$archive = "cross-control-$Version-$target.zip"
$base = if ($DownloadBase) { $DownloadBase.TrimEnd('/') } else { "https://github.com/$repo/releases/download/v$Version" }
$work = Join-Path ([IO.Path]::GetTempPath()) "cross-control-install-$PID"
New-Item -ItemType Directory -Force -Path $work | Out-Null
try {
    Write-Host "Downloading cross-control $Version"
    Invoke-WebRequest "$base/$archive" -OutFile (Join-Path $work $archive) -UseBasicParsing
    Invoke-WebRequest "$base/SHA256SUMS" -OutFile (Join-Path $work 'SHA256SUMS') -UseBasicParsing

    $line = Get-Content (Join-Path $work 'SHA256SUMS') | Where-Object { ($_ -split '\s+')[1] -eq $archive }
    if (-not $line) { throw "$archive is not listed in SHA256SUMS" }
    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash (Join-Path $work $archive) -Algorithm SHA256).Hash
    if ($actual -ne $expected) { throw "Checksum mismatch for $archive" }
    Write-Host 'Checksum verified'

    Expand-Archive (Join-Path $work $archive) -DestinationPath $work -Force
    $pkg = Join-Path $work "cross-control-$Version-$target"

    # Install the module where PowerShell finds it, then use it to install
    # the binaries (it stops and restarts a running daemon around the copy).
    $modules = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'WindowsPowerShell\Modules'
    if ($PSVersionTable.PSVersion.Major -ge 6) {
        $modules = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'PowerShell\Modules'
    }
    $moduleDir = Join-Path $modules 'CrossControl'
    New-Item -ItemType Directory -Force -Path $moduleDir | Out-Null
    Copy-Item -Path (Join-Path $pkg 'CrossControl\*') -Destination $moduleDir -Force
    Import-Module $moduleDir -Force

    $installed = Install-CrossControl -Path $pkg -Destination $Destination -AddToPath
    $exe = Join-Path $Destination 'cross-control.exe'
    Write-Host "Installed $(& $exe --version) to $Destination"
    Write-Host "PowerShell module: $moduleDir"

    $configDir = Join-Path $env:APPDATA 'cross-control'
    if (-not (Test-Path (Join-Path $configDir 'config.toml'))) {
        New-Item -ItemType Directory -Force -Path $configDir | Out-Null
        Copy-Item (Join-Path $pkg 'config.example.toml') (Join-Path $configDir 'config.example.toml') -Force
        Write-Host "Example configuration: $configDir\config.example.toml"
    }

    Write-Host ''
    Write-Host 'Checking this machine (cross-control doctor):'
    & $exe doctor

    if ($Autostart) {
        Enable-CrossControlAutostart -Start | Out-Null
        Write-Host 'cross-control starts at logon, and is running now.'
    }
    else {
        Write-Host 'To start at logon: Enable-CrossControlAutostart -Start'
    }
    Write-Host ''
    Write-Host 'Next: cross-control identity   (this machine''s fingerprint, for pairing)'
    if ($installed.Changed) { Write-Host 'Open a new terminal for PATH changes to apply.' }
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
