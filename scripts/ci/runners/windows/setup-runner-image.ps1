# Prepares a Windows 11 runner image (docs/engineering/ci-runners.md,
# section 6): it checks the software that the jobs expect, installs the newest
# runner application without registering it, creates the folder that the host
# copies the runner's configuration into, registers the startup task that runs
# the runner for one job at every sign-in, sets the power and update settings
# of a machine that is reset after every job, and sends name lookups over
# HTTPS. Run it in an elevated
# Windows PowerShell, signed in as the administrator who signs in
# automatically, from the folder that also holds start-runner.ps1:
#   powershell -ExecutionPolicy Bypass -File .\setup-runner-image.ps1
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
# Windows PowerShell downloads slowly while it draws a progress bar.
$ProgressPreference = 'SilentlyContinue'

$runnerFolder = 'C:\actions-runner'
$programFolder = Join-Path $env:ProgramFiles 'mannie-runner'
# The host's run-runner-cycle.ps1 copies the configuration into this folder.
$inbox = Join-Path $env:ProgramData 'mannie-runner'
$taskName = 'Mannie runner'

function Assert-Success {
    param(
        [Parameter(Mandatory)]
        [string] $Step
    )

    if ($LASTEXITCODE -ne 0) {
        throw "setup-runner-image: $Step failed with exit code $LASTEXITCODE"
    }
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'setup-runner-image: run it in an elevated PowerShell'
}
$user = $identity.Name

