<#
.SYNOPSIS
  End-to-end test of the CrossControl PowerShell module.

.DESCRIPTION
  Deploys two "machines" (separate configuration and runtime directories on
  this host) the way a fleet script would. It installs the binaries,
  creates the identities, writes each layout, exchanges trust and starts
  both daemons. It then checks that they connect to each other, and that
  every step reports a change only when it made one.

  Needs no Pester. Runs on Windows PowerShell 5.1, and on PowerShell 7 on
  Windows and Linux:

    ./deploy/powershell/tests/run.ps1 -Bin target/debug

  On Windows it also registers, starts and removes the real logon task.
#>
param([string]$Bin = 'target/debug')

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Resolve-Path (Join-Path $PSScriptRoot '../../..')
Import-Module (Join-Path $repo 'deploy/powershell/CrossControl/CrossControl.psd1') -Force

$root = Join-Path ([System.IO.Path]::GetTempPath()) "cc-psmodule-$PID"
$isWin = [System.Environment]::OSVersion.Platform -eq 'Win32NT'
$failures = 0

function Check([bool]$Condition, [string]$What) {
    if ($Condition) {
        Write-Host "  ok   $What"
    }
    else {
        Write-Host "  FAIL $What" -ForegroundColor Red
        $script:failures++
    }
}

function Throws([scriptblock]$Block) {
    try { & $Block | Out-Null; $false } catch { $true }
}

# Act as one of the two machines.
function Use-Machine([string]$Name) {
    $env:CROSS_CONTROL_CONFIG_DIR = Join-Path $root "$Name/config"
    $env:CROSS_CONTROL_RUNTIME_DIR = Join-Path $root "$Name/run"
}

$machines = @{
    alpha = @{ Port = 24811; Neighbour = 'bravo'; Position = 'Right' }
    bravo = @{ Port = 24812; Neighbour = 'alpha'; Position = 'Left' }
}

