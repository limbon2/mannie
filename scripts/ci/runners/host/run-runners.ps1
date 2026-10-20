# Runs the self-hosted runners on demand (docs/engineering/ci-runners.md,
# section 6). Every 30 seconds it reads the repository's queued jobs, and for a
# job that one of the configured machines can take it starts
# run-runner-cycle.ps1 on that machine in the background, with at most
# MaxRunning machines at once. Run it as the Windows user that saved the GitHub
# token, as an administrator or a member of Hyper-V Administrators:
#   ./scripts/ci/runners/host/run-runners.ps1
# Ctrl+C stops it. A cycle that is running then stops too: its machine is
# turned off, and a job that was running on it fails.
param(
    [string] $ConfigPath
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'runner-host.psm1') -Force

$pollSeconds = 30
# A machine whose cycle ended without a job waits before its next cycle: one
# minute, then twice as long after each such cycle, up to this.
$longestPauseMinutes = 30

if (-not $ConfigPath) {
    $ConfigPath = Get-DefaultConfigPath
}
$config = Read-RunnerConfig -Path $ConfigPath
$repository = $config.Repository
$cycleScript = Join-Path $PSScriptRoot 'run-runner-cycle.ps1'
$logPath = Join-Path (Get-RunnerSettingsFolder) 'runners.log'

$machines = @(
    foreach ($entry in @($config.Machines)) {
        [pscustomobject]@{
            Name            = $entry.Name
            Labels          = [string[]] @($entry.Labels)
            # The background job of the machine's cycle.
            Job             = $null
            TookJob         = $false
            # Running a job that began before this script started.
            Inherited       = $false
            # Running without a cycle, so the owner may be working on it.
            ReportedRunning = $false
            FailedCycles    = 0
            NotBefore       = [datetime]::MinValue
        }
    }
)
# Problems that last are logged when they appear and when they change, not on
# every round.
$problems = @{}
# Queued jobs that no machine can take, logged once each.
$reportedJobs = @{}

function Add-LogLine {
    param(
        [Parameter(Mandatory)]
        [string] $Line
    )

    # A reader, such as a tail of the log, may have the file open for a moment.
    # The line is tried a few times, and a log that stays locked costs the line
    # in the file, never the runners.
    for ($attempt = 1; $attempt -le 5; $attempt += 1) {
        try {
            $stream = [System.IO.File]::Open($logPath, [System.IO.FileMode]::Append, [System.IO.FileAccess]::Write, [System.IO.FileShare]::ReadWrite)
            try {
                $bytes = [System.Text.Encoding]::UTF8.GetBytes($Line + [Environment]::NewLine)
                $stream.Write($bytes, 0, $bytes.Length)
            } finally {
                $stream.Dispose()
            }
            return
        } catch [System.IO.IOException] {
            Start-Sleep -Milliseconds 200
        }
    }
    Write-Host "could not write the line above to $logPath, which another process holds"
}

function Write-RunnerLog {
    param(
        [Parameter(Mandatory)]
        [string] $Text
    )

    $line = Format-RunnerLog $Text
    Write-Host $line
    Add-LogLine -Line $line
}

function Write-RunnerProblem {
    param(
        [Parameter(Mandatory)]
        [string] $Key,
        [Parameter(Mandatory)]
        [string] $Text
    )

    if ($problems[$Key] -ne $Text) {
        $problems[$Key] = $Text
        Write-RunnerLog $Text
    }
}

function Clear-RunnerProblem {
    param(
        [Parameter(Mandatory)]
        [string] $Key,
        [Parameter(Mandatory)]
        [string] $Text
    )

    if ($problems.ContainsKey($Key)) {
        $problems.Remove($Key)
        Write-RunnerLog $Text
    }
}

function Get-MachineRunners {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Machine,
        [object[]] $Runners
    )

    return @($Runners | Where-Object { Test-RunnerOfMachine -RunnerName $_.name -Machine $Machine.Name })
}

function Remove-MachineRunners {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Machine,
        [object[]] $Runners
    )

    foreach ($runner in @(Get-MachineRunners -Machine $Machine -Runners $Runners)) {
        if (Remove-Runner -Repository $repository -Id $runner.id) {
            Write-RunnerLog ('{0}: removed the runner {1}, which an earlier cycle left behind' -f $Machine.Name, $runner.name)
        }
    }
}

