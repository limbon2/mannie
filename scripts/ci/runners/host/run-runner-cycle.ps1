# One cycle of a disposable runner machine (docs/engineering/ci-runners.md,
# section 6): GitHub registers a just-in-time runner with the machine's labels,
# the machine starts from its clean checkpoint, the guest receives the runner's
# configuration and runs the runner for one job, and then the machine is turned
# off and the registration removed. run-runners.ps1 starts a cycle when a job
# for the machine waits. To run one cycle by hand, as the Windows user that
# saved the GitHub token:
#   ./scripts/ci/runners/host/run-runner-cycle.ps1 -Machine mannie-ci-ubuntu-24.04
param(
    [Parameter(Mandatory)]
    [string] $Machine,
    [string] $ConfigPath
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'runner-host.psm1') -Force

# The folders in which the guest's startup script waits for the files jitconfig
# and ready, which setup-runner-image.ps1 and setup-runner-image.sh create.
$guestInboxes = @{
    Windows = 'C:\ProgramData\mannie-runner'
    Ubuntu  = '/var/lib/mannie-runner'
}

# How long a restored guest may take until it accepts files from the host.
$bootLimit = New-TimeSpan -Minutes 10
# How long the runner may take to come online after the hand-over, and to take
# a job once it is online.
$connectLimit = New-TimeSpan -Minutes 10
$idleLimit = New-TimeSpan -Minutes 10
# How long an online runner may stay unreachable. GitHub fails a job whose
# runner stays away about as long.
$offlineLimit = New-TimeSpan -Minutes 10
# How long the guest may take to power off after GitHub removed its runner.
$shutdownLimit = New-TimeSpan -Minutes 5
# The longest cycle: the longest job timeout, 6 hours, with time to boot and to
# install the toolchains.
$cycleLimit = New-TimeSpan -Hours 7
$pollSeconds = 20

if (-not $ConfigPath) {
    $ConfigPath = Get-DefaultConfigPath
}
$config = Read-RunnerConfig -Path $ConfigPath
$entry = @($config.Machines | Where-Object { $_.Name -eq $Machine })
if ($entry.Count -eq 0) {
    throw "run-runner-cycle: $ConfigPath has no machine $Machine"
}
$entry = $entry[0]
$repository = $config.Repository
# The machine's worker process cannot reach into the user's profile, where the
# other settings live, so the files on their way into a guest wait in
# ProgramData (observed on 2026-10-17).
$handover = Join-Path $env:ProgramData "Mannie\ci-runners\handover\$Machine"

function Write-CycleLog {
    param(
        [Parameter(Mandatory)]
        [string] $Text
    )

    Write-Host (Format-RunnerLog ('{0}: {1}' -f $Machine, $Text))
}

function Start-CleanMachine {
    Stop-RunnerMachine -Machine $Machine
    Restore-VMSnapshot -VMName $Machine -Name $config.Checkpoint -Confirm:$false
    $problems = @(Get-RunnerMachineProblems -Machine $Machine)
    if ($problems.Count -gt 0) {
        throw "run-runner-cycle: the checkpoint $($config.Checkpoint) of $Machine is not ready for jobs: $($problems -join '; '). Fix the machine as docs/engineering/ci-runners.md, section 6, describes, and take the checkpoint again"
    }
    Start-VM -Name $Machine
}

