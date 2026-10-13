#!/bin/sh
# Installs the worker as a service, checks that it runs under its account and
# opens its store in a private data folder, and removes it again (ci.yml on
# Linux with systemd, nightly.yml on FreeBSD). Runs as root on a disposable
# machine: sh scripts/ci/worker-service-unix.sh <path to mannie-worker>
set -eu

worker=$1
account=mannie-ci
system=$(uname -s)
if [ "$system" = FreeBSD ]; then
	data=/var/db/mannie-worker-ci
else
	data=/var/lib/mannie-worker-ci
fi

fail() {
	echo "worker-service: $*" >&2
	exit 1
}

# A system account without a login, as an owner would create for the worker.
if [ "$system" = FreeBSD ]; then
	pw useradd "$account" -d /nonexistent -s /usr/sbin/nologin -c "Mannie worker CI"
else
	useradd --system --no-create-home --shell /usr/sbin/nologin "$account"
fi
install -m 0755 "$worker" /usr/local/bin/mannie-worker

/usr/local/bin/mannie-worker service install --data-dir "$data" --user "$account"

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
	owner_and_mode=$(stat -f '%Su %Lp' "$data")
else
	systemctl is-active --quiet mannie-worker.service || fail "the systemd unit is not active"
	owner_and_mode=$(stat -c '%U %a' "$data")
fi
[ "$owner_and_mode" = "$account 700" ] || fail "the data folder is $owner_and_mode, not owned by $account with mode 700"

/usr/local/bin/mannie-worker service uninstall

if [ "$system" = FreeBSD ]; then
	[ ! -e /usr/local/etc/rc.d/mannie_worker ] || fail "the rc.d script is still there"
else
	[ ! -e /etc/systemd/system/mannie-worker.service ] || fail "the systemd unit is still there"
fi
[ -f "$data/mannie-worker.sqlite" ] || fail "uninstall removed the data folder"
grep -q "the worker stopped" "$data"/logs/* || fail "the worker did not stop cleanly"
echo "worker-service: installed, ran as $account with a private data folder, stopped cleanly and was removed"
