# Shared by the host scripts of the self-hosted runners (docs/engineering/
# ci-runners.md, section 6): the settings folder, the configuration, the GitHub
# token, the REST API calls and the checks of a runner machine. The scripts run
# in Windows PowerShell 5.1 and in PowerShell 7.

Set-StrictMode -Version 3.0

# Windows PowerShell started by PowerShell 7 inherits the Core module path.
# Its built-in modules must come from the machine's Windows PowerShell path.
if ($PSVersionTable.PSVersion.Major -le 5) {
    $env:PSModulePath = [Environment]::GetEnvironmentVariable('PSModulePath', 'Machine')
}

# The REST API version these calls were written for (verified on 2026-10-16).
$apiVersion = '2026-03-10'

# The networks a runner machine never reaches: the private, shared and
# link-local ranges, which hold the host, the owner's network, VPNs and the
# other machines, then multicast, and the reserved range with the broadcast
# address. All of IPv6 is closed, because the NAT carries only IPv4. The
# virtual switch enforces these rules as port ACLs, and for each packet the
# rule with the longest matching prefix applies (verified on 2026-10-16), so
# the allow rule for a guest's gateway, a single address, outranks them.
$blockedNetworks = @(
    '10.0.0.0/8'
    '100.64.0.0/10'
    '169.254.0.0/16'
    '172.16.0.0/12'
    '192.168.0.0/16'
    '224.0.0.0/4'
    '240.0.0.0/4'
    '::/0'
)

# The weights of the extended port ACLs that keep a guest away from the host
# while it uses the host as its gateway. Among extended port ACLs, the rule
# with the higher weight wins, so SSH from the host outranks the guard.
$hostGuardWeight = 10
$hostSshWeight = 20

# The Guest Service Interface, which carries Copy-VMFile, by the identifier that
# ends the Id of its integration service.
$guestServiceId = '6C09BB55-D683-4DA0-8931-C9BF705F6480'

$token = $null

function Get-RunnerSettingsFolder {
    # The configuration, the encrypted token, the log and the files on their way
    # into a guest live in the profile of the Windows user that runs the scripts.
    $folder = Join-Path $env:LOCALAPPDATA 'Mannie\ci-runners'
    if (-not (Test-Path -LiteralPath $folder)) {
        New-Item -ItemType Directory -Path $folder | Out-Null
    }
    return $folder
}

function Get-DefaultConfigPath {
    return Join-Path (Get-RunnerSettingsFolder) 'runners.psd1'
}

function Enter-ControllerLock {
    param(
        [Parameter(Mandatory)]
        [ValidatePattern('^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$')]
        [string] $Repository
    )

    # A scheduled task and a manual start must not restore the same guest
    # while another controller owns its job. The lock spans Windows sessions.
    $name = 'Global\Mannie.CI.' + $Repository.Replace('/', '.')
    $mutex = [System.Threading.Mutex]::new($false, $name)
    try {
        try {
            $acquired = $mutex.WaitOne(0)
        } catch [System.Threading.AbandonedMutexException] {
            # The earlier process ended without releasing the lock. The
            # controller reconciles its machines and registrations at startup.
            $acquired = $true
        }
        if (-not $acquired) {
            throw "runners: another controller is already serving $Repository"
        }
        return $mutex
    } catch {
        $mutex.Dispose()
        throw
    }
}

function Get-BlockedNetworks {
    return $blockedNetworks
}

function Get-SwitchGateway {
    param(
        [Parameter(Mandatory)]
        [string] $SwitchName
    )

    # The host's address on an internal switch is the guests' gateway to the
    # NAT.
    $alias = "vEthernet ($SwitchName)"
    $addresses = @(Get-NetIPAddress -InterfaceAlias $alias -AddressFamily IPv4 -ErrorAction SilentlyContinue)
    if ($addresses.Count -ne 1) {
        throw "runners: the host needs one IPv4 address on $alias, the guests' gateway, and has $($addresses.Count)"
    }
    return $addresses[0].IPAddress
}

