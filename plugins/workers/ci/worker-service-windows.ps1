# Installs the worker as a Windows service with the system's folders, checks
# that it runs as its virtual account with a configuration that only SYSTEM
# and the administrators can change and a data folder that only they and the
# service's account can open, pairs it with a test master that delegates a
# command, and removes it again (ci.yml). The worker's smoke test of
# plugin.toml: runs elevated on a disposable machine, with the debug target
# folder in which CI built the worker and, with the smoke test's helpers,
# the test master `smoke-master`:
#   ./plugins/workers/ci/worker-service-windows.ps1 target/debug
param(
    [Parameter(Mandatory)]
    [string] $TargetFolder
)
$Worker = Join-Path $TargetFolder 'mannie-worker.exe'
$Master = Join-Path $TargetFolder 'examples/smoke-master.exe'

$ErrorActionPreference = 'Stop'

function Wait-Until([scriptblock] $Condition, [string] $Failure) {
    $deadline = (Get-Date).AddSeconds(60)
    while (-not (& $Condition)) {
        if ((Get-Date) -gt $deadline) {
            throw "worker-service: $Failure"
        }
        Start-Sleep -Seconds 1
    }
}

function Assert-Success([string] $Step) {
    if ($LASTEXITCODE -ne 0) {
        throw "worker-service: $Step failed with exit code $LASTEXITCODE"
    }
}

$config = Join-Path $env:ProgramData 'Mannie\worker'
$data = Join-Path $config 'data'
# The program's folder has a space, which checks that the service's command
# line is quoted.
$program = Join-Path $env:ProgramFiles 'Mannie\Worker\mannie-worker.exe'

& $Worker service install
Assert-Success 'service install'

$database = Join-Path $data 'mannie-worker.sqlite'
$logs = Join-Path $data 'logs\*'
Wait-Until { Test-Path $database } 'the worker did not create its database'
Wait-Until { Select-String -Path $logs -Pattern 'the worker started' -Quiet } 'the worker did not log that it started'

$service = Get-CimInstance -ClassName Win32_Service -Filter "Name = 'mannie-worker'"
if ($service.State -ne 'Running') {
    throw "worker-service: the service is $($service.State)"
}
if ($service.StartName -ne 'NT SERVICE\mannie-worker') {
    throw "worker-service: the service runs as $($service.StartName)"
}

# The guide's next step: an allowed folder, which is the tasks' workspace and
# which the service's account may change, then a restart, which reads the
# configuration again.
$work = Join-Path $env:SystemDrive 'mannie-ci-work'
New-Item -ItemType Directory -Force -Path $work | Out-Null
& (Join-Path $env:SystemRoot 'System32\icacls.exe') $work /grant 'NT SERVICE\mannie-worker:(OI)(CI)M' | Out-Null
Assert-Success 'granting the allowed folder'
$configFile = Join-Path $config 'worker.toml'
$configured = (Get-Content -Raw -Path $configFile).Replace('allowed_folders = []', "allowed_folders = ['$work']")
Set-Content -Path $configFile -Value $configured -NoNewline -Encoding utf8
Restart-Service -Name 'mannie-worker'
Wait-Until { Test-Path (Join-Path $data 'health') } 'the worker did not start again with the allowed folder'
if (-not $service.PathName.StartsWith("`"$program`"")) {
    throw "worker-service: the service runs $($service.PathName)"
}

# Each folder has its own access list, inherited from nothing: ProgramData
# lets every user read and add files.
$serviceSid = (New-Object System.Security.Principal.NTAccount('NT SERVICE\mannie-worker')).Translate([System.Security.Principal.SecurityIdentifier]).Value
$expected = @('S-1-5-18', 'S-1-5-32-544', $serviceSid) | Sort-Object -Unique
foreach ($folder in @($config, $data)) {
    $acl = Get-Acl -Path $folder
    if (-not $acl.AreAccessRulesProtected) {
        throw "worker-service: $folder still inherits access from its parent"
    }
    $sids = $acl.Access | ForEach-Object { $_.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value } | Sort-Object -Unique
    if (Compare-Object -ReferenceObject $expected -DifferenceObject $sids) {
        throw "worker-service: $folder grants access to $($sids -join ', ')"
    }
}
# The service's account can only read the configuration.
$writeRights = [System.Security.AccessControl.FileSystemRights]'WriteData, AppendData, Delete, ChangePermissions, TakeOwnership'
$serviceRules = (Get-Acl -Path $config).Access | Where-Object { $_.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value -eq $serviceSid }
foreach ($rule in $serviceRules) {
    if ($rule.FileSystemRights -band $writeRights) {
        throw "worker-service: the service's account has $($rule.FileSystemRights) on the configuration"
    }
}

$paired = & $program pair
Assert-Success 'pair'
$code = ($paired | Select-String -Pattern '^\s+Code:\s+(\S+)').Matches.Groups[1].Value
if (-not $code) {
    throw "worker-service: pair printed no code: $paired"
}
& $Master 127.0.0.1:7445 $code
Assert-Success 'the test master'
$status = & $program status
Assert-Success 'status'
if (-not ($status | Select-String -Pattern '^Pairing: not paired' -Quiet)) {
    throw "worker-service: the worker is still paired after the master removed it: $status"
}

& $program service uninstall
Assert-Success 'service uninstall'
Wait-Until { -not (Get-Service -Name 'mannie-worker' -ErrorAction SilentlyContinue) } 'the service is still installed'
if (-not (Test-Path $database)) {
    throw 'worker-service: uninstall removed the data folder'
}
if (-not (Test-Path (Join-Path $config 'worker.toml'))) {
    throw 'worker-service: uninstall removed the configuration'
}
if (-not (Select-String -Path $logs -Pattern 'the worker stopped' -Quiet)) {
    throw 'worker-service: the worker did not stop cleanly'
}
Write-Output 'worker-service: installed, ran as NT SERVICE\mannie-worker with a configuration it cannot change, ran a delegated command, stopped cleanly and was removed'
