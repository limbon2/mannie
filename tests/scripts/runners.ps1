# Behavior tests invoked by runners.test.mjs. Hyper-V and GitHub calls are
# replaced inside the module, so these tests require neither administrator
# access nor a token, network, runner registration or virtual machine.
param(
    [Parameter(Mandatory)]
    [ValidateSet('syntax', 'labels', 'scheduling', 'configuration', 'queue', 'isolation', 'identity', 'lifecycle', 'lock', 'lock-child')]
    [string] $TestCase,
    [string] $Repository
)
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$hostFolder = Join-Path $root 'scripts/ci/runners/host'
$module = Import-Module (Join-Path $hostFolder 'runner-host.psm1') -PassThru -Force

function Assert-True {
    param([bool]$Condition, [string]$Reason)
    if (-not $Condition) { throw $Reason }
}
function Assert-Fails {
    param([scriptblock]$Action, [string]$Message)
    $caught = $false
    try { & $Action | Out-Null } catch {
        $caught = $true
        Assert-True -Condition $_.Exception.Message.Contains($Message) -Reason "Unexpected failure: $($_.Exception.Message)"
    }
    Assert-True -Condition $caught -Reason "Expected a failure containing: $Message"
}
function New-Machine {
    param([string]$Name, [string]$System = 'mannie-ubuntu-24.04')
    return [pscustomobject]@{
        Name = $Name
        Labels = @('self-hosted', 'disposable', $System)
        InCycle = $false
        Inherited = $false
        Paused = $false
        Running = $false
    }
}
function New-QueuedJob {
    param([int]$Id, [string]$System = 'mannie-ubuntu-24.04')
    return [pscustomobject]@{ id = $Id; labels = @('self-hosted', 'disposable', $System) }
}

