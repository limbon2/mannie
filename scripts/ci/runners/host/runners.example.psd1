# The runner machines of the host scripts (docs/engineering/ci-runners.md,
# section 6). Copy this file to %LOCALAPPDATA%\Mannie\ci-runners\runners.psd1
# and keep the names in step with the Hyper-V machines.
@{
    # The repository whose jobs the runners take.
    Repository = 'limbon2/mannie'

    # The checkpoint of each machine that every cycle restores.
    Checkpoint = 'clean'

    # How many machines may run at once. Each one needs its memory free on the
    # host.
    MaxRunning = 1

    # A just-in-time runner gets exactly the labels that it is registered with,
    # so each machine lists all of them.
    Machines   = @(
        @{
            Name   = 'mannie-ci-windows-11'
            Guest  = 'Windows'
            Labels = @('self-hosted', 'disposable', 'mannie-windows-11')
        }
        @{
            Name   = 'mannie-ci-ubuntu-24.04'
            Guest  = 'Ubuntu'
            Labels = @('self-hosted', 'disposable', 'mannie-ubuntu-24.04')
        }
        @{
            Name   = 'mannie-ci-ubuntu-22.04'
            Guest  = 'Ubuntu'
            Labels = @('self-hosted', 'disposable', 'mannie-ubuntu-22.04')
        }
    )
}
