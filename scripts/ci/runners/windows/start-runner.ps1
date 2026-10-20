# Runs the GitHub Actions runner for one job, then shuts the machine down
# (docs/engineering/ci-runners.md, section 6). The startup task that
# setup-runner-image.ps1 registers starts it at every sign-in of the
# administrator who signs in automatically, with that administrator's full
# rights, so the jobs have a desktop and may install services.
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

# The host's run-runner-cycle.ps1 copies the files into this folder.
$folder = Join-Path $env:ProgramData 'mannie-runner'
$configFile = Join-Path $folder 'jitconfig'
$readyFile = Join-Path $folder 'ready'
$log = Join-Path $folder 'start-runner.log'
$runnerFolder = 'C:\actions-runner'

function Write-Log {
    param(
        [Parameter(Mandatory)]
        [string] $Text
    )

    Add-Content -LiteralPath $log -Value ('{0:yyyy-MM-dd HH:mm:ss} {1}' -f (Get-Date), $Text)
}

Set-Content -LiteralPath $log -Value ('{0:yyyy-MM-dd HH:mm:ss} waiting for the runner configuration from the host' -f (Get-Date))
# The host copies the marker after the configuration, so the configuration is
# complete once the marker is there.
while (-not (Test-Path -LiteralPath $readyFile)) {
    Start-Sleep -Seconds 2
}

# The configuration registers this machine as the runner of one job, so it
# leaves the disk before the runner starts. The runner reads it from this
# variable, masks it in its logs and removes it from the environment that the
# job's steps inherit; on the command line any process could read it.
$runnerName = (Get-Content -LiteralPath $readyFile -Raw).Trim()
$env:ACTIONS_RUNNER_INPUT_JITCONFIG = (Get-Content -LiteralPath $configFile -Raw).Trim()
Remove-Item -LiteralPath $configFile, $readyFile -Force
Write-Log "received the configuration of the runner $runnerName"

# Windows PowerShell put the user's module folder in front of PSModulePath.
# The runner and its jobs get the value that a new sign-in gives.
$env:PSModulePath = [Environment]::GetEnvironmentVariable('PSModulePath', 'Machine')

try {
    & (Join-Path $runnerFolder 'run.cmd')
    Write-Log "the runner exited with code $LASTEXITCODE"
} catch {
    Write-Log "the runner did not run: $($_.Exception.Message)"
} finally {
    Remove-Item -LiteralPath Env:\ACTIONS_RUNNER_INPUT_JITCONFIG -ErrorAction SilentlyContinue
    Write-Log 'shutting down'
    Stop-Computer -Force
}
