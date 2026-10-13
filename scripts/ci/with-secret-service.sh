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
