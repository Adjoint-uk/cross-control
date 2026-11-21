#Requires -Version 5.1
<#
  CrossControl: install, configure and run cross-control with PowerShell.

  Every function is a thin, idempotent wrapper over the `cross-control` CLI's
  `--json` interface: it changes only what differs and reports whether it did
  (a `Changed` property), so the same script can run on every machine on
  every deployment, as with the Ansible role.

  The functions work on Windows and, apart from the Windows logon task
  (Enable-CrossControlAutostart uses systemd there), on Linux with pwsh.
#>

Set-StrictMode -Version Latest

$script:IsWindowsHost = [System.Environment]::OSVersion.Platform -eq 'Win32NT'
$script:ExeSuffix = if ($script:IsWindowsHost) { '.exe' } else { '' }
# Set by Install-CrossControl for the rest of the session.
$script:Exe = $null

# --- Helpers -----------------------------------------------------------------

function Get-CrossControlDefaultInstallDir {
    if ($script:IsWindowsHost) {
        Join-Path $env:LOCALAPPDATA 'Programs\cross-control'
    }
    else {
        Join-Path $HOME '.local/bin'
    }
}

function Get-CrossControlExe {
    <#
    .SYNOPSIS
      The cross-control binary these functions run.
    .DESCRIPTION
      In order: $env:CROSS_CONTROL_EXE; the binary Install-CrossControl
      installed in this session; the default install directory; PATH.
    #>
    [CmdletBinding()]
    [OutputType([string])]
    param()
    if ($env:CROSS_CONTROL_EXE) { return $env:CROSS_CONTROL_EXE }
    if ($script:Exe) { return $script:Exe }
    $default = Join-Path (Get-CrossControlDefaultInstallDir) "cross-control$script:ExeSuffix"
    if (Test-Path -LiteralPath $default) { return $default }
    $command = Get-Command cross-control -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($command) { return $command.Source }
    throw 'cross-control not found: install it with Install-CrossControl, or set $env:CROSS_CONTROL_EXE.'
}

function Invoke-CrossControl {
    <#
    .SYNOPSIS
      Run cross-control with --json and return the parsed result.
    .DESCRIPTION
      Throws if the exit code is not one of -AllowedExitCodes (by default
      only 0), with the command's error message. The exit code is available
      afterwards as $LASTEXITCODE.
    .EXAMPLE
      Invoke-CrossControl config, check, C:\cc\config.toml
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory, Position = 0)]
        [string[]]$Arguments,
        [int[]]$AllowedExitCodes = @(0)
    )
    $exe = Get-CrossControlExe
    # stderr lines arrive as error records; they must not stop the caller.
    $ErrorActionPreference = 'Continue'
    $all = & $exe @Arguments --json 2>&1
    $code = $LASTEXITCODE
    $stdout = @($all | Where-Object { $_ -isnot [System.Management.Automation.ErrorRecord] })
    $stderr = @($all | Where-Object { $_ -is [System.Management.Automation.ErrorRecord] }) -join "`n"
    if ($code -notin $AllowedExitCodes) {
        $message = if ($stderr) { $stderr.Trim() } else { "exit code $code" }
        throw "cross-control $($Arguments -join ' '): $message"
    }
    $global:LASTEXITCODE = $code
    $text = ($stdout -join "`n").Trim()
    if ($text) { $text | ConvertFrom-Json }
}

# A TOML basic string: JSON's string escapes are valid TOML.
function ConvertTo-TomlString([string]$Value) {
    ConvertTo-Json -InputObject $Value -Compress
}

function ConvertTo-TomlBool([bool]$Value) {
    if ($Value) { 'true' } else { 'false' }
}

# Read a field from a hashtable or object, case-insensitively.
function Get-Field($Item, [string]$Name, $Default = $null) {
    if ($Item -is [System.Collections.IDictionary]) {
        foreach ($key in $Item.Keys) {
            if ($key -eq $Name) { return $Item[$key] }
        }
        return $Default
    }
    $property = $Item.PSObject.Properties[$Name]
    if ($property) { $property.Value } else { $Default }
}

