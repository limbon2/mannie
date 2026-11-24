#!/bin/sh
# Starts a Secret Service in the current D-Bus session, then runs the given
# command (ci.yml, credential store, V7). Run it inside `dbus-run-session`:
#
#   dbus-run-session -- sh scripts/ci/with-secret-service.sh gnome-keyring cargo test ...
set -eu

provider=$1
shift

case "$provider" in
gnome-keyring)
	# The password on standard input creates the login keyring, which becomes
	# the default collection, and unlocks it. The test machine is disposable.
	printf 'mannie-ci\n' | gnome-keyring-daemon --unlock --components=secrets >/dev/null
	if command -v dbus-send >/dev/null 2>&1; then
		waited=0
		until dbus-send --session --dest=org.freedesktop.DBus --type=method_call --print-reply \
			/org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner string:org.freedesktop.secrets 2>/dev/null |
			grep -q 'boolean true'; do
			waited=$((waited + 1))
			if [ "$waited" -gt 50 ]; then
				echo "with-secret-service: org.freedesktop.secrets did not register on D-Bus" >&2
				exit 1
			fi
			sleep 0.2
		done
	fi
	;;
kwallet)
	# Opens the default wallet the way a KDE login does (kwallet-login.py).
	python3 "$(dirname "$0")/kwallet-login.py"
	;;
*)
	echo "with-secret-service: unknown Secret Service $provider" >&2
	exit 2
	;;
esac

exec "$@"
