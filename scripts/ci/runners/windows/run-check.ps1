# Runs regular Windows checks with the automatic sign-in's normal token.
# The runner itself remains elevated for the worker service smoke test.
# Elevated sessions on the owner's Windows 11 image cannot persist generic
# credentials, and WebView2 ignores EdgeDriver's environment overrides there.
param(
    [Parameter(Mandatory)]
    [ValidateSet('ci', 'e2e', 'performance')]
    [string] $Check,

    [ValidateRange(1, 14400)]
    [int] $TimeoutSeconds = 7200
)
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

function Get-CheckCommand {
    param([string] $Name)

    switch ($Name) {
        'ci' { return @{ Executable = (Get-Command task.exe -ErrorAction Stop).Source; Arguments = @('ci') } }
        'e2e' { return @{ Executable = (Get-Command task.exe -ErrorAction Stop).Source; Arguments = @('e2e') } }
        'performance' { return @{ Executable = (Get-Command pnpm.cmd -ErrorAction Stop).Source; Arguments = @('exec', 'wdio', 'run', 'tests/e2e/wdio.perf.conf.ts') } }
        default { throw "run-check: unknown check $Name" }
    }
}

function Get-CheckEnvironment {
    # Task Scheduler starts a new environment. Keep the tools that the setup
    # actions put on PATH and the build settings, without writing job tokens
    # or provider credentials into the handover file.
    $settings = @{}
    foreach ($name in @(
        'Path', 'PNPM_HOME', 'CI', 'GITHUB_ACTIONS', 'GITHUB_BASE_REF', 'GITHUB_STEP_SUMMARY',
        'CARGO_HOME', 'RUSTUP_HOME', 'RUSTUP_TOOLCHAIN', 'CARGO_TARGET_DIR',
        'CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL', 'CARGO_TERM_COLOR', 'CARGO_PROFILE_DEV_DEBUG',
        'RUNNER_TEMP', 'RUNNER_TOOL_CACHE', 'MANNIE_PERF_REPORT_ONLY', 'MANNIE_E2E_SHOW_WINDOW'
    )) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -ne $value) { $settings[$name] = $value }
    }
    return $settings
}

function Stop-CheckProcess {
    param([int] $Id, [long] $StartedUtcTicks, [string] $PowerShell, [string] $Script)

    $process = Get-Process -Id $Id -ErrorAction SilentlyContinue
    if (-not $process) { return }
    if ($process.StartTime.ToUniversalTime().Ticks -ne $StartedUtcTicks -or $process.Path -ne $PowerShell) {
        throw 'run-check: refusing to stop a process whose identity changed'
    }
    $details = Get-CimInstance Win32_Process -Filter "ProcessId = $Id"
    if (-not $details -or -not ([string]$details.CommandLine).Contains($Script)) {
        throw 'run-check: refusing to stop a process outside the check task'
    }
    # Stop-ScheduledTask can return before its children release their files.
    # End this verified process tree before stopping the registered task.
    & (Join-Path $env:SystemRoot 'System32\taskkill.exe') /PID $Id /T /F *> $null
    if ($LASTEXITCODE -ne 0) {
        $remaining = Get-Process -Id $Id -ErrorAction SilentlyContinue
        if ($remaining -and $remaining.StartTime.ToUniversalTime().Ticks -eq $StartedUtcTicks) {
            throw 'run-check: could not stop the check process tree'
        }
    }
}

function Remove-CheckFile {
    param([string] $Path)

    for ($attempt = 1; $attempt -le 40; $attempt++) {
        try {
            if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force -ErrorAction Stop }
            return
        } catch [IO.IOException], [UnauthorizedAccessException] {
            if ($attempt -eq 40) { throw }
            Start-Sleep -Milliseconds 250
        }
    }
}

$command = Get-CheckCommand -Name $Check
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $arguments = $command.Arguments
    & $command.Executable @arguments
    exit $LASTEXITCODE
}