# Run a native command, discarding its output; return whether it succeeded.
# Windows PowerShell 5.1 turns a native command's stderr into an error that
# would stop a caller using $ErrorActionPreference = 'Stop'.
function Invoke-NativeQuiet([string]$Command, [string[]]$Arguments) {
    $ErrorActionPreference = 'Continue'
    & $Command @Arguments 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

function Test-FileContent([string]$Path, [string]$Content) {
    (Test-Path -LiteralPath $Path) -and
        ([System.IO.File]::ReadAllText($Path) -ceq $Content)
}

function Wait-CrossControlState([bool]$Running, [int]$TimeoutSeconds) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ($true) {
        $status = Get-CrossControlStatus
        if ([bool]$status.running -eq $Running) { return $status }
        if ((Get-Date) -gt $deadline) {
            $want = if ($Running) { 'start' } else { 'stop' }
            throw "cross-control did not $want within $TimeoutSeconds seconds."
        }
        Start-Sleep -Milliseconds 250
    }
}

# --- Install -----------------------------------------------------------------

function Install-CrossControl {
    <#
    .SYNOPSIS
      Install the cross-control binaries, replacing them only if they differ.
    .DESCRIPTION
      Copies cross-control (and, on Windows, cross-controlw, which the logon
      task runs) from -Path. A running daemon is stopped for the copy (Windows
      cannot replace a running program) and started again afterwards.
    .PARAMETER Path
      A directory holding the binaries (e.g. target\release), or the
      cross-control binary itself.
    .PARAMETER Destination
      Install directory. Default: %LOCALAPPDATA%\Programs\cross-control on
      Windows, ~/.local/bin elsewhere.
    .PARAMETER AddToPath
      Add the install directory to the user's PATH (Windows).
    .EXAMPLE
      Install-CrossControl -Path \\fileserver\tools\cross-control -AddToPath
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [Parameter(Mandatory)]
        [string]$Path,
        [string]$Destination = (Get-CrossControlDefaultInstallDir),
        [switch]$AddToPath
    )
    $sourceDir = if (Test-Path -LiteralPath $Path -PathType Container) { $Path } else { Split-Path -Parent $Path }
    $names = @("cross-control$script:ExeSuffix")
    if ($script:IsWindowsHost) { $names += "cross-controlw$script:ExeSuffix" }

    $copies = foreach ($name in $names) {
        $source = Join-Path $sourceDir $name
        if (-not (Test-Path -LiteralPath $source)) { throw "$source not found." }
        $target = Join-Path $Destination $name
        $same = (Test-Path -LiteralPath $target) -and
            ((Get-FileHash -LiteralPath $source).Hash -eq (Get-FileHash -LiteralPath $target).Hash)
        if (-not $same) { [pscustomobject]@{ Source = $source; Target = $target } }
    }
    $exe = Join-Path $Destination "cross-control$script:ExeSuffix"
    $changed = @($copies).Count -gt 0
    $restarted = $false

    if ($changed -and $PSCmdlet.ShouldProcess($Destination, 'Install cross-control')) {
        $wasRunning = $false
        if (Test-Path -LiteralPath $exe) {
            $script:Exe = $exe
            $wasRunning = [bool](Get-CrossControlStatus).running
            if ($wasRunning) { Stop-CrossControl | Out-Null }
        }
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        foreach ($copy in $copies) {
            Copy-Item -LiteralPath $copy.Source -Destination $copy.Target -Force
            if (-not $script:IsWindowsHost) { & chmod 755 $copy.Target }
        }
        $script:Exe = $exe
        if ($wasRunning) {
            Start-CrossControl | Out-Null
            $restarted = $true
        }
    }
    elseif (Test-Path -LiteralPath $exe) {
        $script:Exe = $exe
    }

    $pathChanged = $false
    if ($AddToPath) {
        if (-not $script:IsWindowsHost) {
            Write-Warning 'Add -AddToPath is for Windows; add the directory to PATH in your shell profile.'
        }
        else {
            $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
            $entries = @($userPath -split ';' | Where-Object { $_ })
            if ($entries -notcontains $Destination -and
                $PSCmdlet.ShouldProcess('user PATH', "Add $Destination")) {
                [Environment]::SetEnvironmentVariable('Path', (($entries + $Destination) -join ';'), 'User')
                $env:Path = "$env:Path;$Destination"
                $pathChanged = $true
            }
        }
    }

    [pscustomobject]@{
        Changed     = $changed -or $pathChanged
        Destination = $Destination
        Restarted   = $restarted
    }
}