function Reset-Machine {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Machine,
        [object[]] $Runners
    )

    # This script may have stopped while its cycles ran. A runner that is
    # running a job keeps its machine until the guest powers off; anything else
    # is turned off and removed.
    $own = @(Get-MachineRunners -Machine $Machine -Runners $Runners)
    $vm = Get-VM -Name $Machine.Name
    if ($vm.State -ne 'Off' -and @($own | Where-Object { $_.busy }).Count -gt 0) {
        $Machine.Inherited = $true
        Write-RunnerLog ('{0}: is running a job that began before this script started; it is left to finish' -f $Machine.Name)
        return
    }
    if ($vm.State -ne 'Off') {
        Stop-RunnerMachine -Machine $Machine.Name
        Write-RunnerLog ('{0}: turned off the machine, which was running without a job' -f $Machine.Name)
    }
    Remove-MachineRunners -Machine $Machine -Runners $own
}

function Receive-Cycle {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Machine
    )

    # Receive-Job shows the cycle's log lines on this console; they are added
    # to the log file here.
    $information = $null
    $failures = $null
    $output = @(Receive-Job -Job $Machine.Job -InformationVariable information -ErrorVariable failures -ErrorAction SilentlyContinue)
    foreach ($record in @($information | Where-Object { $_ })) {
        Add-LogLine -Line "$($record.MessageData)"
    }
    foreach ($failure in @($failures | Where-Object { $_ })) {
        Write-RunnerLog ('{0}: the cycle failed: {1}' -f $Machine.Name, $failure)
    }
    foreach ($item in $output) {
        if ($item.PSObject.Properties['TookJob']) {
            $Machine.TookJob = [bool] $item.TookJob
        }
    }
}

function Update-Machine {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Machine
    )

    if ($Machine.Job) {
        Receive-Cycle -Machine $Machine
        if ($Machine.Job.State -in 'Completed', 'Failed', 'Stopped') {
            Remove-Job -Job $Machine.Job -Force
            $Machine.Job = $null
            if ($Machine.TookJob) {
                $Machine.FailedCycles = 0
                $Machine.NotBefore = [datetime]::MinValue
            } else {
                # A machine that cannot run a job, or a job that went to another
                # runner, does not start the machine again and again.
                $Machine.FailedCycles += 1
                $minutes = [math]::Min([math]::Pow(2, $Machine.FailedCycles - 1), $longestPauseMinutes)
                $Machine.NotBefore = (Get-Date).AddMinutes($minutes)
                $pause = '{0} minutes' -f $minutes
                if ($minutes -eq 1) {
                    $pause = 'a minute'
                }
                Write-RunnerLog ('{0}: the cycle ended without a job; the machine starts again in {1} at the earliest' -f $Machine.Name, $pause)
            }
        }
        return
    }
    if ($Machine.Inherited -and (Get-VM -Name $Machine.Name).State -eq 'Off') {
        $Machine.Inherited = $false
        Write-RunnerLog ('{0}: the job that began before this script started is over' -f $Machine.Name)
        Remove-MachineRunners -Machine $Machine -Runners @(Get-RepositoryRunners -Repository $repository)
    }
    if ($Machine.ReportedRunning -and (Get-VM -Name $Machine.Name).State -eq 'Off') {
        $Machine.ReportedRunning = $false
    }
}

function Start-Cycle {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Machine,
        [Parameter(Mandatory)]
        [object] $Job
    )

    Write-RunnerLog ("{0}: starting a cycle for the job '{1}', {2}" -f $Machine.Name, $Job.name, $Job.html_url)
    $Machine.TookJob = $false
    $Machine.Job = Start-Job -Name "runner cycle $($Machine.Name)" -ScriptBlock {
        param($Script, $Machine, $ConfigPath)
        & $Script -Machine $Machine -ConfigPath $ConfigPath
    } -ArgumentList $cycleScript, $Machine.Name, $ConfigPath
}

