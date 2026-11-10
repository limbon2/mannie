#!/bin/sh
# Checks an installed desktop package and the static programs of the
# plugins in a clean container (release.yml, install check): the desktop app
# starts, keeps running for 15 seconds and reports the kind of the package
# that installed it, and each program, such as the worker, reports the
# release's version. The package is installed before this script runs. The
# argument is the kind of that package, deb or rpm, and VERSION is the
# release version.
set -eu

fail() {
	echo "check-linux-install: $*" >&2
	exit 1
}

package=${1:-}
case "$package" in
deb | rpm) ;;
*) fail "name the kind of the installed package, deb or rpm" ;;
esac

[ -x /usr/bin/mannie-desktop ] || fail "the package did not install /usr/bin/mannie-desktop"

# The container cannot create the user namespaces of WebKitGTK's sandbox. The
# check runs as root in a throwaway container with no untrusted content.
export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
status=0
timeout 15 dbus-run-session -- xvfb-run --auto-servernum /usr/bin/mannie-desktop || status=$?
# timeout exits with 124 when the app was still running after 15 seconds.
[ "$status" -eq 124 ] || fail "the desktop app exited with status $status within 15 seconds"

# The bundler marks the app's binary with the kind of its package, and an
# installed app chooses its package of a later release by that mark, so a
# build without the mark would never update itself (ADR-0063, decision 5).
# The app writes the mark into the start line of its log.
logs="${XDG_DATA_HOME:-$HOME/.local/share}/io.github.limbon2.mannie/logs"
start=$(cat "$logs"/mannie.*.log 2>/dev/null | grep -F '"Mannie is starting"' | tail -n 1)
[ -n "$start" ] || fail "the log in $logs has no start line of the desktop app"
installation=$(printf '%s\n' "$start" | sed -n 's/.*"installation":"\([^"]*\)".*/\1/p')
[ "$installation" = "$package" ] || fail "the desktop app reports the installation \"$installation\", and a $package package installed it"

# Every program answers `--version` with its package's name and the
# release's version.
for archive in dist/*-"$VERSION"-linux-x86_64.tar.gz; do
	name=$(basename "$archive" "-$VERSION-linux-x86_64.tar.gz")
	mkdir -p "programs/$name"
	tar -xzf "$archive" -C "programs/$name"
	reported=$("programs/$name/$name" --version)
	[ "$reported" = "$name $VERSION" ] || fail "$name reports $reported"
done
echo "check-linux-install: the desktop app of the $package package started and the programs of $VERSION run"