# --- Identity and configuration --------------------------------------------------

function Get-CrossControlIdentity {
    <#
    .SYNOPSIS
      This machine's name and certificate fingerprint, creating its identity
      on first use (Created = $true then).
    .EXAMPLE
      # On machine B, trust machine A:
      Invoke-Command -ComputerName A { Get-CrossControlIdentity } | Add-CrossControlTrust
    #>
    [CmdletBinding()]
    param()
    Invoke-CrossControl identity
}

function Set-CrossControlConfig {
    <#
    .SYNOPSIS
      Write config.toml from parameters; validated first, written only if
      different.
    .DESCRIPTION
      The file is checked with `cross-control config check` before it
      replaces the current one, so an invalid layout never lands. A running
      daemon reads the file at start: RestartNeeded says when to call
      Restart-CrossControl.
    .PARAMETER Screens
      Machines next to this one: @{ Name; Position = 'Left'|'Right'|'Above'|'Below'; Address (optional) }.
    .PARAMETER ScreenAdjacency
      Links between other machines: @{ Screen; Neighbor; Position }.
    .PARAMETER Monitors
      This machine's monitors: @{ Width; Height; X; Y }.
    .EXAMPLE
      Set-CrossControlConfig -Name desk -Screens @{ Name = 'laptop'; Position = 'Right'; Address = '192.0.2.20:24800' }
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [string]$Name = [System.Net.Dns]::GetHostName(),
        [object[]]$Screens = @(),
        [object[]]$ScreenAdjacency = @(),
        [object[]]$Monitors = @(),
        [ValidateRange(0, 65535)]
        [int]$Port = 24800,
        [string]$Bind = '0.0.0.0',
        [bool]$Discovery = $true,
        [ValidateSet('auto', 'evdev', 'windows', 'none')]
        [string]$InputBackend = 'auto',
        [string[]]$ReleaseHotkey = @('LeftCtrl', 'LeftShift', 'Escape'),
        [bool]$TrustOnFirstUse = $false,
        [bool]$Clipboard = $true,
        [string]$Certificate,
        [string]$Key,
        # Default: config.toml in the configuration directory.
        [string]$Path
    )
    if ([bool]$Certificate -ne [bool]$Key) {
        throw '-Certificate and -Key must be given together.'
    }
    if (-not $Path) { $Path = (Get-CrossControlIdentity).config }

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('# Managed by the CrossControl PowerShell module; changes here will be overwritten.')
    $lines.Add('')
    $lines.Add('[identity]')
    $lines.Add("name = $(ConvertTo-TomlString $Name)")
    if ($Certificate) {
        $lines.Add("cert = $(ConvertTo-TomlString $Certificate)")
        $lines.Add("key = $(ConvertTo-TomlString $Key)")
    }
    $lines.Add('')
    $lines.Add('[daemon]')
    $lines.Add("port = $Port")
    $lines.Add("bind = $(ConvertTo-TomlString $Bind)")
    $lines.Add("discovery = $(ConvertTo-TomlBool $Discovery)")
    $lines.Add('')
    $lines.Add('[input]')
    $lines.Add("backend = $(ConvertTo-TomlString $InputBackend)")
    $hotkey = ($ReleaseHotkey | ForEach-Object { ConvertTo-TomlString $_ }) -join ', '
    $lines.Add("release_hotkey = [$hotkey]")
    $lines.Add('')
    $lines.Add('[clipboard]')
    $lines.Add("enabled = $(ConvertTo-TomlBool $Clipboard)")
    $lines.Add('')
    $lines.Add('[security]')
    $lines.Add("trust_on_first_use = $(ConvertTo-TomlBool $TrustOnFirstUse)")
    foreach ($monitor in $Monitors) {
        $lines.Add('')
        $lines.Add('[[monitors]]')
        $lines.Add("width = $([int](Get-Field $monitor 'Width'))")
        $lines.Add("height = $([int](Get-Field $monitor 'Height'))")
        $lines.Add("x = $([int](Get-Field $monitor 'X' 0))")
        $lines.Add("y = $([int](Get-Field $monitor 'Y' 0))")
    }
    foreach ($screen in $Screens) {
        $lines.Add('')
        $lines.Add('[[screens]]')
        $lines.Add("name = $(ConvertTo-TomlString (Get-Field $screen 'Name'))")
        $lines.Add("position = $(ConvertTo-TomlString (Get-Field $screen 'Position'))")
        $address = Get-Field $screen 'Address'
        if ($address) { $lines.Add("address = $(ConvertTo-TomlString $address)") }
    }
    foreach ($link in $ScreenAdjacency) {
        $lines.Add('')
        $lines.Add('[[screen_adjacency]]')
        $lines.Add("screen = $(ConvertTo-TomlString (Get-Field $link 'Screen'))")
        $lines.Add("neighbor = $(ConvertTo-TomlString (Get-Field $link 'Neighbor'))")
        $lines.Add("position = $(ConvertTo-TomlString (Get-Field $link 'Position'))")
    }
    $content = ($lines -join "`n") + "`n"

    $changed = -not (Test-FileContent $Path $content)
    if ($changed) {
        # Validate before replacing anything.
        $candidate = "$Path.candidate"
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
        [System.IO.File]::WriteAllText($candidate, $content)
        try {
            Invoke-CrossControl config, check, $candidate | Out-Null
            if ($PSCmdlet.ShouldProcess($Path, 'Write cross-control configuration')) {
                Move-Item -LiteralPath $candidate -Destination $Path -Force
            }
        }
        finally {
            Remove-Item -LiteralPath $candidate -ErrorAction SilentlyContinue
        }
    }
    [pscustomobject]@{
        Changed       = $changed
        Path          = $Path
        RestartNeeded = $changed -and [bool](Get-CrossControlStatus).running
    }
}