function ConvertTo-IpPrefix {
    param([string] $Text)

    # Hyper-V normalizes addresses and may omit a host's /32 or /128. Compare
    # network bytes, not spelling, including expanded IPv6 and host bits.
    $parts = $Text.Split('/')
    $address = $null
    if ($parts.Count -gt 2 -or $Text.Contains('%') -or -not [Net.IPAddress]::TryParse($parts[0], [ref]$address)) { return $null }
    $bytes = $address.GetAddressBytes()
    $length = $bytes.Length * 8
    if ($parts.Count -eq 2) {
        if (-not [int]::TryParse($parts[1], [ref]$length) -or $length -lt 0 -or $length -gt $bytes.Length * 8) { return $null }
    }
    for ($index = 0; $index -lt $bytes.Length; $index += 1) {
        $remaining = $length - $index * 8
        if ($remaining -le 0) { $bytes[$index] = 0 }
        elseif ($remaining -lt 8) { $bytes[$index] = $bytes[$index] -band (255 -shl (8 - $remaining)) }
    }
    return [pscustomobject]@{ Family = $bytes.Length; Length = $length; Network = [Convert]::ToBase64String($bytes) }
}

function Test-AclRemoteAddress {
    param(
        [Parameter(Mandatory)] [object] $Rule,
        [Parameter(Mandatory)] [string] $Address
    )

    # Basic and extended port ACLs use different property names.
    $remote = $Rule.PSObject.Properties['RemoteAddress']
    if (-not $remote) { $remote = $Rule.PSObject.Properties['RemoteIPAddress'] }
    if (-not $remote) { return $false }
    $actual = ConvertTo-IpPrefix ([string]$remote.Value)
    $expected = ConvertTo-IpPrefix $Address
    return $null -ne $actual -and $null -ne $expected -and $actual.Family -eq $expected.Family -and $actual.Length -eq $expected.Length -and $actual.Network -eq $expected.Network
}

function Test-HostGuardRule {
    param(
        [Parameter(Mandatory)]
        [object] $Rule,
        [Parameter(Mandatory)]
        [string] $Direction,
        [Parameter(Mandatory)]
        [string] $Gateway
    )

    # An extended port ACL that refuses all IP traffic in one direction with
    # the host's address on the switch. The port ACLs must allow that address,
    # since their deny rules also drop ARP, but extended port ACLs filter IP
    # traffic alone, so the guest keeps its gateway and loses the host's
    # services, whatever the host's firewall does (observed on 2026-10-19).
    return $Rule.Direction -eq $Direction -and $Rule.Action -eq 'Deny' -and (Test-AclRemoteAddress -Rule $Rule -Address $Gateway) -and $Rule.Protocol -eq 'ANY' -and $Rule.LocalPort -eq 'ANY' -and $Rule.RemotePort -eq 'ANY' -and $Rule.Weight -eq $hostGuardWeight
}

function Test-HostSshRule {
    param(
        [Parameter(Mandatory)]
        [object] $Rule,
        [Parameter(Mandatory)]
        [string] $Gateway
    )

    # The extended port ACL that lets the host open SSH connections to the
    # guest. It is stateful, so only the replies of the host's own connections
    # pass the guard rules, and the guest opens no connection to the host
    # (observed on 2026-10-19).
    return $Rule.Direction -eq 'Inbound' -and $Rule.Action -eq 'Allow' -and (Test-AclRemoteAddress -Rule $Rule -Address $Gateway) -and $Rule.Protocol -eq 'TCP' -and $Rule.LocalPort -eq '22' -and $Rule.RemotePort -eq 'ANY' -and $Rule.Stateful -and $Rule.Weight -eq $hostSshWeight
}

function Add-HostGuard {
    param(
        [Parameter(Mandatory)]
        [object] $VMNetworkAdapter,
        [Parameter(Mandatory)]
        [string] $Gateway
    )

    # Adds the extended port ACLs of Test-HostGuardRule and Test-HostSshRule
    # that the adapter lacks.
    $extended = @(Get-VMNetworkAdapterExtendedAcl -VMNetworkAdapter $VMNetworkAdapter)
    foreach ($direction in 'Inbound', 'Outbound') {
        $guards = @($extended | Where-Object { Test-HostGuardRule -Rule $_ -Direction $direction -Gateway $Gateway })
        if ($guards.Count -eq 0) {
            Add-VMNetworkAdapterExtendedAcl -VMNetworkAdapter $VMNetworkAdapter -Action Deny -Direction $direction -RemoteIPAddress $Gateway -Weight $hostGuardWeight
        }
    }
    if (@($extended | Where-Object { Test-HostSshRule -Rule $_ -Gateway $Gateway }).Count -eq 0) {
        Add-VMNetworkAdapterExtendedAcl -VMNetworkAdapter $VMNetworkAdapter -Action Allow -Direction Inbound -RemoteIPAddress $Gateway -Protocol TCP -LocalPort 22 -Stateful $true -Weight $hostSshWeight
    }
}

