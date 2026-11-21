<#
.SYNOPSIS
  End-to-end test of `cross-control service` on Windows: register the logon
  task, start it, check the daemon runs windowless under cross-controlw,
  stop it cleanly, and remove the task.

.DESCRIPTION
  Uses private config/runtime directories and the headless input backend, so
  it touches nothing else on the machine. CI runs it on the Windows runner:

    ./deploy/windows/tests/logon-task.ps1 -Bin target/debug
#>
param([string]$Bin = 'target/debug')

$ErrorActionPreference = 'Stop'
$cc = (Resolve-Path (Join-Path $Bin 'cross-control.exe')).Path
$root = Join-Path ([IO.Path]::GetTempPath()) "cc-logon-task-$PID"
$env:CROSS_CONTROL_CONFIG_DIR = Join-Path $root 'config'
$env:CROSS_CONTROL_RUNTIME_DIR = Join-Path $root 'run'
New-Item -ItemType Directory -Force $env:CROSS_CONTROL_CONFIG_DIR, $env:CROSS_CONTROL_RUNTIME_DIR | Out-Null
@'
[identity]
name = "logon-task-test"
[daemon]
bind = "127.0.0.1"
port = 0
discovery = false
[input]
backend = "none"
[clipboard]
enabled = false
'@ | Set-Content -Encoding utf8 (Join-Path $env:CROSS_CONTROL_CONFIG_DIR 'config.toml')

# Run cross-control with --json; check the exit code; return the document.
function cc([string[]]$Arguments, [int]$Expect = 0) {
    $out = & $cc @Arguments --json
    if ($LASTEXITCODE -ne $Expect) {
        throw "cross-control $Arguments exited $LASTEXITCODE (expected $Expect): $out"
    }
    $out | ConvertFrom-Json
}

function Wait-Until([scriptblock]$Condition, [string]$What, [int]$Seconds = 30) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while (-not (& $Condition)) {
        if ((Get-Date) -gt $deadline) { throw "timed out waiting for: $What" }
        Start-Sleep -Milliseconds 250
    }
}

function Show-Log {
    $log = Join-Path $env:CROSS_CONTROL_RUNTIME_DIR 'cross-control.log'
    if (Test-Path $log) { Write-Host "--- daemon log ---"; Get-Content $log | Write-Host }
}

try {
    $installed = cc @('service', 'install', '--enable')
    if (-not $installed.changed) { throw 'first install reported no change' }

    Wait-Until { & $cc status *> $null; $LASTEXITCODE -eq 0 } 'the daemon to start'
    $status = cc @('status')
    Write-Host "daemon running as PID $($status.pid)"
    $launcher = Get-Process cross-controlw -ErrorAction SilentlyContinue
    if (-not $launcher) { throw 'the task did not start cross-controlw' }
    $daemon = Get-Process -Id $status.pid
    if ($daemon.MainWindowHandle -ne 0) { throw 'the daemon has a window' }

    $again = cc @('service', 'install')
    if ($again.changed) { throw 're-installing an unchanged task reported a change' }

    $stopped = cc @('stop')
    if (-not $stopped.graceful) { throw 'stop was not graceful' }
    # A clean exit ends the launcher too (no restart).
    Wait-Until { -not (Get-Process cross-controlw -ErrorAction SilentlyContinue) } 'cross-controlw to exit' 15
    cc @('status') -Expect 3 | Out-Null

    $removed = cc @('service', 'uninstall')
    if (-not $removed.changed) { throw 'uninstall reported no change' }
    schtasks /Query /TN cross-control *> $null
    if ($LASTEXITCODE -eq 0) { throw 'the task is still registered' }
    Write-Host 'logon task end-to-end: ok'
}
catch {
    Show-Log
    throw
}
finally {
    & $cc service uninstall *> $null
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}