# --- Trust -----------------------------------------------------------------

function Get-CrossControlTrust {
    <#
    .SYNOPSIS
      The machines this one trusts (Source: known_peers, or config for
      fingerprints pinned in config.toml).
    #>
    [CmdletBinding()]
    param()
    @((Invoke-CrossControl trust, list).trusted)
}

function Add-CrossControlTrust {
    <#
    .SYNOPSIS
      Trust a machine's certificate. Re-adding an identical entry changes
      nothing.
    .EXAMPLE
      Add-CrossControlTrust -Name laptop -Fingerprint 'SHA256:ab:cd:...'
    .EXAMPLE
      Invoke-Command -ComputerName laptop { Get-CrossControlIdentity } | Add-CrossControlTrust
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [Parameter(Mandatory, ValueFromPipelineByPropertyName)]
        [string]$Name,
        [Parameter(Mandatory, ValueFromPipelineByPropertyName)]
        [string]$Fingerprint
    )
    process {
        if ($PSCmdlet.ShouldProcess($Name, 'Trust')) {
            $result = Invoke-CrossControl trust, add, $Name, $Fingerprint
            [pscustomobject]@{ Changed = [bool]$result.changed; Name = $Name }
        }
    }
}

function Remove-CrossControlTrust {
    <#
    .SYNOPSIS
      Stop trusting a machine. Fingerprints pinned in config.toml are not
      touched.
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [Parameter(Mandatory, ValueFromPipelineByPropertyName)]
        [string]$Name
    )
    process {
        if ($PSCmdlet.ShouldProcess($Name, 'Stop trusting')) {
            $result = Invoke-CrossControl trust, remove, $Name
            [pscustomobject]@{ Changed = [bool]$result.changed; Name = $Name }
        }
    }
}