function Read-RunnerConfig {
    param(
        [Parameter(Mandatory)]
        [string] $Path
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "runners: there is no configuration at $Path; copy scripts/ci/runners/host/runners.example.psd1 there and name your machines in it"
    }
    $config = Import-PowerShellDataFile -LiteralPath $Path
    foreach ($key in 'Repository', 'Checkpoint', 'MaxRunning', 'Machines') {
        if (-not $config.ContainsKey($key)) {
            throw "runners: $Path has no $key"
        }
    }
    if ($config.Repository -notmatch '^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$') {
        throw "runners: Repository in $Path is not owner/name: $($config.Repository)"
    }
    if ($config.MaxRunning -isnot [int] -or $config.MaxRunning -lt 1) {
        throw "runners: MaxRunning in $Path must be a whole number of at least 1"
    }

    $names = @{}
    foreach ($machine in @($config.Machines)) {
        foreach ($key in 'Name', 'Guest', 'Labels') {
            if (-not $machine.ContainsKey($key)) {
                throw "runners: a machine in $Path has no $key"
            }
        }
        if ($names.ContainsKey($machine.Name)) {
            throw "runners: $Path names the machine $($machine.Name) twice"
        }
        $names[$machine.Name] = $true
        if ($machine.Guest -notin 'Windows', 'Ubuntu') {
            throw "runners: the guest of $($machine.Name) must be Windows or Ubuntu"
        }
        $labels = @($machine.Labels)
        # A just-in-time runner gets exactly the labels of its request, and the
        # jobs ask for both of these (docs/engineering/ci-runners.md, section 2).
        foreach ($label in 'self-hosted', 'disposable') {
            if ($labels -notcontains $label) {
                throw "runners: the labels of $($machine.Name) lack $label"
            }
        }
        if ($labels.Count -lt 3) {
            throw "runners: the labels of $($machine.Name) lack the label of its system"
        }
        if (@($labels | Sort-Object -Unique).Count -ne $labels.Count) {
            throw "runners: the labels of $($machine.Name) repeat a label"
        }
    }
    if ($names.Count -eq 0) {
        throw "runners: $Path names no machines"
    }
    return $config
}

function Format-RunnerLog {
    param(
        [Parameter(Mandatory)]
        [string] $Text
    )

    return '{0:yyyy-MM-dd HH:mm:ss} {1}' -f (Get-Date), $Text
}

function Save-GitHubToken {
    param(
        [Parameter(Mandatory)]
        [securestring] $Token
    )

    $path = Join-Path (Get-RunnerSettingsFolder) 'github-token'
    # Without a key, ConvertFrom-SecureString encrypts with DPAPI, which only
    # this Windows user on this machine can decrypt.
    Set-Content -LiteralPath $path -Value (ConvertFrom-SecureString -SecureString $Token) -Encoding Ascii
}

function Connect-GitHub {
    param(
        # A token to check before it is saved. Without one, the saved token.
        [securestring] $Token
    )

    if ($Token) {
        $script:token = $Token
        return
    }
    $path = Join-Path (Get-RunnerSettingsFolder) 'github-token'
    if (-not (Test-Path -LiteralPath $path)) {
        throw 'runners: there is no GitHub token; run scripts/ci/runners/host/save-github-token.ps1 first'
    }
    try {
        $script:token = ConvertTo-SecureString -String (Get-Content -LiteralPath $path -Raw).Trim()
    } catch {
        throw "runners: $env:USERDOMAIN\$env:USERNAME cannot decrypt $path; save the token again as the Windows user that runs the runner scripts"
    }
}

function Get-HttpStatus {
    param(
        [Parameter(Mandatory)]
        [System.Management.Automation.ErrorRecord] $ErrorRecord
    )

    # Windows PowerShell throws a WebException and PowerShell 7 an
    # HttpResponseException; both carry the response, and network failures
    # carry none.
    $response = $ErrorRecord.Exception.PSObject.Properties['Response']
    if ($response -and $response.Value) {
        return [int] $response.Value.StatusCode
    }
    return 0
}