function Initialize-Handover {
    # The machine's worker process, which runs as the machine's own account,
    # reads the file that Copy-VMFile copies (observed on 2026-10-17). The
    # folder lets in that account besides the owner, SYSTEM and the
    # administrators, and nobody else, since the file holds the runner's
    # configuration.
    New-Item -ItemType Directory -Force -Path $handover | Out-Null
    $vmId = (Get-VM -Name $Machine).VMId.ToString().ToUpperInvariant()
    $readers = @(
        [System.Security.Principal.WindowsIdentity]::GetCurrent().User
        [System.Security.Principal.SecurityIdentifier]::new('S-1-5-18')
        [System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
    )
    $acl = [System.Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($reader in $readers) {
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($reader, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
    }
    $machineAccount = [System.Security.Principal.NTAccount]::new('NT VIRTUAL MACHINE', $vmId)
    $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($machineAccount, 'Read', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
    Set-Acl -LiteralPath $handover -AclObject $acl
    # Files of a cycle that stopped before it could delete them.
    Get-ChildItem -LiteralPath $handover | Remove-Item -Force
}

function Copy-IntoGuest {
    param(
        [Parameter(Mandatory)]
        [string] $Name,
        [Parameter(Mandatory)]
        [string] $Content,
        [Parameter(Mandatory)]
        [datetime] $Deadline
    )

    # Copy-VMFile copies only from a file, so the content is on the host's disk
    # for each attempt and no longer. The file has the name that the guest
    # waits for: a Windows guest takes the whole destination path, while the
    # copy daemon of a Linux guest takes a folder and adds the source file's
    # name (observed on 2026-10-17). The inbox must exist: without
    # -CreateFullPath, a guest whose setup script never ran refuses the copy,
    # where it would otherwise create the inbox for the root user and wait for
    # a runner that it has no unit to start (observed on 2026-10-19).
    $source = Join-Path $handover $Name
    $inbox = $guestInboxes[$entry.Guest]
    $destination = $inbox + '/'
    if ($entry.Guest -eq 'Windows') {
        $destination = $inbox + "\$Name"
    }
    while ($true) {
        try {
            [System.IO.File]::WriteAllText($source, $Content)
            Copy-VMFile -Name $Machine -SourcePath $source -DestinationPath $destination -FileSource Host -Force
            return
        } catch {
            # The guest accepts files once it has booted and started its Guest
            # Service Interface.
            if ((Get-Date) -gt $Deadline) {
                throw "run-runner-cycle: could not copy $Name into $Machine within $($bootLimit.TotalMinutes) minutes: $($_.Exception.Message) Either the guest did not boot, or its image lacks $inbox because setup-runner-image did not finish"
            }
        } finally {
            if (Test-Path -LiteralPath $source) {
                Remove-Item -LiteralPath $source -Force
            }
        }
        Start-Sleep -Seconds 5
    }
}

function Send-RunnerConfig {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Runner
    )

    $deadline = (Get-Date) + $bootLimit
    Copy-IntoGuest -Name 'jitconfig' -Content $Runner.JitConfig -Deadline $deadline
    # The guest reads the configuration once the marker is there. The marker is
    # copied second, so the configuration is complete by then.
    Copy-IntoGuest -Name 'ready' -Content $Runner.Name -Deadline $deadline
}

function Wait-ForRunner {
    param(
        [Parameter(Mandatory)]
        [pscustomobject] $Runner,
        [Parameter(Mandatory)]
        [datetime] $Started
    )

    # Returns whether the runner took a job, which run-runners.ps1 uses to
    # decide how soon the machine may run again.
    $handedOver = Get-Date
    $online = $false
    $tookJob = $false
    $lastOnline = $handedOver
    $idleSince = $null
    $removedAt = $null
    $apiFailure = ''
    while ($true) {
        Start-Sleep -Seconds $pollSeconds
        $now = Get-Date
        $poweredOff = (Get-VM -Name $Machine).State -eq 'Off'

        if ($now - $Started -gt $cycleLimit) {
            Write-CycleLog "the cycle ran for longer than $($cycleLimit.TotalHours) hours"
            return $tookJob
        }

        try {
            $state = Get-Runner -Repository $repository -Id $Runner.Id
        } catch {
            # The machine keeps running while GitHub cannot be reached; the
            # limits above still end the cycle.
            if ($_.Exception.Message -ne $apiFailure) {
                $apiFailure = $_.Exception.Message
                Write-CycleLog "cannot read the runner's state: $apiFailure"
            }
            if ($poweredOff) {
                Write-CycleLog 'the guest powered off'
                return $tookJob
            }
            continue
        }
        if ($apiFailure) {
            $apiFailure = ''
            Write-CycleLog "the runner's state is readable again"
        }

        # A short job may finish and shut down between polls. Read GitHub
        # before returning for a stopped guest, so its removed registration
        # still records that the machine took a job.
        if ($null -eq $state) {
            # GitHub removes a just-in-time runner once its job is done.
            if ($null -eq $removedAt) {
                $removedAt = $now
                $tookJob = $true
                Write-CycleLog 'GitHub removed the runner after its job'
            } elseif ($now - $removedAt -gt $shutdownLimit) {
                Write-CycleLog "the guest did not power off within $($shutdownLimit.TotalMinutes) minutes after the job"
                return $tookJob
            }
        } elseif ($state.status -eq 'online') {
            if (-not $online) {
                $online = $true
                Write-CycleLog 'the runner is online'
            }
            $lastOnline = $now
            if ($state.busy) {
                if (-not $tookJob) {
                    $tookJob = $true
                    Write-CycleLog 'the runner took a job'
                }
            } elseif (-not $tookJob) {
                if ($null -eq $idleSince) {
                    $idleSince = $now
                } elseif ($now - $idleSince -gt $idleLimit) {
                    Write-CycleLog "the runner took no job within $($idleLimit.TotalMinutes) minutes"
                    return $tookJob
                }
            }
        } elseif (-not $online) {
            if ($now - $handedOver -gt $connectLimit) {
                Write-CycleLog "the runner did not come online within $($connectLimit.TotalMinutes) minutes"
                return $tookJob
            }
        } elseif ($now - $lastOnline -gt $offlineLimit) {
            Write-CycleLog "the runner was offline for $($offlineLimit.TotalMinutes) minutes"
            return $tookJob
        }
        if ($poweredOff) {
            Write-CycleLog 'the guest powered off'
            return $tookJob
        }
    }
}

$started = Get-Date
Initialize-Handover

Connect-GitHub
$runner = New-JitRunner -Repository $repository -Name (New-RunnerName -Machine $Machine) -Labels $entry.Labels
Write-CycleLog "GitHub registered the runner $($runner.Name) with the labels $($entry.Labels -join ', ')"
$tookJob = $false
try {
    Start-CleanMachine
    Write-CycleLog "restored the checkpoint $($config.Checkpoint) and started the machine"
    Send-RunnerConfig -Runner $runner
    $runner.JitConfig = $null
    Write-CycleLog 'handed the runner configuration to the guest'
    $tookJob = Wait-ForRunner -Runner $runner -Started $started
} finally {
    $runner.JitConfig = $null
    # Each step of the cleanup runs even when the other fails.
    try {
        Stop-RunnerMachine -Machine $Machine
        Write-CycleLog 'turned the machine off'
    } catch {
        Write-CycleLog "could not turn the machine off: $($_.Exception.Message)"
    }
    # GitHub removes a runner after its job; this removes one that took none.
    try {
        if (Remove-Runner -Repository $repository -Id $runner.Id) {
            Write-CycleLog "removed the runner $($runner.Name)"
        }
    } catch {
        Write-CycleLog "could not remove the runner $($runner.Name): $($_.Exception.Message)"
    }
}
[pscustomobject]@{
    Machine = $Machine
    TookJob = $tookJob
}