try {
    Write-Host 'Syntax'
    foreach ($file in Get-ChildItem -Recurse -Path (Join-Path $repo 'deploy') -Include *.ps1, *.psm1, *.psd1) {
        $errors = $null
        [System.Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$null, [ref]$errors) | Out-Null
        Check (@($errors).Count -eq 0) "$($file.Name) parses $(@($errors) -join '; ')"
    }
    Check ($null -ne (Test-ModuleManifest (Join-Path $repo 'deploy/powershell/CrossControl/CrossControl.psd1'))) 'the module manifest is valid'

    Write-Host 'Install'
    $dest = Join-Path $root 'bin'
    $installed = Install-CrossControl -Path (Resolve-Path $Bin) -Destination $dest
    Check $installed.Changed 'first install copies the binaries'
    Check (-not (Install-CrossControl -Path (Resolve-Path $Bin) -Destination $dest).Changed) 'second install changes nothing'
    Check ((Get-CrossControlExe) -like "$dest*") 'the installed binary is used from now on'

    Write-Host 'Identities'
    $identities = foreach ($name in 'alpha', 'bravo') {
        Use-Machine $name
        $id = Get-CrossControlIdentity
        Check ([bool]$id.created) "$name's identity is created"
        Check (-not (Get-CrossControlIdentity).created) "$name's identity is created only once"
        Check ($id.fingerprint -like 'SHA256:*') "$name has a fingerprint"
        [pscustomobject]@{ Name = $name; Fingerprint = $id.fingerprint }
    }

    Write-Host 'Layout'
    foreach ($name in 'alpha', 'bravo') {
        Use-Machine $name
        $m = $machines[$name]
        $other = $machines[$m.Neighbour]
        $layout = @{
            Name         = $name
            Port         = $m.Port
            Bind         = '127.0.0.1'
            Discovery    = $false
            InputBackend = 'none'
            Clipboard    = $false
            Screens      = @(@{ Name = $m.Neighbour; Position = $m.Position; Address = "127.0.0.1:$($other.Port)" })
        }
        $set = Set-CrossControlConfig @layout
        Check $set.Changed "$name's config is written"
        Check (-not $set.RestartNeeded) "$name needs no restart (not running)"
        Check (-not (Set-CrossControlConfig @layout).Changed) "$name's unchanged config is left alone"

        $before = Get-Content -Raw $set.Path
        $bad = $layout.Clone()
        $bad.Screens = @(@{ Name = 'x'; Position = 'Sideways' })
        Check (Throws { Set-CrossControlConfig @bad }) "an invalid layout for $name is refused"
        Check ((Get-Content -Raw $set.Path) -eq $before) "the refused layout did not replace $name's config"
    }

    Write-Host 'Trust'
    foreach ($name in 'alpha', 'bravo') {
        Use-Machine $name
        # Everyone's identity, own included: the module skips this machine.
        $trust = Set-CrossControlTrust -Peers $identities
        Check $trust.Changed "$name trusts the others"
        Check (@($trust.Trusted) -join ',' -eq ($machines[$name].Neighbour)) "$name trusts exactly its neighbour"
        Check (-not (Set-CrossControlTrust -Peers $identities).Changed) "$name's trust is already right"
    }
    Use-Machine 'alpha'
    Add-CrossControlTrust -Name temp -Fingerprint $identities[0].Fingerprint | Out-Null
    Check ((Set-CrossControlTrust -Peers $identities).Changed) 'an extra trusted machine is removed'
    Check (-not (Get-CrossControlTrust | Where-Object name -eq 'temp')) 'and is gone'

    Write-Host 'Doctor'
    $checks = Test-CrossControl
    Check (@($checks).Count -gt 0) 'doctor reports checks'
    Check (Test-CrossControl -Quiet) 'doctor finds nothing failing'

    Write-Host 'Run'
    foreach ($name in 'alpha', 'bravo') {
        Use-Machine $name
        $started = Start-CrossControl
        Check $started.Changed "$name starts"
        Check (-not (Start-CrossControl).Changed) "$name is not started twice"
    }
    $deadline = (Get-Date).AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 500
        $connected = foreach ($name in 'alpha', 'bravo') {
            Use-Machine $name
            @((Get-CrossControlStatus).peers).Count -eq 1
        }
    } until (($connected -notcontains $false) -or (Get-Date) -gt $deadline)
    Check ($connected -notcontains $false) 'alpha and bravo are connected to each other'

    Use-Machine 'alpha'
    $first = (Get-CrossControlStatus).pid
    $restarted = Restart-CrossControl
    Check ($restarted.Pid -ne $first) 'restart replaces the process'

    foreach ($name in 'alpha', 'bravo') {
        Use-Machine $name
        $stopped = Stop-CrossControl
        Check ($stopped.Changed -and $stopped.Graceful) "$name stops cleanly"
        Check (-not (Get-CrossControlStatus).running) "$name is stopped"
        Check (-not (Stop-CrossControl).Changed) "stopping $name again changes nothing"
    }

    if ($isWin) {
        Write-Host 'Autostart (logon task)'
        Use-Machine 'alpha'
        $enabled = Enable-CrossControlAutostart -Start
        Check $enabled.Changed 'the logon task is registered'
        $deadline = (Get-Date).AddSeconds(30)
        while (-not (Get-CrossControlStatus).running -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
        Check ([bool](Get-CrossControlStatus).running) 'the task starts the daemon'
        Check (-not (Enable-CrossControlAutostart).Changed) 'an unchanged task is left alone'
        $disabled = Disable-CrossControlAutostart
        Check $disabled.Changed 'the task is removed'
        Check (-not (Get-CrossControlStatus).running) 'removing the task stops the daemon'
    }
}
catch {
    Write-Host "  FAIL unexpected error: $_" -ForegroundColor Red
    Write-Host $_.ScriptStackTrace
    $failures++
}
finally {
    foreach ($name in 'alpha', 'bravo') {
        Use-Machine $name
        try { Stop-CrossControl | Out-Null } catch { Write-Host "cleanup: $_" }
        if ($isWin) { try { Disable-CrossControlAutostart | Out-Null } catch { Write-Host "cleanup: $_" } }
    }
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}

if ($failures) {
    Write-Host "$failures check(s) failed" -ForegroundColor Red
    exit 1
}
Write-Host 'CrossControl module: all checks passed'