function Read-ErrorBody {
    param(
        [Parameter(Mandatory)]
        [System.Management.Automation.ErrorRecord] $ErrorRecord
    )

    # PowerShell 7 puts the body of an error response in ErrorDetails, and
    # Windows PowerShell leaves it in the response's stream.
    if ($ErrorRecord.ErrorDetails -and $ErrorRecord.ErrorDetails.Message) {
        return $ErrorRecord.ErrorDetails.Message
    }
    $response = $ErrorRecord.Exception.PSObject.Properties['Response']
    if ($response -and $response.Value -and $response.Value.PSObject.Methods['GetResponseStream']) {
        $reader = New-Object System.IO.StreamReader($response.Value.GetResponseStream())
        try {
            return $reader.ReadToEnd()
        } finally {
            $reader.Dispose()
        }
    }
    return ''
}

function Get-GitHubError {
    param(
        [Parameter(Mandatory)]
        [System.Management.Automation.ErrorRecord] $ErrorRecord,
        [int] $Status
    )

    $message = $ErrorRecord.Exception.Message
    $body = Read-ErrorBody -ErrorRecord $ErrorRecord
    if ($body) {
        $message = $body
        try {
            $message = (ConvertFrom-Json -InputObject $body).message
        } catch {
            # The body is not GitHub's JSON error, so it is shown as it came.
        }
    }
    if ($Status -ne 0) {
        return "HTTP $Status, $message"
    }
    return $message
}

function Invoke-GitHubApi {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('GET', 'POST', 'DELETE')]
        [string] $Method,
        [Parameter(Mandatory)]
        [string] $Path,
        [hashtable] $Body,
        # Returns nothing for 404 Not Found, as for a runner that GitHub removed
        # after its job.
        [switch] $AllowNotFound
    )

    if ($null -eq $script:token) {
        throw 'runners: Connect-GitHub has not run'
    }
    # The token is in plain text only in the headers of this request.
    $headers = @{
        Accept                 = 'application/vnd.github+json'
        Authorization          = 'Bearer ' + (New-Object System.Net.NetworkCredential('', $script:token)).Password
        'X-GitHub-Api-Version' = $apiVersion
    }
    $request = @{
        Method    = $Method
        Uri       = "https://api.github.com/$Path"
        Headers   = $headers
        UserAgent = 'mannie-ci-runners'
        TimeoutSec = 30
    }
    # Since 7.4, TimeoutSec limits connection establishment only. Bound reads
    # as well; Windows PowerShell 5.1 uses its request timeout (verified on
    # 2026-10-20, linked in docs/engineering/ci-runners.md).
    if ($PSVersionTable.PSVersion -ge [version]'7.4') { $request.OperationTimeoutSeconds = 30 }
    if ($Body) {
        $request.Body = ConvertTo-Json -InputObject $Body -Compress
        $request.ContentType = 'application/json; charset=utf-8'
    }
    try {
        return Invoke-RestMethod @request
    } catch {
        $status = Get-HttpStatus -ErrorRecord $_
        if ($AllowNotFound -and $status -eq 404) {
            return $null
        }
        throw "GitHub: $Method /$Path failed: $(Get-GitHubError -ErrorRecord $_ -Status $status)"
    }
}

function Get-GitHubList {
    param(
        [Parameter(Mandatory)]
        [string] $Path,
        # The property of the response that holds the items, next to total_count.
        [Parameter(Mandatory)]
        [string] $Property
    )

    $separator = '?'
    if ($Path.Contains('?')) {
        $separator = '&'
    }
    $items = New-Object System.Collections.Generic.List[object]
    $page = 1
    while ($true) {
        $response = Invoke-GitHubApi -Method GET -Path "$Path$($separator)per_page=100&page=$page"
        $batch = @($response.$Property)
        foreach ($item in $batch) {
            $items.Add($item)
        }
        if ($batch.Count -lt 100 -or $items.Count -ge $response.total_count) {
            break
        }
        $page += 1
    }
    return $items.ToArray()
}

