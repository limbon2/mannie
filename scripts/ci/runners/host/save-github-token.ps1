# Stores the GitHub token of the runner scripts in %LOCALAPPDATA%\Mannie\
# ci-runners, encrypted with DPAPI for the current Windows user
# (docs/engineering/ci-runners.md, section 6). It asks for the token, so the
# token is on no command line and in no history. Run it as the Windows user
# that runs run-runners.ps1:
#   ./scripts/ci/runners/host/save-github-token.ps1
param(
    [string] $Repository = 'limbon2/mannie'
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'runner-host.psm1') -Force

$token = Read-Host -Prompt "Fine-grained token for $Repository (Administration: read and write, Actions: read)" -AsSecureString
if ($token.Length -eq 0) {
    throw 'save-github-token: no token was entered'
}
# The token is saved only once it can read the runners and the workflow runs.
# Creating a runner would also prove that it may write, but would register one.
Connect-GitHub -Token $token
Test-RepositoryAccess -Repository $Repository
Save-GitHubToken -Token $token
Write-Output "save-github-token: saved the token for $env:USERDOMAIN\$env:USERNAME; it reads the runners and the workflow runs of $Repository"