function Set-CrossControlTrust {
    <#
    .SYNOPSIS
      Make the trusted machines exactly -Peers: add what is missing or
      different, remove the rest.
    .DESCRIPTION
      Authoritative, like the Ansible role: a machine dropped from the list
      stops being trusted. Only known_peers entries are managed; fingerprints
      pinned in config.toml stay. A running daemon picks the change up on
      the next connection; no restart needed.
    .PARAMETER Peers
      Objects with Name and Fingerprint, e.g. from Get-CrossControlIdentity
      on each machine.
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]]$Peers
    )
    $self = (Invoke-CrossControl trust, list).self.fingerprint
    $wanted = @{}
    foreach ($peer in $Peers) {
        $fingerprint = Get-Field $peer 'Fingerprint'
        if ($fingerprint -eq $self) { continue }  # this machine itself
        $wanted[(Get-Field $peer 'Name')] = $fingerprint
    }
    $current = @(Get-CrossControlTrust | Where-Object { $_.source -eq 'known_peers' })
    $changed = $false
    foreach ($entry in $current) {
        if (-not $wanted.ContainsKey($entry.name)) {
            if ((Remove-CrossControlTrust -Name $entry.name).Changed) { $changed = $true }
        }
    }
    foreach ($name in $wanted.Keys) {
        $result = Add-CrossControlTrust -Name $name -Fingerprint $wanted[$name]
        if ($result -and $result.Changed) { $changed = $true }
    }
    [pscustomobject]@{ Changed = $changed; Trusted = @($wanted.Keys | Sort-Object) }
}

# --- Running ---------------------------------------------------------------

function Get-CrossControlStatus {
    <#
    .SYNOPSIS
      Whether the daemon is running, its PID, focus and connected peers.
    #>
    [CmdletBinding()]
    param()
    Invoke-CrossControl status -AllowedExitCodes 0, 3
}

function Test-CrossControl {
    <#
    .SYNOPSIS
      Check this machine is ready (`cross-control doctor`). Returns the
      checks; with -Quiet, just whether none failed.
    #>
    [CmdletBinding()]
    param([switch]$Quiet)
    $result = Invoke-CrossControl doctor -AllowedExitCodes 0, 1
    if ($Quiet) { return [bool]$result.ok }
    $result.checks
}

