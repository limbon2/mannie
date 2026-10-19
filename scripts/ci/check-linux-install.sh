#!/bin/sh
# Checks an installed desktop package and the static programs of the
# plugins in a clean container (release.yml, install check): the desktop app
# starts and keeps running for 15 seconds, and each program, such as the
# worker, reports the release's version. The package is installed before
# this script runs. VERSION is the release version.
set -eu

fail() {
	echo "check-linux-install: $*" >&2
	exit 1
}

[ -x /usr/bin/mannie-desktop ] || fail "the package did not install /usr/bin/mannie-desktop"

# The container cannot create the user namespaces of WebKitGTK's sandbox. The
# check runs as root in a throwaway container with no untrusted content.
export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
status=0
timeout 15 dbus-run-session -- xvfb-run --auto-servernum /usr/bin/mannie-desktop || status=$?
# timeout exits with 124 when the app was still running after 15 seconds.
[ "$status" -eq 124 ] || fail "the desktop app exited with status $status within 15 seconds"

# Every program answers `--version` with its package's name and the
# release's version.
for archive in dist/*-"$VERSION"-linux-x86_64.tar.gz; do
	name=$(basename "$archive" "-$VERSION-linux-x86_64.tar.gz")
	mkdir -p "programs/$name"
	tar -xzf "$archive" -C "programs/$name"
	reported=$("programs/$name/$name" --version)
	[ "$reported" = "$name $VERSION" ] || fail "$name reports $reported"
done
echo "check-linux-install: the desktop app started and the programs of $VERSION run"