function Start-NeededCycles {
    param(
        [object[]] $Queued,
        [object[]] $Runners
    )

    $now = Get-Date
    $byName = @{}
    $states = @(
        foreach ($machine in $machines) {
            $byName[$machine.Name] = $machine
            $inCycle = $null -ne $machine.Job
            $running = $false
            if (-not $inCycle -and -not $machine.Inherited) {
                $running = (Get-VM -Name $machine.Name).State -ne 'Off'
            }
            [pscustomobject]@{
                Name      = $machine.Name
                Labels    = $machine.Labels
                InCycle   = $inCycle
                Inherited = $machine.Inherited
                Paused    = $machine.NotBefore -gt $now
                Running   = $running
            }
        }
    )
    $plan = Select-NeededCycles -Machines $states -Queued $Queued -Runners $Runners -MaxRunning $config.MaxRunning
    foreach ($start in $plan.Starts) {
        Start-Cycle -Machine $byName[$start.Machine.Name] -Job $start.Job
    }
    foreach ($state in $plan.Blocked) {
        $machine = $byName[$state.Name]
        if (-not $machine.ReportedRunning) {
            $machine.ReportedRunning = $true
            Write-RunnerLog ('{0}: is running without a cycle, so it takes no job until it is off' -f $machine.Name)
        }
    }

    $stillQueued = @{}
    foreach ($job in $Queued) {
        $stillQueued[[string] $job.id] = $true
    }
    foreach ($job in $plan.Unserved) {
        $id = [string] $job.id
        if (-not $reportedJobs.ContainsKey($id)) {
            $reportedJobs[$id] = $true
            Write-RunnerLog ("no machine has the labels {0} that the job '{1}' asks for, {2}" -f (@($job.labels) -join ', '), $job.name, $job.html_url)
        }
    }
    foreach ($id in @($reportedJobs.Keys)) {
        if (-not $stillQueued.ContainsKey($id)) {
            $reportedJobs.Remove($id)
        }
    }
}

function Stop-Cycles {
    foreach ($machine in @($machines | Where-Object { $_.Job })) {
        Stop-Job -Job $machine.Job
        # The cycle cleans up in its own process, which may end with this
        # script before it is done, so the cleanup runs here as well.
        try {
            Stop-RunnerMachine -Machine $machine.Name
            Remove-MachineRunners -Machine $machine -Runners @(Get-RepositoryRunners -Repository $repository)
            Write-RunnerLog ('{0}: stopped the cycle and turned the machine off' -f $machine.Name)
        } catch {
            Write-RunnerLog ('{0}: could not clean up after the stopped cycle: {1}' -f $machine.Name, $_.Exception.Message)
        }
        Remove-Job -Job $machine.Job -Force
        $machine.Job = $null
    }
}


$controllerLock = Enter-ControllerLock -Repository $repository
try {
    Connect-GitHub
    Test-RepositoryAccess -Repository $repository
    foreach ($machine in $machines) {
        try {
            Get-VM -Name $machine.Name | Out-Null
        } catch {
            throw "run-runners: cannot read the machine $($machine.Name); run as an administrator or a member of Hyper-V Administrators, and check its name in $($ConfigPath): $($_.Exception.Message)"
        }
        if (-not (Get-VMSnapshot -VMName $machine.Name -Name $config.Checkpoint -ErrorAction SilentlyContinue)) {
            throw "run-runners: $($machine.Name) has no checkpoint named $($config.Checkpoint)"
        }
    }
    Write-RunnerLog "serving $repository with $(@($machines.Name) -join ', '), at most $($config.MaxRunning) at once"
    $runners = @(Get-RepositoryRunners -Repository $repository)
    foreach ($machine in $machines) {
        Reset-Machine -Machine $machine -Runners $runners
    }

    try {
        while ($true) {
            # A failure that may pass, such as GitHub or the network being away,
            # is logged and tried again in the next round.
            foreach ($machine in $machines) {
                try {
                    Update-Machine -Machine $machine
                    Clear-RunnerProblem -Key $machine.Name -Text ('{0}: works again' -f $machine.Name)
                } catch {
                    Write-RunnerProblem -Key $machine.Name -Text ('{0}: {1}' -f $machine.Name, $_.Exception.Message)
                }
            }
            try {
                $queued = @(Get-QueuedRunnerJobs -Repository $repository)
                $runners = @()
                if ($queued.Count -gt 0) {
                    $runners = @(Get-RepositoryRunners -Repository $repository)
                }
                Start-NeededCycles -Queued $queued -Runners $runners
                Clear-RunnerProblem -Key 'queue' -Text 'the queue is readable again'
            } catch {
                Write-RunnerProblem -Key 'queue' -Text "cannot serve the queue: $($_.Exception.Message)"
            }
            Start-Sleep -Seconds $pollSeconds
        }
    } finally {
        Stop-Cycles
        Write-RunnerLog 'stopped'
    }
} finally {
    $controllerLock.ReleaseMutex()
    $controllerLock.Dispose()
}
