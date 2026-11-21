# CrossControl PowerShell module

Install, configure and run cross-control from PowerShell. It is the Windows
counterpart of the [Ansible role](../ansible/roles/cross_control/README.md),
and works with pwsh on Linux too.

Every function is a thin wrapper over the CLI's `--json` interface
([automation](../../docs/automation.md)). Each one changes only what differs
and returns a `Changed` property, so one script can run on every machine on
every deployment. The functions that make changes also support `-WhatIf`.

```powershell
Import-Module .\deploy\powershell\CrossControl

Install-CrossControl -Path .\target\release -AddToPath
Set-CrossControlConfig -Name desk -Screens @{ Name = 'laptop'; Position = 'Right'; Address = '192.0.2.20:24800' }
Invoke-Command -ComputerName laptop { Get-CrossControlIdentity } | Add-CrossControlTrust
Enable-CrossControlAutostart -Start
Get-CrossControlStatus
```

| Function | Does |
|---|---|
| `Install-CrossControl -Path DIR [-Destination] [-AddToPath]` | Copies `cross-control` (and `cross-controlw` on Windows) only if they differ. Stops and restarts a running daemon around the copy. |
| `Get-CrossControlIdentity` | Name and fingerprint; creates the identity on first use (`created`). |
| `Set-CrossControlConfig` | Writes `config.toml` from parameters (`-Screens`, `-Monitors`, `-Port`, `-InputBackend`, …). It is validated with `config check` before replacing the current file. `RestartNeeded` says whether the running daemon must be restarted. |
| `Set-CrossControlTrust -Peers` | Makes the trusted machines exactly `-Peers` (objects with `Name` and `Fingerprint`). This machine's own entry is skipped. |
| `Add-` / `Remove-` / `Get-CrossControlTrust` | Single trust entries; `Add` accepts pipeline input from `Get-CrossControlIdentity`. |
| `Start-` / `Stop-` / `Restart-CrossControl` | Run the daemon in the background (through the logon task when registered) and wait for it. `Stop` is a clean shutdown. |
| `Enable-` / `Disable-CrossControlAutostart` | The logon task on Windows (`service install` / `uninstall`); the systemd user service on Linux. |
| `Get-CrossControlStatus`, `Test-CrossControl [-Quiet]` | `status` and `doctor`. |
| `Invoke-CrossControl ARGS` | Any other command, with its JSON parsed. |

The binary is found from `$env:CROSS_CONTROL_EXE`, then the one
`Install-CrossControl` installed, then the default install directory
(`%LOCALAPPDATA%\Programs\cross-control`), then `PATH`.
`CROSS_CONTROL_CONFIG_DIR` / `CROSS_CONTROL_RUNTIME_DIR` work as for the CLI,
and `Enable-CrossControlAutostart` carries them into the task.

## Fleets

[`examples/Deploy-CrossControl.ps1`](examples/Deploy-CrossControl.ps1) deploys
a layout to many machines over PowerShell remoting, as the Ansible role does.
It installs everywhere, collects every identity, writes each layout and
exchanges trust, then registers and starts the logon tasks:

```powershell
$layout = @{
    desk   = @{ Screens = @(@{ Name = 'laptop'; Position = 'Right' }) }
    laptop = @{ Screens = @(@{ Name = 'desk'; Position = 'Left' }) }
}
.\deploy\powershell\examples\Deploy-CrossControl.ps1 -Layout $layout -BinaryPath .\target\release
```

Connect as the user who sits at each machine. The daemon runs in their
desktop session through the logon task: it starts now if they are logged on,
otherwise at their next logon.

## Testing

[`tests/run.ps1`](tests/run.ps1) deploys two machines on one host with the
module (separate directories, headless input backend). It checks that every
step is idempotent, that an invalid layout is refused, and that the two
daemons connect to each other. On Windows it also checks the logon task.
CI runs it on Windows PowerShell 5.1, and on PowerShell 7 on Windows and
Linux:

```powershell
.\deploy\powershell\tests\run.ps1 -Bin target\debug
```