function Test-RepositoryAccess {
    param(
        [Parameter(Mandatory)]
        [string] $Repository
    )

    # The runner endpoints need the token's Administration permission, and the
    # queue its Actions permission. A request to create a runner would also
    # prove write access, but it would register one.
    Invoke-GitHubApi -Method GET -Path "repos/$Repository/actions/runners?per_page=1" | Out-Null
    Invoke-GitHubApi -Method GET -Path "repos/$Repository/actions/runs?per_page=1" | Out-Null
}

function Get-QueuedRunnerJobs {
    param(
        [Parameter(Mandatory)]
        [string] $Repository
    )

    # A run whose jobs all wait is queued, and one that started some of its jobs
    # is in progress.
    $runs = @()
    foreach ($status in 'queued', 'in_progress') {
        $runs += @(Get-GitHubList -Path "repos/$Repository/actions/runs?status=$status" -Property 'workflow_runs')
    }
    # A run that changed its status between the two requests is listed twice.
    $jobs = @{}
    foreach ($run in $runs) {
        foreach ($job in @(Get-GitHubList -Path "repos/$Repository/actions/runs/$($run.id)/jobs?filter=latest" -Property 'jobs')) {
            # A job without the self-hosted label is for GitHub's own runners.
            if ($job.status -eq 'queued' -and @($job.labels) -contains 'self-hosted') {
                $jobs[[string] $job.id] = $job
            }
        }
    }
    return @($jobs.Values | Sort-Object -Property created_at)
}

function Get-RepositoryRunners {
    param(
        [Parameter(Mandatory)]
        [string] $Repository
    )

    return Get-GitHubList -Path "repos/$Repository/actions/runners" -Property 'runners'
}

function New-JitRunner {
    param(
        [Parameter(Mandatory)]
        [string] $Repository,
        [Parameter(Mandatory)]
        [string] $Name,
        [Parameter(Mandatory)]
        [string[]] $Labels
    )

    $body = @{
        name            = $Name
        # A repository's runners belong to the default group.
        runner_group_id = 1
        labels          = $Labels
        work_folder     = '_work'
    }
    $response = Invoke-GitHubApi -Method POST -Path "repos/$Repository/actions/runners/generate-jitconfig" -Body $body
    return [pscustomobject]@{
        Id        = $response.runner.id
        Name      = $response.runner.name
        JitConfig = $response.encoded_jit_config
    }
}

function Get-Runner {
    param(
        [Parameter(Mandatory)]
        [string] $Repository,
        [Parameter(Mandatory)]
        [long] $Id
    )

    return Invoke-GitHubApi -Method GET -Path "repos/$Repository/actions/runners/$Id" -AllowNotFound
}

function Remove-Runner {
    param(
        [Parameter(Mandatory)]
        [string] $Repository,
        [Parameter(Mandatory)]
        [long] $Id
    )

    # Returns whether there was a registration to remove: GitHub removes a
    # just-in-time runner by itself after its job.
    if ($null -eq (Get-Runner -Repository $Repository -Id $Id)) {
        return $false
    }
    Invoke-GitHubApi -Method DELETE -Path "repos/$Repository/actions/runners/$Id" -AllowNotFound | Out-Null
    return $true
}

function Test-LabelsCover {
    param(
        [Parameter(Mandatory)]
        [string[]] $RunnerLabels,
        [Parameter(Mandatory)]
        [string[]] $JobLabels
    )

    # A runner can take a job when it has every label the job asks for. Labels
    # are case-insensitive, as -contains is.
    foreach ($label in $JobLabels) {
        if ($RunnerLabels -notcontains $label) {
            return $false
        }
    }
    return $true
}

