#!/bin/sh
# Installs the worker as a service with the system's folders, checks that it
# runs under its account with a configuration that only root can change and
# a private data folder, pairs it with a test master that delegates a
# command, and removes it again (ci.yml on Linux with systemd, nightly.yml on
# FreeBSD). Runs as root on a disposable machine:
#   sh scripts/ci/worker-service-unix.sh <mannie-worker> <smoke-master>
# The test master is `cargo build -p mannie-link --features testing --example
# smoke-master`.
set -eu

worker=$1
master=$2
account=mannie-ci
system=$(uname -s)
if [ "$system" = FreeBSD ]; then
	config=/usr/local/etc/mannie-worker
	data=/var/db/mannie-worker
else
	config=/etc/mannie-worker
	data=/var/lib/mannie-worker
fi
program=/usr/local/libexec/mannie-worker/mannie-worker

fail() {
	echo "worker-service: $*" >&2
	exit 1
}

owner_and_mode() {
	if [ "$system" = FreeBSD ]; then
		stat -f '%Su %Lp' "$1"
	else
		stat -c '%U %a' "$1"
	fi
}

# A system account without a login, as the guide tells the owner to create.
if [ "$system" = FreeBSD ]; then
	pw useradd "$account" -d /nonexistent -s /usr/sbin/nologin -c "Mannie worker CI"
else
	useradd --system --no-create-home --shell /usr/sbin/nologin "$account"
fi

"$worker" service install --user "$account"

waited=0
until [ -f "$data/mannie-worker.sqlite" ]; do
	waited=$((waited + 1))
	[ "$waited" -le 60 ] || fail "the worker did not create its database in $data"
	sleep 1
done
until grep -q "the worker started" "$data"/logs/*; do
	waited=$((waited + 1))
	[ "$waited" -le 60 ] || fail "the worker did not log that it started"
	sleep 1
done

if [ "$system" = FreeBSD ]; then
	service mannie_worker status || fail "the rc.d service is not running"
else
	systemctl is-active --quiet mannie-worker.service || fail "the systemd unit is not active"
fi

# The guide's next step: an allowed folder, which is the tasks' workspace,
# then a restart, which reads the configuration again.
work=/var/mannie-ci-work
install -d -o "$account" -m 0750 "$work"
sed -i.bak "s|^allowed_folders = \[\]|allowed_folders = [\"$work\"]|" "$config/worker.toml"
rm "$config/worker.toml.bak"
grep -q "^allowed_folders = \[\"$work\"\]" "$config/worker.toml" || fail "could not allow $work in the configuration"
if [ "$system" = FreeBSD ]; then
	service mannie_worker restart
else
	systemctl restart mannie-worker.service
fi
waited=0
until [ -f "$data/health" ]; do
	waited=$((waited + 1))
	[ "$waited" -le 60 ] || fail "the worker did not start again with the allowed folder"
	sleep 1
done

# The worker started as its account, which it refuses when that account can
# change the configuration; the owners and modes say the same.
[ "$(owner_and_mode "$data")" = "$account 700" ] || fail "the data folder is $(owner_and_mode "$data"), not $account 700"
[ "$(owner_and_mode "$config")" = "root 755" ] || fail "the configuration's folder is $(owner_and_mode "$config"), not root 755"
[ "$(owner_and_mode "$config/worker.toml")" = "root 644" ] || fail "the configuration is $(owner_and_mode "$config/worker.toml"), not root 644"
[ "$(owner_and_mode "$program")" = "root 755" ] || fail "the program is $(owner_and_mode "$program"), not root 755"
[ -L /usr/local/bin/mannie-worker ] || fail "/usr/local/bin/mannie-worker is not a link to the program"

code=$(mannie-worker pair | sed -n 's/^  Code: *//p')
[ -n "$code" ] || fail "pair printed no code"
[ "$(owner_and_mode "$data/pairing-code")" = "$account 600" ] || fail "the pairing code is $(owner_and_mode "$data/pairing-code"), not $account 600"
"$master" 127.0.0.1:7445 "$code" || fail "the test master could not pair and delegate a command"
mannie-worker status | grep -q "^Pairing: not paired" || fail "the worker is still paired after the master removed it"

mannie-worker service uninstall

if [ "$system" = FreeBSD ]; then
	[ ! -e /usr/local/etc/rc.d/mannie_worker ] || fail "the rc.d script is still there"
else
	[ ! -e /etc/systemd/system/mannie-worker.service ] || fail "the systemd unit is still there"
fi
[ -f "$data/mannie-worker.sqlite" ] || fail "uninstall removed the data folder"
[ -f "$config/worker.toml" ] || fail "uninstall removed the configuration"
grep -q "the worker stopped" "$data"/logs/* || fail "the worker did not stop cleanly"
echo "worker-service: installed, ran as $account with a configuration only root can change, ran a delegated command, stopped cleanly and was removed"
