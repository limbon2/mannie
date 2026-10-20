# The normal interactive task started by run-check.ps1. Its configuration
# carries command paths and a fixed list of non-secret build settings only.
param(
    [Parameter(Mandatory)]
    [string] $Configuration
)
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$configurationData = Get-Content -LiteralPath $Configuration -Raw | ConvertFrom-Json
$exitCode = 1
try {
    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'run-check-task: the check must run with the normal interactive token'
        }
    }
    foreach ($property in $configurationData.Environment.PSObject.Properties) {
        [Environment]::SetEnvironmentVariable($property.Name, [string]$property.Value, 'Process')
    }
    $process = Get-Process -Id $PID
    $pendingStart = $configurationData.Started + '.pending'
    @{ Id = $PID; StartedUtcTicks = $process.StartTime.ToUniversalTime().Ticks } |
        ConvertTo-Json | Set-Content -LiteralPath $pendingStart -Encoding UTF8
    Move-Item -LiteralPath $pendingStart -Destination $configurationData.Started
    # The parent records our process before allowing a command to start, so
    # cancellation can end its whole process tree without a PID handover race.
    $deadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path -LiteralPath $configurationData.StartMarker)) {
        if ((Get-Date) -ge $deadline) { throw 'run-check-task: the parent did not acknowledge startup within 30 seconds' }
        Start-Sleep -Milliseconds 100
    }
    Set-Location -LiteralPath $configurationData.Directory
    $arguments = @($configurationData.Arguments)
    & $configurationData.Executable @arguments *> $configurationData.Log
    $exitCode = $LASTEXITCODE
} catch {
    $_ | Out-String | Add-Content -LiteralPath $configurationData.Log
} finally {
    $pending = $configurationData.Result + '.pending'
    [pscustomobject]@{ ExitCode = $exitCode } | ConvertTo-Json | Set-Content -LiteralPath $pending -Encoding UTF8
    Move-Item -LiteralPath $pending -Destination $configurationData.Result
}
exit $exitCode
