# Installs the host controller in ProgramData and registers its sign-in task.
# Run elevated as the Windows user whose DPAPI-protected GitHub token serves
# the repository. Guest setup and clean checkpoints must already be complete.
param(
    [string] $MachineNames = 'mannie-ci-windows-11,mannie-ci-ubuntu-24.04,mannie-ci-ubuntu-22.04',
    [ValidateRange(1, 16)]
    [int] $MaxRunning = 1,
    [switch] $Start
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'runner-host.psm1') -Force

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'install-controller: run elevated as the user who saved the GitHub token'
}
$taskName = 'Mannie CI runners'
$existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
if ($existing -and $existing.State -eq 'Running') {
    throw "install-controller: stop the task '$taskName' after its current job finishes before updating it"
}

$template = Import-PowerShellDataFile -LiteralPath (Join-Path $PSScriptRoot 'runners.example.psd1')
$names = @($MachineNames.Split(',') | ForEach-Object { $_.Trim() })
if ($names.Count -eq 0 -or @($names | Where-Object { -not $_ }).Count -gt 0 -or @($names | Select-Object -Unique).Count -ne $names.Count) {
    throw 'install-controller: MachineNames must list distinct, nonempty machine names separated by commas'
}
$machines = @(
    foreach ($name in $names) {
        $entry = @($template.Machines | Where-Object { $_.Name -eq $name })
        if ($entry.Count -ne 1) {
            throw "install-controller: $name is not in runners.example.psd1"
        }
        Get-VM -Name $name | Out-Null
        Get-VMSnapshot -VMName $name -Name $template.Checkpoint | Out-Null
        $problems = @(Get-RunnerMachineProblems -Machine $name)
        if ($problems.Count -gt 0) {
            throw "install-controller: $name is not ready: $($problems -join '; ')"
        }
        $entry[0]
    }
)
Connect-GitHub
Test-RepositoryAccess -Repository $template.Repository

# Only one controller may update or use this installation. Release the lock
# before starting its scheduled task, which acquires the same lock itself.
$controllerLock = Enter-ControllerLock -Repository $template.Repository
try {
    $folder = Join-Path $env:ProgramData 'Mannie\ci-runners\host'
    New-Item -ItemType Directory -Path $folder -Force | Out-Null
    $acl = [System.Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $administrators = [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
    $acl.SetOwner($administrators)
    foreach ($sid in 'S-1-5-18', 'S-1-5-32-544') {
        $reader = [System.Security.Principal.SecurityIdentifier]::new($sid)
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($reader, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
    }
    Set-Acl -LiteralPath $folder -AclObject $acl
    foreach ($file in 'run-runners.ps1', 'run-runner-cycle.ps1', 'runner-host.psm1', 'isolate-runner-machine.ps1', 'save-github-token.ps1') {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $file) -Destination (Join-Path $folder $file) -Force
    }

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('@{')
    $lines.Add("    Repository = '$($template.Repository)'")
    $lines.Add("    Checkpoint = '$($template.Checkpoint)'")
    $lines.Add("    MaxRunning = $MaxRunning")
    $lines.Add('    Machines = @(')
    foreach ($machine in $machines) {
        $lines.Add('        @{')
        $lines.Add("            Name = '$($machine.Name)'")
        $lines.Add("            Guest = '$($machine.Guest)'")
        $labels = @($machine.Labels | ForEach-Object { "'$_'" }) -join ', '
        $lines.Add("            Labels = @($labels)")
        $lines.Add('        }')
    }
    $lines.Add('    )')
    $lines.Add('}')
    $configPath = Get-DefaultConfigPath
    Set-Content -LiteralPath $configPath -Value $lines -Encoding UTF8
    Read-RunnerConfig -Path $configPath | Out-Null

    $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $script = Join-Path $folder 'run-runners.ps1'
    $action = New-ScheduledTaskAction -Execute $powershell -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$script`""
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $identity.Name
    $taskPrincipal = New-ScheduledTaskPrincipal -UserId $identity.Name -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 4 -MultipleInstances IgnoreNew -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable
    Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $taskPrincipal -Settings $settings -Force | Out-Null
    Write-Output "install-controller: installed $script for $($identity.Name) with $($names -join ', '), at most $MaxRunning at once"
} finally {
    $controllerLock.ReleaseMutex()
    $controllerLock.Dispose()
}
if ($Start) {
    Start-ScheduledTask -TaskName $taskName
    Write-Output "install-controller: started '$taskName'"
}