switch ($TestCase) {
    'syntax' {
        foreach ($file in Get-ChildItem (Join-Path $root 'scripts/ci/runners') -Recurse -File | Where-Object { $_.Extension -in '.ps1', '.psm1', '.psd1' }) {
            $tokens = $null
            $errors = $null
            [System.Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$tokens, [ref]$errors) | Out-Null
            Assert-True -Condition ($errors.Count -eq 0) -Reason "$($file.Name): $($errors -join '; ')"
        }
    }
    'labels' {
        $labels = @('self-hosted', 'disposable', 'mannie-ubuntu-24.04')
        Assert-True (Test-LabelsCover $labels @('SELF-HOSTED', 'disposable')) 'Labels must be case-insensitive.'
        Assert-True (-not (Test-LabelsCover $labels @('ubuntu-24.04'))) 'A private runner must not take a hosted job.'
        Assert-True (-not (Test-LabelsCover $labels @('self-hosted', 'mannie-windows-11'))) 'A Linux guest must not take a Windows job.'
    }
    'scheduling' {
        $first = New-Machine 'first'
        $second = New-Machine 'second'
        $jobs = @((New-QueuedJob 1), (New-QueuedJob 2))
        $plan = Select-NeededCycles -Machines @($first, $second) -Queued $jobs -Runners @() -MaxRunning 1
        Assert-True ($plan.Starts.Count -eq 1 -and $plan.Starts[0].Job.id -eq 1) 'Start only the oldest job within the capacity.'
        $first.InCycle = $true
        $plan = Select-NeededCycles -Machines @($first, $second) -Queued $jobs -Runners @() -MaxRunning 2
        Assert-True ($plan.Starts.Count -eq 1 -and $plan.Starts[0].Job.id -eq 2) 'Reserve one queued job for a cycle that is coming online.'
        $first.InCycle = $false
        $first.Inherited = $true
        $plan = Select-NeededCycles -Machines @($first, $second) -Queued $jobs -Runners @() -MaxRunning 1
        Assert-True ($plan.Starts.Count -eq 0) 'An inherited job must occupy a slot.'
        $first.Inherited = $false
        $first.Running = $true
        $second.Paused = $true
        $plan = Select-NeededCycles -Machines @($first, $second) -Queued $jobs -Runners @() -MaxRunning 1
        Assert-True ($plan.Starts.Count -eq 0 -and $plan.Blocked.Count -eq 1) 'Leave a manually running machine and a paused machine alone.'
        $plan = Select-NeededCycles -Machines @($first) -Queued @((New-QueuedJob 3 'mannie-windows-11')) -Runners @() -MaxRunning 1
        Assert-True ($plan.Unserved.Count -eq 1) 'Report jobs whose labels have no machine.'
        $first.Running = $false
        $plan = Select-NeededCycles -Machines @($first) -Queued @() -Runners @() -MaxRunning 1
        Assert-True ($plan.Starts.Count -eq 0) 'Never start a machine when no job waits.'
    }
    'configuration' {
        $folder = Join-Path ([IO.Path]::GetTempPath()) ('mannie-runner-test-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $folder | Out-Null
        $path = Join-Path $folder 'runners.psd1'
        try {
            $valid = "@{ Repository = 'limbon2/mannie'; Checkpoint = 'clean'; MaxRunning = 1; Machines = @(@{ Name = 'linux'; Guest = 'Ubuntu'; Labels = @('self-hosted', 'disposable', 'mannie-ubuntu-24.04') }) }"
            Set-Content -LiteralPath $path -Value $valid
            Assert-True ((Read-RunnerConfig $path).MaxRunning -eq 1) 'Read a valid configuration.'
            Set-Content -LiteralPath $path -Value ($valid.Replace("Checkpoint = 'clean';", ''))
            Assert-Fails { Read-RunnerConfig $path } 'has no Checkpoint'
            Set-Content -LiteralPath $path -Value ($valid.Replace('MaxRunning = 1', 'MaxRunning = 0'))
            Assert-Fails { Read-RunnerConfig $path } 'at least 1'
            Set-Content -LiteralPath $path -Value ($valid.Replace("'disposable', ", ''))
            Assert-Fails { Read-RunnerConfig $path } 'lack disposable'
            Set-Content -LiteralPath $path -Value ($valid.Replace("Guest = 'Ubuntu'", "Guest = 'Other'"))
            Assert-Fails { Read-RunnerConfig $path } 'must be Windows or Ubuntu'
            Set-Content -LiteralPath $path -Value ($valid.Replace("'limbon2/mannie'", "'../wrong'"))
            Assert-Fails { Read-RunnerConfig $path } 'not owner/name'
        } finally {
            if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
            Remove-Item -LiteralPath $folder -Force
        }
    }
    'queue' {
        $jobs = & $module {
            function script:Get-GitHubList {
                param([string]$Path, [string]$Property)
                switch -Wildcard ($Path) {
                    '*runs?status=queued' { return [pscustomobject]@{ id = 10 } }
                    '*runs?status=in_progress' { return @([pscustomobject]@{ id = 10 }, [pscustomobject]@{ id = 11 }) }
                    '*runs/10/jobs*' {
                        return @(
                            [pscustomobject]@{ id = 1; status = 'queued'; labels = @('self-hosted'); created_at = '2026-10-20T01:00:00Z' },
                            [pscustomobject]@{ id = 2; status = 'queued'; labels = @('ubuntu-24.04'); created_at = '2026-10-20T00:00:00Z' },
                            [pscustomobject]@{ id = 3; status = 'in_progress'; labels = @('self-hosted'); created_at = '2026-10-20T00:00:00Z' }
                        )
                    }
                    '*runs/11/jobs*' { return [pscustomobject]@{ id = 4; status = 'queued'; labels = @('self-hosted'); created_at = '2026-10-20T00:30:00Z' } }
                    default { throw "Unexpected API path: $Path" }
                }
            }
            @(Get-QueuedRunnerJobs -Repository 'limbon2/mannie')
        }
        Assert-True ($jobs.Count -eq 2 -and $jobs[0].id -eq 4 -and $jobs[1].id -eq 1) 'Deduplicate runs, ignore hosted and running jobs, and return the oldest job first.'
    }
    'isolation' {
        $outcomes = & $module {
            $script:serviceEnabled = $true
            $script:rules = @(
                foreach ($direction in 'Inbound', 'Outbound') {
                    [pscustomobject]@{ Direction = $direction; Action = 'Allow'; RemoteAddress = '10.77.1.1' }
                    foreach ($network in Get-BlockedNetworks) {
                        [pscustomobject]@{ Direction = $direction; Action = 'Deny'; RemoteAddress = $network }
                    }
                }
            )
            $script:extended = @(
                foreach ($direction in 'Inbound', 'Outbound') {
                    [pscustomobject]@{ Direction = $direction; Action = 'Deny'; RemoteIPAddress = '10.77.1.1'; Protocol = 'ANY'; LocalPort = 'ANY'; RemotePort = 'ANY'; Weight = 10; Stateful = $false }
                }
                [pscustomobject]@{ Direction = 'Inbound'; Action = 'Allow'; RemoteIPAddress = '10.77.1.1'; Protocol = 'TCP'; LocalPort = '22'; RemotePort = 'ANY'; Weight = 20; Stateful = $true }
            )
            function script:Get-VM { return [pscustomobject]@{ AutomaticCheckpointsEnabled = $false } }
            function script:Get-VMIntegrationService { return [pscustomobject]@{ Id = 'vm\6C09BB55-D683-4DA0-8931-C9BF705F6480'; Enabled = $script:serviceEnabled } }
            function script:Get-VMNetworkAdapter { return [pscustomobject]@{ Name = 'Ethernet'; SwitchName = 'Mannie CI'; DhcpGuard = 'On'; RouterGuard = 'On'; MacAddressSpoofing = 'Off' } }
            function script:Get-VMSwitch { return [pscustomobject]@{ Name = 'Mannie CI'; SwitchType = 'Internal' } }
            function script:Get-NetIPAddress { return [pscustomobject]@{ IPAddress = '10.77.1.1' } }
            function script:Get-VMNetworkAdapterAcl { return $script:rules }
            function script:Get-VMNetworkAdapterExtendedAcl { return $script:extended }
            $good = @(Get-RunnerMachineProblems 'test')
            $script:extended = @($script:extended | Where-Object { $_.Action -ne 'Deny' })
            $missingGuards = @(Get-RunnerMachineProblems 'test')
            $script:serviceEnabled = $false
            $missingService = @(Get-RunnerMachineProblems 'test')
            $script:rules += [pscustomobject]@{ Direction = 'Outbound'; Action = 'Allow'; RemoteAddress = '192.168.0.0/16' }
            $allowedNetwork = @(Get-RunnerMachineProblems 'test')
            [pscustomobject]@{ Good = $good; MissingGuards = $missingGuards; MissingService = $missingService; AllowedNetwork = $allowedNetwork }
        }
        Assert-True ($outcomes.Good.Count -eq 0) 'Accept the fully isolated machine.'
        Assert-True (($outcomes.MissingGuards -join ' ').Contains('does not refuse')) 'Refuse an image that exposes host services.'
        Assert-True (($outcomes.MissingService -join ' ').Contains('Guest services are off')) 'Refuse an image that cannot receive the host configuration.'
        Assert-True (($outcomes.AllowedNetwork -join ' ').Contains('allows traffic with all of 192.168.0.0/16')) 'Refuse a broad exception to private-network isolation.'
    }
    'lifecycle' {
        # Load the cycle's real wait function without running its VM setup or
        # registering a runner. The simulated clock makes timeouts immediate.
        $tokens = $null
        $errors = $null
        $tree = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $hostFolder 'run-runner-cycle.ps1'), [ref]$tokens, [ref]$errors)
        $wait = $tree.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Wait-ForRunner' }, $true)
        . ([scriptblock]::Create($wait.Extent.Text))
        $Machine = 'test'
        $repository = 'limbon2/mannie'
        $pollSeconds = 20
        $connectLimit = New-TimeSpan -Minutes 1
        $idleLimit = New-TimeSpan -Minutes 1
        $offlineLimit = New-TimeSpan -Minutes 1
        $shutdownLimit = New-TimeSpan -Minutes 1
        $cycleLimit = New-TimeSpan -Minutes 5
        function Start-Sleep { }
        function Get-Date {
            $script:clock = $script:clock.AddSeconds(10)
            return $script:clock
        }
        function Get-VM {
            $state = 'Running'
            if ($script:scenario -in @('fast-completed', 'boot-failed', 'stopped-api-failed')) { $state = 'Off' }
            return [pscustomobject]@{ State = $state }
        }
        function Get-Runner {
            switch ($script:scenario) {
                'fast-completed' { return $null }
                'boot-failed' { return [pscustomobject]@{ status = 'offline'; busy = $false } }
                'stopped-api-failed' { throw 'API unavailable' }
                'running-api-failed' { throw 'API unavailable' }
                'never-online' { return [pscustomobject]@{ status = 'offline'; busy = $false } }
                'idle' { return [pscustomobject]@{ status = 'online'; busy = $false } }
                'offline-after-job' {
                    $script:reads += 1
                    if ($script:reads -eq 1) { return [pscustomobject]@{ status = 'online'; busy = $true } }
                    return [pscustomobject]@{ status = 'offline'; busy = $true }
                }
                'shutdown-timeout' {
                    $script:reads += 1
                    if ($script:reads -eq 1) { return [pscustomobject]@{ status = 'online'; busy = $true } }
                    return $null
                }
            }
            throw 'Unexpected lifecycle scenario.'
        }
        function Write-CycleLog { param([string]$Text) $script:messages.Add($Text) }
        foreach ($case in @(
            @{ Name = 'fast-completed'; TookJob = $true; Message = 'GitHub removed the runner after its job' },
            @{ Name = 'boot-failed'; TookJob = $false; Message = 'the guest powered off' },
            @{ Name = 'stopped-api-failed'; TookJob = $false; Message = 'the guest powered off' },
            @{ Name = 'running-api-failed'; TookJob = $false; Message = 'the cycle ran for longer than' },
            @{ Name = 'never-online'; TookJob = $false; Message = 'the runner did not come online' },
            @{ Name = 'idle'; TookJob = $false; Message = 'the runner took no job within' },
            @{ Name = 'offline-after-job'; TookJob = $true; Message = 'the runner was offline for' },
            @{ Name = 'shutdown-timeout'; TookJob = $true; Message = 'the guest did not power off within' }
        )) {
            $script:scenario = $case.Name
            $script:clock = [datetime]'2026-10-20T00:00:00'
            $script:reads = 0
            $script:messages = [System.Collections.Generic.List[string]]::new()
            $result = Wait-ForRunner -Runner ([pscustomobject]@{ Id = 1 }) -Started $script:clock
            Assert-True ($result -eq $case.TookJob) "$($case.Name): wrong job outcome."
            Assert-True (($script:messages -join '; ').Contains($case.Message)) "$($case.Name): missing lifecycle reason."
        }
    }
    'identity' {
        $name = New-RunnerName 'mannie-ci-ubuntu-22.04'
        Assert-True (Test-RunnerOfMachine $name 'mannie-ci-ubuntu-22.04') 'Recognize an owned runner.'
        Assert-True (-not (Test-RunnerOfMachine 'mannie-ci-ubuntu-22x04-20261020-120000' 'mannie-ci-ubuntu-22.04')) 'Escape punctuation in machine names.'
        Assert-True (-not (Test-RunnerOfMachine ($name + '-other') 'mannie-ci-ubuntu-22.04')) 'Do not remove a registration with an unrelated suffix.'
    }
    'lock' {
        $Repository = "limbon2/mannie-lock-test-$PID"
        $held = Enter-ControllerLock -Repository $Repository
        try {
            $start = [Diagnostics.ProcessStartInfo]::new()
            $start.FileName = (Get-Process -Id $PID).Path
            $start.Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$PSCommandPath`" -TestCase lock-child -Repository $Repository"
            $start.UseShellExecute = $false
            $start.CreateNoWindow = $true
            $child = [Diagnostics.Process]::Start($start)
            try {
                if (-not $child.WaitForExit(10000)) {
                    $child.Kill()
                    throw 'The second controller did not fail promptly.'
                }
                Assert-True ($child.ExitCode -eq 0) 'A second process must not acquire the controller lock.'
            } finally { $child.Dispose() }
        } finally {
            $held.ReleaseMutex()
            $held.Dispose()
        }
        $next = Enter-ControllerLock -Repository $Repository
        $next.ReleaseMutex()
        $next.Dispose()
    }
    'lock-child' {
        try {
            $unexpected = Enter-ControllerLock -Repository $Repository
            $unexpected.ReleaseMutex()
            $unexpected.Dispose()
            exit 9
        } catch {
            if (-not $_.Exception.Message.Contains('another controller is already serving')) { throw }
        }
    }
}
Write-Output "$TestCase passed"
