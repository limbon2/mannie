# Gives a runner machine the internet and nothing else on the owner's network
# (docs/engineering/ci-runners.md, section 6): port ACLs on its network
# adapters, which the virtual switch enforces whatever the guest does, with an
# allow rule for its gateway, extended port ACLs that keep it away from the
# host behind that gateway, and guards that drop DHCP offers and router
# announcements from the guest. Run it as an administrator after connecting
# the machine to the runners' internal switch, and before taking the machine's
# clean checkpoint:
#   ./scripts/ci/runners/host/isolate-runner-machine.ps1 -Machine mannie-ci-ubuntu-24.04
# The validated baseline allows only IPv4's default route and the gateway.
# Additional server exceptions require a reviewed address and port policy.
param(
    [Parameter(Mandatory)]
    [string] $Machine
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'runner-host.psm1') -Force

$adapters = @(Get-VMNetworkAdapter -VMName $Machine)
if ($adapters.Count -eq 0) {
    throw "isolate-runner-machine: $Machine has no network adapter"
}
foreach ($adapter in $adapters) {
    if (-not $adapter.SwitchName) {
        throw "isolate-runner-machine: $($adapter.Name) of $Machine is not connected to a switch"
    }
    Set-VMNetworkAdapter -VMNetworkAdapter $adapter -DhcpGuard On -RouterGuard On -MacAddressSpoofing Off
    $rules = @(Get-VMNetworkAdapterAcl -VMNetworkAdapter $adapter)
    # The deny rules also drop ARP for the addresses they cover (observed on
    # 2026-10-17), so the guest's gateway, the host's address on the switch, is
    # allowed. The host's firewall refuses the guests' connections to it
    # (docs/engineering/ci-runners.md, section 6).
    $gateway = Get-SwitchGateway -SwitchName $adapter.SwitchName
    foreach ($direction in 'Inbound', 'Outbound') {
        $allowed = @($rules | Where-Object { $_.Direction -eq $direction -and $_.Action -eq 'Allow' -and (Test-AclRemoteAddress -Rule $_ -Address $gateway) })
        if ($allowed.Count -eq 0) {
            Add-VMNetworkAdapterAcl -VMNetworkAdapter $adapter -RemoteIPAddress $gateway -Direction $direction -Action Allow
        }
    }
    # The rest of IPv4 goes through the NAT to the internet.
    $internet = @($rules | Where-Object { $_.RemoteAddress -eq '0.0.0.0/0' -and $_.Action -eq 'Allow' })
    if ($internet.Count -eq 0) {
        Add-VMNetworkAdapterAcl -VMNetworkAdapter $adapter -RemoteIPAddress '0.0.0.0/0' -Direction Both -Action Allow
    }
    foreach ($network in Get-BlockedNetworks) {
        foreach ($direction in 'Inbound', 'Outbound') {
            $denied = @($rules | Where-Object { $_.RemoteAddress -eq $network -and $_.Direction -eq $direction -and $_.Action -eq 'Deny' })
            if ($denied.Count -eq 0) {
                Add-VMNetworkAdapterAcl -VMNetworkAdapter $adapter -RemoteIPAddress $network -Direction $direction -Action Deny
            }
        }
    }
    # The gateway that the rules above allow is the host, whose services would
    # answer jobs whenever its firewall is off (observed on 2026-10-19).
    Add-HostGuard -VMNetworkAdapter $adapter -Gateway $gateway
}

Get-VMNetworkAdapterAcl -VMName $Machine | Sort-Object -Property Direction, RemoteAddress | Format-Table -Property Direction, RemoteAddress, Action -AutoSize
Get-VMNetworkAdapterExtendedAcl -VMName $Machine | Sort-Object -Property Weight, Direction | Format-Table -Property Direction, Action, RemoteIPAddress, Protocol, LocalPort, Stateful, Weight -AutoSize
$problems = @(Get-RunnerMachineProblems -Machine $Machine)
if ($problems.Count -gt 0) {
    Write-Output "isolate-runner-machine: before you take the clean checkpoint of $Machine, fix these:"
    foreach ($problem in $problems) {
        Write-Output "  - $problem"
    }
} else {
    Write-Output "isolate-runner-machine: $Machine reaches the internet and nothing else; it is ready for its clean checkpoint"
}
