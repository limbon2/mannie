# Behavior tests invoked by runners.test.mjs. Hyper-V and GitHub calls are
# replaced inside the module, so these tests require neither administrator
# access nor a token, network, runner registration or virtual machine.
param(
    [Parameter(Mandatory)]
    [ValidateSet('syntax', 'windows-path', 'windows-checks', 'labels', 'scheduling', 'configuration', 'queue', 'isolation', 'api', 'recovery', 'identity', 'lifecycle', 'lock', 'lock-child')]
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
        InheritedRunnerIds = @()
        InheritedStarted = $null
        InheritedLastOnline = $null
        InheritedEndedAt = $null
        Paused = $false
        Running = $false
        Job = $null
        ReportedRunning = $false
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
    'windows-path' {
        $startup = Join-Path $root 'scripts/ci/runners/windows/start-runner.ps1'
        $tokens = $null
        $errors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile($startup, [ref]$tokens, [ref]$errors)
        $definition = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Set-GitBashPath' }, $true)
        . ([scriptblock]::Create($definition.Extent.Text))
        $folder = Join-Path ([IO.Path]::GetTempPath()) ('mannie-git-path-' + [guid]::NewGuid().ToString('N'))
        $gitRoot = Join-Path $folder 'Git with spaces'
        $gitBin = Join-Path $gitRoot 'bin'
        $gitCmd = Join-Path $gitRoot 'cmd'
        New-Item -ItemType Directory -Path $gitBin, $gitCmd -Force | Out-Null
        $gitExe = Join-Path $gitCmd 'git.exe'
        $bashExe = Join-Path $gitBin 'bash.exe'
        Set-Content -LiteralPath $gitExe, $bashExe -Value ''
        $originalPath = $env:Path
        $originalLocalAppData = $env:LOCALAPPDATA
        $script:gitAvailable = $true
        function Get-Command {
            param([string] $Name, [string] $ErrorAction)
            if ($script:gitAvailable) { return [pscustomobject]@{ Source = $gitExe } }
        }
        try {
            Set-GitBashPath
            $powerShell = (Get-Process -Id $PID).Path
            $inherited = & $powerShell -NoProfile -NonInteractive -Command '[Console]::Write((Get-Command bash.exe -ErrorAction Stop).Source)'
            Assert-True ($LASTEXITCODE -eq 0 -and $inherited -eq $bashExe) 'Jobs must find Git Bash through the inherited PATH, including an installation folder with spaces.'
            Assert-True ($env:Path.EndsWith($originalPath)) 'Keep the rest of the inherited command search path.'
            Remove-Item -LiteralPath $bashExe
            Assert-Fails { Set-GitBashPath } 'Git Bash is missing'
            $script:gitAvailable = $false
            Assert-Fails { Set-GitBashPath } 'install Git for Windows first'

            $setup = Join-Path $root 'scripts/ci/runners/windows/setup-runner-image.ps1'
            $setupAst = [System.Management.Automation.Language.Parser]::ParseFile($setup, [ref]$tokens, [ref]$errors)
            $check = $setupAst.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Test-PowerShellExecutable' }, $true)
            . ([scriptblock]::Create($check.Extent.Text))
            $env:LOCALAPPDATA = $folder
            $script:powerShellSource = $null
            function Get-Command {
                param([string] $Name, [string] $ErrorAction)
                if ($script:powerShellSource) { return [pscustomobject]@{ Source = $script:powerShellSource } }
            }
            Assert-True (-not (Test-PowerShellExecutable)) 'A missing PowerShell installation must fail the image preflight.'
            $script:powerShellSource = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\pwsh.exe'
            Assert-True (-not (Test-PowerShellExecutable)) 'A Store app execution alias must not pass as an executable that Node can open.'
            $script:powerShellSource = Join-Path $folder 'PowerShell\7\pwsh.exe'
            Assert-True (Test-PowerShellExecutable) 'The MSI installation must pass the image preflight.'
        } finally {
            $env:Path = $originalPath
            $env:LOCALAPPDATA = $originalLocalAppData
            foreach ($file in $gitExe, $bashExe) {
                if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force }
            }
            Remove-Item -LiteralPath $gitBin, $gitCmd, $gitRoot, $folder -Force
        }
    }
    'windows-checks' {
        $scriptPath = Join-Path $root 'scripts/ci/runners/windows/run-check.ps1'
        $tokens = $null
        $errors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$errors)
        foreach ($name in 'Get-CheckCommand', 'Get-CheckEnvironment', 'Stop-CheckProcess', 'Remove-CheckFile') {
            $function = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
            . ([scriptblock]::Create($function.Extent.Text))
        }
        function Get-Command {
            param([string]$Name, [string]$ErrorAction)
            return [pscustomobject]@{ Source = $Name }
        }
        Assert-True ((Get-CheckCommand -Name 'ci').Arguments[0] -eq 'ci') 'Run the full CI task.'
        Assert-True ((Get-CheckCommand -Name 'e2e').Arguments[0] -eq 'e2e') 'Run the UI build and tests.'
        $performance = Get-CheckCommand -Name 'performance'
        Assert-True ($performance.Executable -eq 'pnpm.cmd' -and ($performance.Arguments -join ' ') -eq 'exec wdio run tests/e2e/wdio.perf.conf.ts') 'Preserve the performance command arguments.'
        Assert-Fails { Get-CheckCommand -Name 'other' } 'unknown check'
        $savedToken = $env:GITHUB_TOKEN
        $savedColor = $env:CARGO_TERM_COLOR
        $folder = Join-Path ([IO.Path]::GetTempPath()) ('mannie-check-test-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $folder | Out-Null
        try {
            $env:GITHUB_TOKEN = 'synthetic-token-must-not-be-forwarded'
            $env:CARGO_TERM_COLOR = 'never'
            $environment = Get-CheckEnvironment
            Assert-True (-not $environment.ContainsKey('GITHUB_TOKEN')) 'Never put the job token in the configuration file.'
            Assert-True ($environment.CARGO_TERM_COLOR -eq 'never') 'Keep build settings in the normal task.'
            Assert-True (-not (($environment | ConvertTo-Json).Contains($env:GITHUB_TOKEN))) 'The serialized handover must not contain the token.'
            $configuration = Join-Path $folder 'configuration.json'
            $log = Join-Path $folder 'check.log'
            $result = Join-Path $folder 'result.json'
            $started = Join-Path $folder 'started.json'
            $startMarker = Join-Path $folder 'run'
            Set-Content -LiteralPath $startMarker -Value 'Unit test startup acknowledgment.'
            $data = @{
                Executable = (Get-Process -Id $PID).Path
                Arguments = @('-NoProfile', '-NonInteractive', '-Command', 'Write-Output $env:CARGO_TERM_COLOR; exit 7')
                Directory = $folder
                Environment = @{ CARGO_TERM_COLOR = 'never' }
                Log = $log
                Result = $result
                Started = $started
                StartMarker = $startMarker
            }
            $child = Join-Path $root 'scripts/ci/runners/windows/run-check-task.ps1'
            $data | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $configuration -Encoding UTF8
            & $data.Executable -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $child -Configuration $configuration
            Assert-True ($LASTEXITCODE -eq 7) 'The child must return the failing command exit code.'
            Assert-True ((Get-Content -LiteralPath $log -Raw).Contains('never')) 'Keep the command output for the parent.'
            Assert-True ((Get-Content -LiteralPath $result -Raw | ConvertFrom-Json).ExitCode -eq 7) 'Publish the same failing exit code in the completion file.'
            Assert-True (-not (Test-Path -LiteralPath ($result + '.pending'))) 'Publish the completion by renaming its finished file.'
            Assert-True ((Get-Content -LiteralPath $started -Raw | ConvertFrom-Json).Id -gt 0) 'Record the child process before it starts a command.'
            Remove-Item -LiteralPath $log, $result, $started
            $data.Executable = Join-Path $folder 'missing.exe'
            $data | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $configuration -Encoding UTF8
            & (Get-Process -Id $PID).Path -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $child -Configuration $configuration
            Assert-True ($LASTEXITCODE -eq 1 -and (Get-Content -LiteralPath $result -Raw | ConvertFrom-Json).ExitCode -eq 1) 'A launch failure must also complete with failure.'
            Assert-Fails { Stop-CheckProcess -Id $PID -StartedUtcTicks 0 -PowerShell (Get-Process -Id $PID).Path -Script $child } 'identity changed'
            $locked = Join-Path $folder 'locked'
            Set-Content -LiteralPath $locked -Value 'synthetic lock'
            $script:removeAttempts = 0
            function Remove-Item {
                param([string]$LiteralPath, [switch]$Force, [string]$ErrorAction)
                $script:removeAttempts++
                if ($script:removeAttempts -le 2) { throw [IO.IOException]::new('synthetic file lock') }
                Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Force -ErrorAction Stop
            }
            Remove-CheckFile -Path $locked
            Assert-True ($script:removeAttempts -eq 3 -and -not (Test-Path -LiteralPath $locked)) 'Retry transient file locks within a bounded cleanup.'
        } finally {
            $env:GITHUB_TOKEN = $savedToken
            $env:CARGO_TERM_COLOR = $savedColor
            foreach ($name in 'configuration.json', 'check.log', 'result.json', 'result.json.pending', 'started.json', 'started.json.pending', 'run', 'locked') {
                $path = Join-Path $folder $name
                if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
            }
            Remove-Item -LiteralPath $folder -Force
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
            $script:serviceEnabled = $true
            $script:extended = @(
                foreach ($direction in 'Inbound', 'Outbound') {
                    [pscustomobject]@{ Direction = $direction; Action = 'Deny'; RemoteIPAddress = '10.77.1.1/32'; Protocol = 'ANY'; LocalPort = 'ANY'; RemotePort = 'ANY'; Weight = 10; Stateful = $false }
                }
                [pscustomobject]@{ Direction = 'Inbound'; Action = 'Allow'; RemoteIPAddress = '10.77.1.1'; Protocol = 'TCP'; LocalPort = '22'; RemotePort = 'ANY'; Weight = 20; Stateful = $true }
            )
            $baseRules = @($script:rules)
            $bypasses = @(
                foreach ($address in @('192.168.0.0/24', '192.168.0.0/16', '192.0.0.0/8', '192.168.0.1/32', '10.77.1.0/24', '10.77.1.2', '::/0', '::/1', 'fd00::/64', 'fd00::1/128', '::ffff:192.168.0.1', 'malformed/999')) {
                    $script:rules = @($baseRules) + @(
                        foreach ($direction in 'Inbound', 'Outbound') {
                            [pscustomobject]@{ Direction = $direction; Action = 'Allow'; RemoteAddress = $address }
                        }
                    )
                    [pscustomobject]@{ Address = $address; Problems = @(Get-RunnerMachineProblems 'test') }
                }
            )
            $script:rules = @($baseRules | ForEach-Object {
                $address = $_.RemoteAddress
                if ($address -eq '::/0') { $address = '0:0:0:0:0:0:0:0/0' }
                if ($address -eq '10.77.1.1') { $address = '10.77.1.1/32' }
                [pscustomobject]@{ Direction = $_.Direction; Action = $_.Action; RemoteAddress = $address }
            }) + @([pscustomobject]@{ Direction = 'Outbound'; Action = 'Allow'; RemoteAddress = '0.0.0.0/0' })
            $normalized = @(Get-RunnerMachineProblems 'test')
            [pscustomobject]@{ Good = $good; MissingGuards = $missingGuards; MissingService = $missingService; Bypasses = $bypasses; Normalized = $normalized }
        }
        Assert-True ($outcomes.Good.Count -eq 0) 'Accept the fully isolated machine.'
        Assert-True (($outcomes.MissingGuards -join ' ').Contains('does not refuse')) 'Refuse an image that exposes host services.'
        Assert-True (($outcomes.MissingService -join ' ').Contains('Guest services are off')) 'Refuse an image that cannot receive the host configuration.'
        foreach ($bypass in $outcomes.Bypasses) {
            Assert-True (($bypass.Problems -join ' ').Contains('unexpected allow')) "Refuse the unreviewed allow $($bypass.Address), even when a broader deny exists."
        }
        Assert-True ($outcomes.Normalized.Count -eq 0) 'Accept equivalent host and IPv6 prefixes and the default IPv4 internet allow.'
    }
    'api' {
        & $module {
            $script:token = ConvertTo-SecureString 'fake-runner-test-token' -AsPlainText -Force
            $script:requestTimeouts = [System.Collections.Generic.List[object]]::new()
            $script:failure = ''
            function script:Invoke-RestMethod {
                param($Method, $Uri, $Headers, $UserAgent, $TimeoutSec, $OperationTimeoutSeconds, $Body, $ContentType)
                $script:requestTimeouts.Add([pscustomobject]@{ Connection = $TimeoutSec; Operation = $OperationTimeoutSeconds })
                if ($script:failure -eq 'timeout') { throw [Net.WebException]::new('request timed out') }
                if ($script:failure -eq 'missing') {
                    $error = [Exception]::new('not found')
                    $error | Add-Member -NotePropertyName Response -NotePropertyValue ([pscustomobject]@{ StatusCode = 404 })
                    throw $error
                }
                return [pscustomobject]@{ ok = $true }
            }
            foreach ($method in 'GET', 'POST', 'DELETE') { Invoke-GitHubApi -Method $method -Path 'test' | Out-Null }
            foreach ($request in $script:requestTimeouts) {
                if ($request.Connection -ne 30) { throw 'A GitHub request has no finite connection timeout.' }
                if ($PSVersionTable.PSVersion -ge [version]'7.4' -and $request.Operation -ne 30) { throw 'A PowerShell 7 request has no finite read timeout.' }
            }
            $script:failure = 'timeout'
            $caught = $false
            try { Invoke-GitHubApi -Method GET -Path 'test' | Out-Null }
            catch {
                $caught = $_.Exception.Message.Contains('GitHub: GET /test failed: request timed out')
                if ($_.Exception.Message.Contains('fake-runner-test-token')) { throw 'The API error exposed a token.' }
            }
            if (-not $caught) { throw 'An API timeout did not reach the controller as a recoverable failure.' }
            $script:failure = 'missing'
            if ($null -ne (Invoke-GitHubApi -Method GET -Path 'test' -AllowNotFound)) { throw 'A removed runner must return no state.' }
        }
    }
    'recovery' {
        $tokens = $null
        $errors = $null
        $tree = [System.Management.Automation.Language.Parser]::ParseFile((Join-Path $hostFolder 'run-runners.ps1'), [ref]$tokens, [ref]$errors)
        foreach ($name in @('Get-MachineRunners', 'Remove-MachineRunners', 'Reset-Machine', 'Initialize-RunnerMachines', 'Stop-InheritedMachine', 'Update-InheritedMachine', 'Update-Machine')) {
            $definition = $tree.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
            . ([scriptblock]::Create($definition.Extent.Text))
        }
        $repository = 'limbon2/mannie'
        function Get-Date { return $script:clock }
        function Get-VM { return [pscustomobject]@{ State = $script:vmState; Uptime = $script:uptime } }
        function Stop-RunnerMachine { $script:stops += 1; $script:vmState = 'Off' }
        function Get-Runner {
            $script:reads += 1
            if ($script:unavailable) { throw 'API unavailable' }
            if ($script:response -eq 'removed') { return $null }
            return [pscustomobject]@{ status = $script:response; busy = $script:busy }
        }
        function Test-RepositoryAccess { if ($script:unavailable) { throw 'API unavailable' } }
        function Get-RepositoryRunners {
            if ($script:unavailable) { throw 'API unavailable' }
            return [pscustomobject]@{ id = 42; name = 'recovery-test-20261020-000000'; busy = $true; status = 'online' }
        }
        function Remove-Runner {
            if ($script:unavailable) { throw 'API unavailable' }
            $script:removed += 1
            return $true
        }
        function Write-RunnerLog { param($Text) $script:messages.Add($Text) }
        function Write-RunnerProblem { param($Key, $Text) $script:messages.Add($Text) }
        function Clear-RunnerProblem { }
        function Start-InheritedFixture {
            $script:clock = [datetime]'2026-10-20T00:00:00'
            $script:vmState = 'Running'
            $script:uptime = New-TimeSpan -Hours 1
            $script:stops = 0
            $script:reads = 0
            $script:removed = 0
            $script:unavailable = $false
            $script:response = 'online'
            $script:busy = $true
            $script:messages = [System.Collections.Generic.List[string]]::new()
            $machine = New-Machine 'recovery-test'
            $registration = [pscustomobject]@{ id = 42; name = 'recovery-test-20261020-000000'; busy = $true; status = 'online' }
            Reset-Machine -Machine $machine -Runners @($registration)
            Assert-True $machine.Inherited 'Adopt the busy registered machine.'
            Assert-True ($machine.InheritedStarted -eq $script:clock.AddHours(-1)) 'Keep the original VM boot time across controller restarts.'
            return $machine
        }
        $machine = Start-InheritedFixture
        $script:clock = $script:clock.AddMinutes(8)
        Update-Machine $machine
        Assert-True ($machine.Inherited -and $script:stops -eq 0 -and $machine.InheritedLastOnline -eq $script:clock) 'Leave a healthy inherited job running and refresh its observed state.'

        foreach ($runnerState in 'removed', 'online') {
            $machine = Start-InheritedFixture
            $script:response = $runnerState
            $script:busy = $false
            Update-Machine $machine
            Assert-True ($null -ne $machine.InheritedEndedAt) 'Observe a completed inherited job.'
            $script:clock = $script:clock.AddMinutes(6)
            $script:unavailable = $true
            Update-Machine $machine
            Assert-True (-not $machine.Inherited -and $script:stops -eq 1 -and $script:reads -eq 1) 'Stop after the shutdown deadline even when the next API read and cleanup would fail.'
        }
        foreach ($runnerState in 'offline', 'unavailable') {
            $machine = Start-InheritedFixture
            $script:response = $runnerState
            $script:unavailable = $runnerState -eq 'unavailable'
            $script:clock = $script:clock.AddMinutes(9)
            Update-Machine $machine
            Assert-True $machine.Inherited 'Allow a short loss of runner visibility.'
            $script:clock = $script:clock.AddMinutes(2)
            Update-Machine $machine
            Assert-True (-not $machine.Inherited -and $script:stops -eq 1 -and $script:reads -eq 1) "An offline runner or API outage must not occupy capacity forever ($runnerState; inherited=$($machine.Inherited); stops=$script:stops; reads=$script:reads; logs=$($script:messages -join ' / '))."
        }
        $machine = Start-InheritedFixture
        $machine.InheritedStarted = $script:clock.AddHours(-8)
        $script:unavailable = $true
        Update-Machine $machine
        Assert-True (-not $machine.Inherited -and $script:stops -eq 1 -and $script:reads -eq 0) 'Enforce the original overall deadline before attempting GitHub.'

        foreach ($recovers in @($true, $false)) {
            $machine = Start-InheritedFixture
            $machine.Inherited = $false
            $machine.InheritedRunnerIds = @()
            $machines = @($machine)
            $script:unavailable = $true
            Initialize-RunnerMachines
            Assert-True ($machine.Inherited -and $machine.InheritedRunnerIds.Count -eq 0) 'An initial API outage must not abort monitoring of configured running VMs.'
            if ($recovers) {
                $script:clock = $script:clock.AddMinutes(1)
                $script:unavailable = $false
                Update-Machine $machine
                Assert-True ($machine.Inherited -and $machine.InheritedRunnerIds[0] -eq 42 -and $script:stops -eq 0) 'Discover the owned runner and leave its job running when GitHub recovers.'
            } else {
                $script:clock = $script:clock.AddMinutes(11)
                Update-Machine $machine
                Assert-True (-not $machine.Inherited -and $script:stops -eq 1) 'An initial API outage must still bound VM lifetime.'
            }
        }
        $machine = Start-InheritedFixture
        $script:vmState = 'Off'
        Update-Machine $machine
        Assert-True (-not $machine.Inherited -and $script:removed -eq 1) 'Free a stopped inherited machine and remove its registration.'
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
