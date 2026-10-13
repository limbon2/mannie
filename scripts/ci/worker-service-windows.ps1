# Installs the worker as a Windows service, checks that it runs as its
# virtual account and opens its store in a folder that only that account,
# SYSTEM and the administrators can open, and removes it again (ci.yml).
# Runs elevated on a disposable machine:
#   ./scripts/ci/worker-service-windows.ps1 -Worker target/debug/mannie-worker.exe
param(
    [Parameter(Mandatory)]
    [string] $Worker
)

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

# A folder with a space checks that the service's command line is quoted.
$installFolder = Join-Path $env:ProgramFiles 'Mannie Worker CI'
$data = Join-Path $env:ProgramData 'Mannie Worker CI\data'
New-Item -ItemType Directory -Force -Path $installFolder | Out-Null
$executable = Join-Path $installFolder 'mannie-worker.exe'
Copy-Item -Path $Worker -Destination $executable

& $executable service install --data-dir $data
if ($LASTEXITCODE -ne 0) {
    throw "worker-service: service install failed with exit code $LASTEXITCODE"
}

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

# Only SYSTEM, the administrators and the service account have access, and
# nothing is inherited from ProgramData, which lets every user read.
$serviceSid = (New-Object System.Security.Principal.NTAccount('NT SERVICE\mannie-worker')).Translate([System.Security.Principal.SecurityIdentifier]).Value
$acl = Get-Acl -Path $data
if (-not $acl.AreAccessRulesProtected) {
    throw 'worker-service: the data folder still inherits access from its parent'
}
$sids = $acl.Access | ForEach-Object { $_.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value } | Sort-Object -Unique
$expected = @('S-1-5-18', 'S-1-5-32-544', $serviceSid) | Sort-Object -Unique
if (Compare-Object -ReferenceObject $expected -DifferenceObject $sids) {
    throw "worker-service: the data folder grants access to $($sids -join ', ')"
}

& $executable service uninstall
if ($LASTEXITCODE -ne 0) {
    throw "worker-service: service uninstall failed with exit code $LASTEXITCODE"
}
Wait-Until { -not (Get-Service -Name 'mannie-worker' -ErrorAction SilentlyContinue) } 'the service is still installed'
if (-not (Test-Path $database)) {
    throw 'worker-service: uninstall removed the data folder'
}
if (-not (Select-String -Path $logs -Pattern 'the worker stopped' -Quiet)) {
    throw 'worker-service: the worker did not stop cleanly'
}
Write-Output 'worker-service: installed, ran as NT SERVICE\mannie-worker with a private data folder, stopped cleanly and was removed'
