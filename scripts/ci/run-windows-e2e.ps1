# WebView2 automation needs the normal desktop integrity level. GitHub's
# Windows jobs start elevated, so only the end-to-end process drops privileges.
param(
    [ValidateSet('suite', 'performance')]
    [string]$Mode = 'suite'
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$folder = Join-Path $env:RUNNER_TEMP ('mannie-e2e-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $folder | Out-Null
$child = Join-Path $folder 'run.ps1'
$log = Join-Path $folder 'run.log'
$result = Join-Path $folder 'exit.txt'

# Checkout files can grant write access through Administrators alone. The
# debug app exports IPC bindings at startup, so the same account needs its
# own inherited grant after its administrator group becomes deny-only.
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
foreach ($path in @($root, $folder)) {
    & icacls $path /grant ('*' + $identity.User.Value + ':(OI)(CI)M') /Q
    if ($LASTEXITCODE -ne 0) {
        throw "Could not grant the test account access to $path"
    }
}
$env:MANNIE_E2E_ROOT = $root
$env:MANNIE_E2E_LOG = $log
$env:MANNIE_E2E_RESULT = $result
$env:MANNIE_E2E_MODE = $Mode
@'
$ErrorActionPreference = 'Stop'
$code = 1
try {
    Set-Location -LiteralPath $env:MANNIE_E2E_ROOT
    & {
        $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
        if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'The end-to-end process still has administrator rights'
        }
        whoami /groups
        if ($env:MANNIE_E2E_MODE -eq 'suite') {
            task e2e:run
        } else {
            pnpm exec wdio run tests/e2e/wdio.perf.conf.ts
        }
        $script:code = $LASTEXITCODE
    } *> $env:MANNIE_E2E_LOG
} catch {
    $_ | Out-String | Add-Content -LiteralPath $env:MANNIE_E2E_LOG
} finally {
    Set-Content -LiteralPath $env:MANNIE_E2E_RESULT -Value $code
}
'@ | Set-Content -LiteralPath $child

Add-Type -TypeDefinition (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'windows-user-token.cs') -Raw)
$powerShell = (Get-Command pwsh).Source
$command = '"' + $powerShell + '" -NoLogo -NoProfile -File "' + $child + '"'
$childId = [MannieCi.StandardUser]::Start($powerShell, $command, $root)
Write-Output "Started the end-to-end process at medium integrity (PID $childId)."
$deadline = [DateTime]::UtcNow.AddMinutes(55)
$reader = $null
try {
    while ($true) {
        if ($null -eq $reader -and (Test-Path -LiteralPath $log)) {
            $stream = [IO.File]::Open($log, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
            $reader = [IO.StreamReader]::new($stream)
        }
        if ($null -ne $reader) {
            while ($null -ne ($line = $reader.ReadLine())) { Write-Output $line }
        }
        if ($null -eq (Get-Process -Id $childId -ErrorAction SilentlyContinue)) { break }
        if ([DateTime]::UtcNow -gt $deadline) {
            & taskkill /PID $childId /T /F
            throw 'The end-to-end process exceeded its time limit'
        }
        Start-Sleep -Milliseconds 250
    }
    if ($null -ne $reader) {
        while ($null -ne ($line = $reader.ReadLine())) { Write-Output $line }
    }
} finally {
    if ($null -ne $reader) { $reader.Dispose() }
}
if (-not (Test-Path -LiteralPath $result)) {
    throw 'The end-to-end process exited without recording its result'
}
exit [int](Get-Content -LiteralPath $result)