$workspace = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../../..')).ProviderPath
if ((Get-Location).ProviderPath -ne $workspace) {
    throw 'run-check: start the check from the repository root'
}
# The elevated checkout can belong to Administrators. The same account's
# normal token needs to write build outputs, and Git checks its ownership.
$account = $identity.User
$workspaceAcl = Get-Acl -LiteralPath $workspace
$workspaceAcl.SetOwner($account)
$workspaceAcl.SetAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($account, 'Modify', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
Set-Acl -LiteralPath $workspace -AclObject $workspaceAcl
$git = Join-Path $workspace '.git'
if (Test-Path -LiteralPath $git) {
    $gitAcl = Get-Acl -LiteralPath $git
    $gitAcl.SetOwner($account)
    Set-Acl -LiteralPath $git -AclObject $gitAcl
}

$folder = Join-Path ([IO.Path]::GetTempPath()) ('mannie-ci-check-' + [guid]::NewGuid().ToString('N'))
$taskName = 'Mannie check ' + [guid]::NewGuid().ToString('N')
$taskPath = '\Mannie\'
$registered = $false
$reader = $null
$taskProcess = $null
$exitCode = 1
try {
    New-Item -ItemType Directory -Path $folder | Out-Null
    $acl = [System.Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @('S-1-5-18', 'S-1-5-32-544', $identity.User.Value)) {
        $account = [System.Security.Principal.SecurityIdentifier]::new($sid)
        $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($account, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
    }
    Set-Acl -LiteralPath $folder -AclObject $acl
    $configuration = Join-Path $folder 'configuration.json'
    $child = Join-Path $folder 'run-check-task.ps1'
    $log = Join-Path $folder 'check.log'
    $result = Join-Path $folder 'result.json'
    $startedFile = Join-Path $folder 'started.json'
    $startMarker = Join-Path $folder 'run'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'run-check-task.ps1') -Destination $child
    @{
        Arguments = $command.Arguments
        Directory = (Get-Location).ProviderPath
        Environment = Get-CheckEnvironment
        Executable = $command.Executable
        Log = $log
        Result = $result
        Started = $startedFile
        StartMarker = $startMarker
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $configuration -Encoding UTF8
    $powershell = (Get-Command pwsh.exe -ErrorAction Stop).Source
    $actionArguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + $child + '" -Configuration "' + $configuration + '"'
    $action = New-ScheduledTaskAction -Execute $powershell -Argument $actionArguments
    $taskPrincipal = New-ScheduledTaskPrincipal -UserId $identity.User.Value -LogonType Interactive -RunLevel Limited
    # Let the parent end the verified process tree before Task Scheduler's
    # backup deadline can end only the PowerShell task process.
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::FromSeconds($TimeoutSeconds + 30)) -Priority 4 -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $taskName -TaskPath $taskPath -Action $action -Principal $taskPrincipal -Settings $settings -Force | Out-Null
    $registered = $true
    Start-ScheduledTask -TaskName $taskName -TaskPath $taskPath
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $startDeadline = (Get-Date).AddSeconds(30)
    while ($true) {
        if (-not $taskProcess -and (Test-Path -LiteralPath $startedFile)) {
            $taskProcess = Get-Content -LiteralPath $startedFile -Raw | ConvertFrom-Json
            Set-Content -LiteralPath $startMarker -Value 'The parent recorded the check process.'
        }
        if (-not $reader -and (Test-Path -LiteralPath $log)) {
            $stream = [IO.File]::Open($log, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
            $reader = [IO.StreamReader]::new($stream)
        }
        if ($reader) {
            while ($null -ne ($line = $reader.ReadLine())) { Write-Output $line }
        }
        if (Test-Path -LiteralPath $result) {
            $completion = Get-Content -LiteralPath $result -Raw | ConvertFrom-Json
            $exitCode = [int]$completion.ExitCode
            if ($reader) {
                while ($null -ne ($line = $reader.ReadLine())) { Write-Output $line }
            }
            break
        }
        $state = (Get-ScheduledTask -TaskName $taskName -TaskPath $taskPath).State
        if (-not $taskProcess -and (Get-Date) -ge $startDeadline) { throw 'run-check: the normal interactive task did not start within 30 seconds' }
        if ($taskProcess -and $state -ne 'Running') {
            if (Test-Path -LiteralPath $result) { continue }
            throw 'run-check: the normal interactive task stopped without a result'
        }
        if ((Get-Date) -ge $deadline) { throw "run-check: the check exceeded $TimeoutSeconds seconds" }
        Start-Sleep -Seconds 1
    }
} finally {
    try {
        try {
            if ($taskProcess) {
                Stop-CheckProcess -Id $taskProcess.Id -StartedUtcTicks $taskProcess.StartedUtcTicks -PowerShell $powershell -Script $child
            }
        } finally {
            if ($registered) {
                try { Stop-ScheduledTask -TaskName $taskName -TaskPath $taskPath -ErrorAction SilentlyContinue } finally {
                    Unregister-ScheduledTask -TaskName $taskName -TaskPath $taskPath -Confirm:$false
                }
            }
        }
    } finally {
        if ($reader) { $reader.Dispose() }
        if (Test-Path -LiteralPath $folder) {
            $resolved = (Resolve-Path -LiteralPath $folder).ProviderPath
            $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
            $attributes = (Get-Item -LiteralPath $folder -Force).Attributes
            if (-not $resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or
                (Split-Path -Leaf $resolved) -notlike 'mannie-ci-check-*' -or
                ($attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw 'run-check: refusing to remove a path outside the generated temporary folder'
            }
            # Delete only the files this handover owns. Unexpected contents
            # stop removal without traversing any directory or link.
            foreach ($name in 'configuration.json', 'run-check-task.ps1', 'check.log', 'result.json', 'result.json.pending', 'started.json', 'started.json.pending', 'run') {
                $path = Join-Path $resolved $name
                Remove-CheckFile -Path $path
            }
            Remove-Item -LiteralPath $resolved -Force
        }
    }
}
exit $exitCode