function Select-NeededCycles {
    param(
        # Each machine has Name, Labels, InCycle, Inherited (running a job that
        # began before run-runners.ps1 started), Paused (waiting after cycles
        # without a job) and Running (on while it is in no cycle).
        [object[]] $Machines,
        # The queued jobs, oldest first.
        [object[]] $Queued,
        [object[]] $Runners,
        [Parameter(Mandatory)]
        [int] $MaxRunning
    )

    # Decides which machines start a cycle for which queued job. Returns the
    # starts, the jobs that no machine has the labels for, and the machines
    # that could take a job but run without a cycle.
    $starts = New-Object System.Collections.Generic.List[object]
    $unserved = New-Object System.Collections.Generic.List[object]
    $blocked = @{}
    $chosen = @{}
    $slots = $MaxRunning - @($Machines | Where-Object { $_.InCycle -or $_.Inherited }).Count

    # A machine in a cycle whose runner has not taken a job yet will take one of
    # the queued jobs, so that job needs no other machine.
    $spare = @{}
    foreach ($machine in @($Machines | Where-Object { $_.InCycle })) {
        $busy = @($Runners | Where-Object { $_.busy -and (Test-RunnerOfMachine -RunnerName $_.name -Machine $machine.Name) })
        if ($busy.Count -eq 0) {
            $spare[$machine.Name] = $machine
        }
    }

    foreach ($job in $Queued) {
        $labels = [string[]] @($job.labels)
        $taker = @($spare.Values | Where-Object { Test-LabelsCover -RunnerLabels $_.Labels -JobLabels $labels }) | Select-Object -First 1
        if ($taker) {
            $spare.Remove($taker.Name)
            continue
        }
        $able = @($Machines | Where-Object { Test-LabelsCover -RunnerLabels $_.Labels -JobLabels $labels })
        if ($able.Count -eq 0) {
            $unserved.Add($job)
            continue
        }
        if ($slots -le 0) {
            continue
        }
        $free = @($able | Where-Object { -not $_.InCycle -and -not $_.Inherited -and -not $_.Paused -and -not $chosen.ContainsKey($_.Name) })
        foreach ($candidate in $free) {
            if ($candidate.Running) {
                $blocked[$candidate.Name] = $candidate
                continue
            }
            $chosen[$candidate.Name] = $true
            $starts.Add([pscustomobject]@{ Machine = $candidate; Job = $job })
            $slots -= 1
            break
        }
    }
    return [pscustomobject]@{
        Starts   = $starts.ToArray()
        Unserved = $unserved.ToArray()
        Blocked  = @($blocked.Values)
    }
}

function New-RunnerName {
    param(
        [Parameter(Mandatory)]
        [string] $Machine
    )

    # A runner's name starts with its machine's, so the scripts can tell which
    # registrations a machine left behind.
    return '{0}-{1:yyyyMMdd-HHmmss}' -f $Machine, (Get-Date).ToUniversalTime()
}

function Test-RunnerOfMachine {
    param(
        [Parameter(Mandatory)]
        [string] $RunnerName,
        [Parameter(Mandatory)]
        [string] $Machine
    )

    return $RunnerName -match ('^' + [regex]::Escape($Machine) + '-\d{8}-\d{6}$')
}

function Stop-RunnerMachine {
    param(
        [Parameter(Mandatory)]
        [string] $Machine
    )

    # The next cycle restores the checkpoint, so nothing on the machine needs to
    # be saved or shut down cleanly.
    $vm = Get-VM -Name $Machine
    if ($vm.State -eq 'Saved') {
        Remove-VMSavedState -VM $vm
    } elseif ($vm.State -ne 'Off') {
        Stop-VM -VM $vm -TurnOff -Confirm:$false
    }
}