# The software of docs/engineering/ci-runners.md, section 3, which the owner
# installs before running this script.
$missing = @()
$git = Get-Command git.exe -ErrorAction SilentlyContinue
if (-not $git) {
    $missing += 'Git for Windows'
} else {
    $gitRoot = Split-Path -Parent (Split-Path -Parent $git.Source)
    if (-not (Test-Path -LiteralPath (Join-Path $gitRoot 'bin\bash.exe'))) {
        $missing += 'Git Bash from Git for Windows'
    }
}
if (-not (Get-Command pwsh.exe -ErrorAction SilentlyContinue)) {
    $missing += 'PowerShell 7'
}
if (-not (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'))) {
    $missing += 'rustup'
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$cppTools = $null
if (Test-Path -LiteralPath $vswhere) {
    $cppTools = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
}
if (-not $cppTools) {
    $missing += "Visual Studio's C++ build tools"
}
if ($missing.Count -gt 0) {
    throw "setup-runner-image: install $($missing -join ', ') first"
}

# The runner application, unregistered: each cycle registers it for one job.
if (Test-Path -LiteralPath (Join-Path $runnerFolder '.runner')) {
    throw "setup-runner-image: the runner in $runnerFolder is registered; an image holds an unregistered runner"
}
$release = Invoke-RestMethod -Uri 'https://api.github.com/repos/actions/runner/releases/latest' -UserAgent 'mannie-ci-runners'
$version = $release.tag_name.TrimStart('v')
$assetName = "actions-runner-win-x64-$version.zip"
$asset = @($release.assets | Where-Object { $_.name -eq $assetName })
if ($asset.Count -ne 1) {
    throw "setup-runner-image: the runner release $version has no $assetName"
}
$archive = Join-Path $env:TEMP $assetName
Invoke-WebRequest -Uri $asset[0].browser_download_url -OutFile $archive -UseBasicParsing
# GitHub publishes the SHA-256 digest of every release asset (verified on
# 2026-10-16).
$expected = $asset[0].digest -replace '^sha256:', ''
$actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
if ($actual -ne $expected) {
    throw "setup-runner-image: $assetName has the SHA-256 digest $actual, but GitHub publishes $expected"
}
if (Test-Path -LiteralPath $runnerFolder) {
    Remove-Item -LiteralPath $runnerFolder -Recurse -Force
}
Expand-Archive -LiteralPath $archive -DestinationPath $runnerFolder
Remove-Item -LiteralPath $archive
Write-Output "setup-runner-image: installed the runner $version in $runnerFolder"

# Only SYSTEM, which runs the Guest Service Interface that writes the host's
# files, and the administrators may open the inbox.
New-Item -ItemType Directory -Force -Path $inbox | Out-Null
$acl = [System.Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
foreach ($sid in 'S-1-5-18', 'S-1-5-32-544') {
    $account = [System.Security.Principal.SecurityIdentifier]::new($sid)
    $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($account, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
}
Set-Acl -LiteralPath $inbox -AclObject $acl
Get-ChildItem -LiteralPath $inbox -Force | Remove-Item -Recurse -Force

New-Item -ItemType Directory -Force -Path $programFolder | Out-Null
$startScript = Join-Path $programFolder 'start-runner.ps1'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'start-runner.ps1') -Destination $startScript -Force

# The task runs in the automatic sign-in's session, with the administrator's
# full rights and without a window. Task Scheduler runs tasks below normal
# priority, which their processes inherit, and stops them after three days;
# the runner and its jobs run at normal priority for as long as a job takes.
$powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$action = New-ScheduledTaskAction -Execute $powershell -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$startScript`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
$taskPrincipal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 4 -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $taskName -TaskPath '\Mannie\' -Action $action -Trigger $trigger -Principal $taskPrincipal -Settings $settings -Force | Out-Null
Write-Output "setup-runner-image: registered the task \Mannie\$taskName for $user"

# A sleeping or hibernating machine stops its job, and turning hibernation off
# also turns off fast startup, so every boot starts from the checkpoint's disk.
& powercfg.exe /hibernate off
Assert-Success 'turning hibernation off'
foreach ($setting in 'standby-timeout-ac', 'monitor-timeout-ac') {
    & powercfg.exe /change $setting 0
    Assert-Success "setting $setting"
}

# An update could restart the machine in the middle of a job, and the next
# restore throws it away. The owner updates the image deliberately instead.
$updatePolicy = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU'
New-Item -Path $updatePolicy -Force | Out-Null
Set-ItemProperty -Path $updatePolicy -Name 'NoAutoUpdate' -Type DWord -Value 1

# Name lookups go over HTTPS, which is TCP: the host's route to the internet
# may lose UDP, as the owner's lost up to 60 percent of plain lookups while
# every lookup over TCP passed (observed on 2026-10-19). Windows knows the
# templates of 1.1.1.1 and 9.9.9.9, the name servers of the guide, and with
# these settings it upgrades every lookup to them to HTTPS and never falls
# back to UDP.
foreach ($server in '1.1.1.1', '9.9.9.9') {
    if (-not (Get-DnsClientDohServerAddress -ServerAddress $server -ErrorAction SilentlyContinue)) {
        throw "setup-runner-image: Windows has no DNS over HTTPS template for $server"
    }
    Set-DnsClientDohServerAddress -ServerAddress $server -AutoUpgrade $true -AllowFallbackToUdp $false
}
$nameServers = @(Get-DnsClientServerAddress -AddressFamily IPv4 | Where-Object { $_.ServerAddresses } | ForEach-Object { $_.ServerAddresses })
foreach ($nameServer in $nameServers) {
    if ($nameServer -notin '1.1.1.1', '9.9.9.9') {
        Write-Output "setup-runner-image: the name server $nameServer has no DNS over HTTPS setting; use 1.1.1.1 and 9.9.9.9"
    }
}

$winlogon = Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
$autoLogon = $winlogon.PSObject.Properties['AutoAdminLogon']
if (-not $autoLogon -or $autoLogon.Value -ne '1') {
    Write-Output "setup-runner-image: automatic sign-in is off; turn it on for $user with Sysinternals Autologon"
}
Write-Output 'setup-runner-image: done; restart to check that the task waits for the host, then shut down and take the clean checkpoint on the host'
