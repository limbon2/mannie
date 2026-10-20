#!/bin/sh
# Runs the GitHub Actions runner for one job, then powers the machine off
# (docs/engineering/ci-runners.md, section 6). mannie-runner.service starts it
# at every boot as the user runner, who may use sudo without a password, as
# the jobs need. Stopping the unit stops the runner without powering off.
set -eu

# The host's run-runner-cycle.ps1 copies the files into this folder, which only
# the runner's user may open.
inbox=/var/lib/mannie-runner
config=$inbox/jitconfig
ready=$inbox/ready

echo "mannie-runner: waiting for the runner configuration from the host"
# The host copies the marker after the configuration, so the configuration is
# complete once the marker is there.
until [ -f "$ready" ]; do
	sleep 2
done

# The configuration registers this machine as the runner of one job, so it
# leaves the disk before the runner starts. The runner reads it from this
# variable, masks it in its logs and removes it from the environment that the
# job's steps inherit; on the command line any process could read it.
runner_name=$(cat "$ready")
ACTIONS_RUNNER_INPUT_JITCONFIG=$(cat "$config")
export ACTIONS_RUNNER_INPUT_JITCONFIG
rm -f "$config" "$ready"
echo "mannie-runner: received the configuration of the runner $runner_name"

status=0
./run.sh || status=$?
echo "mannie-runner: the runner exited with status $status; powering off"
exec sudo systemctl poweroff
