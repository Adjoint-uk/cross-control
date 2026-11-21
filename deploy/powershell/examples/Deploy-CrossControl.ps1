<#
.SYNOPSIS
  Deploy cross-control to Windows machines over PowerShell remoting: the
  PowerShell counterpart of the Ansible role.

.DESCRIPTION
  For every machine in -Layout it:
    1. copies the CrossControl module and the binaries over, and installs
       them (only if changed);
    2. creates the machine's identity and collects its fingerprint;
    3. writes its layout and makes it trust exactly the other machines;
    4. registers the logon task and starts it, restarting the daemon if
       its layout changed.

  Re-running it changes only what differs. Removing a machine from -Layout
  and re-running makes the others stop trusting it.

  Connect as the user who sits at each machine. The daemon runs in that
  user's desktop session, through the logon task, so it starts only while
  they are logged on (at their next logon otherwise).

.PARAMETER Layout
  Computer name -> Set-CrossControlConfig parameters for it. `Name`
  defaults to the computer name.

.PARAMETER BinaryPath
  Local directory with cross-control.exe and cross-controlw.exe.

.EXAMPLE
  $layout = @{
      desk   = @{ Screens = @(@{ Name = 'laptop'; Position = 'Right' }) }
      laptop = @{ Screens = @(@{ Name = 'desk'; Position = 'Left' }) }
  }
  .\Deploy-CrossControl.ps1 -Layout $layout -BinaryPath .\target\release
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [hashtable]$Layout,
    [Parameter(Mandatory)]
    [string]$BinaryPath,
    [pscredential]$Credential
)

$ErrorActionPreference = 'Stop'
$module = Join-Path $PSScriptRoot '..\CrossControl'
$binaries = 'cross-control.exe', 'cross-controlw.exe' | ForEach-Object { Join-Path $BinaryPath $_ }

$connect = @{ ComputerName = @($Layout.Keys) }
if ($Credential) { $connect.Credential = $Credential }
$sessions = New-PSSession @connect

try {
    # 1-2. Install and collect identities.
    $identities = foreach ($session in $sessions) {
        $computer = $session.ComputerName
        $stage = Invoke-Command -Session $session {
            $dir = Join-Path $env:TEMP 'cross-control-deploy'
            New-Item -ItemType Directory -Force -Path $dir | Out-Null
            $dir
        }
        Copy-Item -ToSession $session -Path $module -Destination $stage -Recurse -Force
        Copy-Item -ToSession $session -Path $binaries -Destination $stage -Force
        $name = if ($Layout[$computer].Name) { $Layout[$computer].Name } else { $computer }
        Invoke-Command -Session $session {
            Import-Module (Join-Path $using:stage 'CrossControl') -Force
            $installed = Install-CrossControl -Path $using:stage -AddToPath
            $identity = Get-CrossControlIdentity
            [pscustomobject]@{
                Computer    = $using:computer
                Name        = $using:name
                Fingerprint = $identity.fingerprint
                Installed   = $installed.Changed
            }
        }
    }

    # 3-4. Layout, trust, autostart.
    $results = foreach ($session in $sessions) {
        $computer = $session.ComputerName
        $config = @{ Name = $computer } + $Layout[$computer]
        Invoke-Command -Session $session {
            $config = $using:config
            $layout = Set-CrossControlConfig @config
            $trust = Set-CrossControlTrust -Peers $using:identities
            if ($layout.RestartNeeded) { Stop-CrossControl | Out-Null }
            $autostart = Enable-CrossControlAutostart -Start
            Start-Sleep -Seconds 2
            [pscustomobject]@{
                Computer       = $using:computer
                ConfigChanged  = $layout.Changed
                TrustChanged   = $trust.Changed
                TaskChanged    = $autostart.Changed
                Running        = [bool](Get-CrossControlStatus).running
            }
        }
    }
    $results | Select-Object Computer, ConfigChanged, TrustChanged, TaskChanged, Running | Format-Table -AutoSize
    $identities | Select-Object Computer, Name, Fingerprint, Installed | Format-Table -AutoSize
}
finally {
    $sessions | Remove-PSSession
}