function Start-CrossControl {
    <#
    .SYNOPSIS
      Start the daemon in the background and wait until it runs.
    .DESCRIPTION
      On Windows, through the logon task if it is registered, else by
      starting cross-controlw directly (no console window). Elsewhere, the
      systemd user service if installed, else a background process.
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [string]$ConfigPath,
        [int]$TimeoutSeconds = 30
    )
    $status = Get-CrossControlStatus
    if ($status.running) {
        return [pscustomobject]@{ Changed = $false; Pid = $status.pid }
    }
    if (-not $PSCmdlet.ShouldProcess('cross-control', 'Start')) { return }
    $exe = Get-CrossControlExe
    $startArgs = @()
    if ($ConfigPath) { $startArgs += @('--config', $ConfigPath) }

    if ($script:IsWindowsHost) {
        $hasTask = Invoke-NativeQuiet schtasks.exe '/Query', '/TN', 'cross-control'
        if ($hasTask -and -not $ConfigPath) {
            if (-not (Invoke-NativeQuiet schtasks.exe '/Run', '/TN', 'cross-control')) {
                throw 'Could not run the cross-control logon task.'
            }
        }
        else {
            $launcher = Join-Path (Split-Path -Parent $exe) 'cross-controlw.exe'
            $quoted = $startArgs | ForEach-Object { '"' + ($_ -replace '"', '\"') + '"' }
            Start-Process -FilePath $launcher -ArgumentList $quoted | Out-Null
        }
    }
    else {
        $hasUnit = (Get-Command systemctl -ErrorAction SilentlyContinue) -and
            (Invoke-NativeQuiet systemctl '--user', 'cat', 'cross-control.service')
        if ($hasUnit -and -not $ConfigPath) {
            if (-not (Invoke-NativeQuiet systemctl '--user', 'start', 'cross-control.service')) {
                throw 'Could not start the cross-control systemd user service.'
            }
        }
        else {
            $log = Join-Path ([System.IO.Path]::GetTempPath()) "cross-control-$PID.log"
            Start-Process -FilePath $exe -ArgumentList (@('start') + $startArgs) `
                -RedirectStandardError $log -RedirectStandardOutput ([System.IO.Path]::ChangeExtension($log, '.out')) | Out-Null
        }
    }
    $status = Wait-CrossControlState -Running $true -TimeoutSeconds $TimeoutSeconds
    [pscustomobject]@{ Changed = $true; Pid = $status.pid }
}

function Stop-CrossControl {
    <#
    .SYNOPSIS
      Stop the daemon cleanly and wait until it has exited.
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param()
    if (-not (Get-CrossControlStatus).running) {
        return [pscustomobject]@{ Changed = $false; Graceful = $true }
    }
    if (-not $PSCmdlet.ShouldProcess('cross-control', 'Stop')) { return }
    $result = Invoke-CrossControl stop -AllowedExitCodes 0, 3
    if ($LASTEXITCODE -eq 3) {
        return [pscustomobject]@{ Changed = $false; Graceful = $true }
    }
    [pscustomobject]@{ Changed = $true; Graceful = [bool]$result.graceful; Pid = $result.stopped }
}

function Restart-CrossControl {
    <#
    .SYNOPSIS
      Stop the daemon if it runs, then start it.
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param([string]$ConfigPath)
    if ($PSCmdlet.ShouldProcess('cross-control', 'Restart')) {
        Stop-CrossControl | Out-Null
        $started = if ($ConfigPath) { Start-CrossControl -ConfigPath $ConfigPath } else { Start-CrossControl }
        [pscustomobject]@{ Changed = $true; Pid = $started.Pid }
    }
}

# --- Autostart -------------------------------------------------------------

function Enable-CrossControlAutostart {
    <#
    .SYNOPSIS
      Start the daemon at logon: a Task Scheduler logon task on Windows (a
      systemd user service on Linux). Unchanged definitions are left alone.
    .PARAMETER Start
      Also start it now.
    .PARAMETER ConfigPath
      Pass --config to the daemon. Directory overrides
      (CROSS_CONTROL_CONFIG_DIR / CROSS_CONTROL_RUNTIME_DIR) set in this
      session are carried over too.
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [switch]$Start,
        [string]$ConfigPath
    )
    $arguments = @('service', 'install')
    if ($Start) { $arguments += '--enable' }
    if ($ConfigPath) { $arguments += @('--config', $ConfigPath) }
    if ($PSCmdlet.ShouldProcess('cross-control', 'Enable autostart')) {
        $result = Invoke-CrossControl $arguments
        [pscustomobject]@{ Changed = [bool]$result.changed; Started = [bool]$Start }
    }
}

function Disable-CrossControlAutostart {
    <#
    .SYNOPSIS
      Stop the daemon and remove the logon task (or systemd user service).
    #>
    [CmdletBinding(SupportsShouldProcess)]
    param()
    if ($PSCmdlet.ShouldProcess('cross-control', 'Disable autostart')) {
        $result = Invoke-CrossControl service, uninstall
        [pscustomobject]@{ Changed = [bool]$result.changed }
    }
}

Export-ModuleMember -Function @(
    'Get-CrossControlExe'
    'Invoke-CrossControl'
    'Install-CrossControl'
    'Get-CrossControlIdentity'
    'Set-CrossControlConfig'
    'Get-CrossControlTrust'
    'Add-CrossControlTrust'
    'Remove-CrossControlTrust'
    'Set-CrossControlTrust'
    'Get-CrossControlStatus'
    'Test-CrossControl'
    'Start-CrossControl'
    'Stop-CrossControl'
    'Restart-CrossControl'
    'Enable-CrossControlAutostart'
    'Disable-CrossControlAutostart'
)