function Get-RunnerMachineProblems {
    param(
        [Parameter(Mandatory)]
        [string] $Machine
    )

    # A checkpoint keeps the machine's settings, so a checkpoint taken before the
    # machine was isolated takes the isolation away again when it is restored.
    # run-runner-cycle.ps1 checks every restored machine with this.
    $vm = Get-VM -Name $Machine
    $problems = New-Object System.Collections.Generic.List[string]
    if ($vm.AutomaticCheckpointsEnabled) {
        $problems.Add('automatic checkpoints are on, so every start adds a checkpoint')
    }
    $guestService = Get-VMIntegrationService -VM $vm | Where-Object { $_.Id -like "*\$guestServiceId" }
    if (-not $guestService -or -not $guestService.Enabled) {
        $problems.Add('Guest services are off, so the host cannot copy the runner configuration into the guest')
    }
    $adapters = @(Get-VMNetworkAdapter -VM $vm)
    if ($adapters.Count -eq 0) { $problems.Add('the machine has no network adapter') }
    foreach ($adapter in $adapters) {
        $gateway = $null
        $rules = @(Get-VMNetworkAdapterAcl -VMNetworkAdapter $adapter)
        if ($adapter.SwitchName) {
            $switch = Get-VMSwitch -Name $adapter.SwitchName
            if ($switch.SwitchType -ne 'Internal') {
                $problems.Add("$($adapter.Name) is connected to the $($switch.SwitchType) switch $($switch.Name), not to an internal switch behind the NAT")
            } else {
                # The deny rules also drop ARP for the addresses they cover, so
                # the guest reaches nothing unless its gateway is allowed.
                $gateway = Get-SwitchGateway -SwitchName $switch.Name
                foreach ($direction in 'Inbound', 'Outbound') {
                    $allowed = @($rules | Where-Object { $_.Direction -eq $direction -and $_.Action -eq 'Allow' -and (Test-AclRemoteAddress -Rule $_ -Address $gateway) })
                    if ($allowed.Count -eq 0) {
                        $problems.Add("$($adapter.Name) does not allow $($direction.ToLower()) traffic with its gateway $gateway, so the guest has no network")
                    }
                }
                # The gateway is the host, whose services would answer jobs
                # whenever its firewall is off.
                $extended = @(Get-VMNetworkAdapterExtendedAcl -VMNetworkAdapter $adapter)
                foreach ($direction in 'Inbound', 'Outbound') {
                    $guards = @($extended | Where-Object { Test-HostGuardRule -Rule $_ -Direction $direction -Gateway $gateway })
                    if ($guards.Count -eq 0) {
                        $problems.Add("$($adapter.Name) does not refuse $($direction.ToLower()) IP traffic with the host at $gateway, so jobs could reach the host's services")
                    }
                }
                foreach ($rule in @($extended | Where-Object { $_.Action -eq 'Allow' -and -not (Test-HostSshRule -Rule $_ -Gateway $gateway) })) {
                    $problems.Add("$($adapter.Name) has an extended port ACL that allows $("$($rule.Direction)".ToLower()) traffic with $($rule.RemoteIPAddress), which could let jobs reach the host")
                }
            }
        } else {
            $problems.Add("$($adapter.Name) is not connected to a switch")
        }
        if ($adapter.DhcpGuard -ne 'On') {
            $problems.Add("$($adapter.Name) has DHCP guard off")
        }
        if ($adapter.RouterGuard -ne 'On') {
            $problems.Add("$($adapter.Name) has router guard off")
        }
        if ($adapter.MacAddressSpoofing -ne 'Off') {
            $problems.Add("$($adapter.Name) allows MAC address spoofing")
        }
        foreach ($network in $blockedNetworks) {
            foreach ($direction in 'Inbound', 'Outbound') {
                $denied = @($rules | Where-Object { $_.Direction -eq $direction -and $_.Action -eq 'Deny' -and (Test-AclRemoteAddress -Rule $_ -Address $network) })
                if ($denied.Count -eq 0) {
                    $problems.Add("$($adapter.Name) does not deny $($direction.ToLower()) traffic with $network")
                }
            }
        }
        # A narrower Allow outranks a required deny. Accept only the explicit
        # baseline: the gateway host (guarded above) and IPv4's default route.
        # Private-server exceptions need a separately reviewed policy change.
        foreach ($rule in @($rules | Where-Object { $_.Action -eq 'Allow' })) {
            $isGateway = $gateway -and (Test-AclRemoteAddress -Rule $rule -Address $gateway)
            $isInternet = Test-AclRemoteAddress -Rule $rule -Address '0.0.0.0/0'
            if (-not $isGateway -and -not $isInternet) {
                $problems.Add("$($adapter.Name) has an unexpected allow for $($rule.RemoteAddress), which can bypass its network denies")
            }
        }
    }
    return $problems.ToArray()
}

Export-ModuleMember -Function @(
    'Add-HostGuard'
    'Connect-GitHub'
    'Enter-ControllerLock'
    'Format-RunnerLog'
    'Get-BlockedNetworks'
    'Get-DefaultConfigPath'
    'Get-QueuedRunnerJobs'
    'Get-RepositoryRunners'
    'Get-Runner'
    'Get-RunnerMachineProblems'
    'Get-RunnerSettingsFolder'
    'Get-SwitchGateway'
    'New-JitRunner'
    'New-RunnerName'
    'Read-RunnerConfig'
    'Remove-Runner'
    'Save-GitHubToken'
    'Select-NeededCycles'
    'Stop-RunnerMachine'
    'Test-AclRemoteAddress'
    'Test-LabelsCover'
    'Test-RepositoryAccess'
    'Test-RunnerOfMachine'
)
