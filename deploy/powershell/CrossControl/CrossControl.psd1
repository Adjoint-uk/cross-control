@{
    RootModule        = 'CrossControl.psm1'
    ModuleVersion     = '0.2.0'
    GUID              = '6f0b8a43-2f4e-4f39-9b8f-6d0a3c1e7c55'
    Author            = 'cross-control contributors'
    Copyright         = '(c) cross-control contributors. MIT or Apache-2.0.'
    Description       = 'Install, configure and run cross-control (software KVM) idempotently, for scripted and fleet deployment.'
    PowerShellVersion = '5.1'
    CompatiblePSEditions = @('Desktop', 'Core')
    FunctionsToExport = @(
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
    CmdletsToExport   = @()
    VariablesToExport = @()
    AliasesToExport   = @()
    PrivateData       = @{
        PSData = @{
            Prerelease = ''
            Tags       = @('KVM', 'keyboard', 'mouse', 'cross-control', 'Windows', 'Linux')
            ProjectUri = 'https://github.com/Adjoint-uk/cross-control'
        }
    }
}
