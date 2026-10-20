#!/bin/sh
# Prepares an Ubuntu 24.04 or 22.04 runner image (docs/engineering/
# ci-runners.md, section 6): a root file system over the whole disk, name
# lookups over TLS, the packages that the jobs expect, Docker, the Hyper-V
# daemon that receives files from the host, the user runner with passwordless
# sudo and rustup, the newest runner application without a registration, and
# the unit that runs it for one job at every boot. Run it as root in the
# guest, from the folder that also holds start-runner.sh and
# mannie-runner.service:
#   sudo sh setup-runner-image.sh
set -eu

user=runner
home=/home/$user
runner_folder=$home/actions-runner
# The host's run-runner-cycle.ps1 copies the runner configuration into this
# folder.
inbox=/var/lib/mannie-runner
# Docker's default bridge, pinned so that containers find the name server that
# listens on its address.
docker_bridge=172.17.0.1
here=$(cd "$(dirname "$0")" && pwd)

fail() {
	echo "setup-runner-image: $*" >&2
	exit 1
}

[ "$(id -u)" -eq 0 ] || fail "run it as root, with sudo"
. /etc/os-release
case "$VERSION_ID" in
24.04 | 22.04) ;;
*) fail "this is Ubuntu $VERSION_ID; the runner images run Ubuntu 24.04 or 22.04" ;;
esac
[ -d /run/systemd/system ] || fail "systemd is not the init system"

# The installer's LVM layout gives the root volume only part of the disk, half
# of a 64 GB disk (observed on 2026-10-19), and the builds need all of it. The
# volume grows over the free space of its volume group, and its file system
# with it, while it is mounted.
root_device=$(findmnt -n -o SOURCE /)
root_volume=$(lvs --noheadings --separator : -o lv_dm_path,vg_name,lv_name 2>/dev/null |
	tr -d ' ' | awk -F: -v device="$root_device" '$1 == device { print $2 "/" $3 }')
if [ -n "$root_volume" ]; then
	free_extents=$(vgs --noheadings -o vg_free_count "${root_volume%/*}" | tr -d ' ')
	if [ "$free_extents" -gt 0 ]; then
		lvextend --resizefs --extents +100%FREE "$root_volume"
		echo "setup-runner-image: the root volume $root_volume now fills its volume group"
	fi
fi

# Name lookups go over TLS, which is TCP: the host's route to the internet may
# lose UDP, as the owner's lost 40 to 60 percent of plain lookups while every
# lookup over TCP passed (observed on 2026-10-19). Containers, which cannot
# reach the stub on 127.0.0.53, get one on Docker's bridge address.
install -d -m 0755 /etc/systemd/resolved.conf.d
cat >/etc/systemd/resolved.conf.d/mannie-runner.conf <<EOF
[Resolve]
DNS=1.1.1.1#cloudflare-dns.com 9.9.9.9#dns.quad9.net
DNSOverTLS=yes
DNSStubListenerExtra=$docker_bridge
EOF
systemctl restart systemd-resolved.service

# The jobs run as an account of their own, apart from the owner's login. It
# has no password: it needs none for sudo below, and nobody signs in as it.
if ! id "$user" >/dev/null 2>&1; then
	useradd --create-home --shell /bin/bash "$user"
fi

export DEBIAN_FRONTEND=noninteractive
apt-get update
# The jobs install the rest, as on GitHub's runners. The cloud tools hold
# hv_fcopy_daemon, which writes the files that the host copies into the guest,
# for the kernel of linux-generic.
apt-get install -y --no-install-recommends \
	build-essential pkg-config curl ca-certificates git python3 dbus \
	docker.io linux-cloud-tools-generic

# Each boot starts from the checkpoint, so the daily package timers would find
# their last run long past and start at once, holding the package lock that
# the jobs need. The owner updates the image deliberately instead.
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer

install -d -m 0755 /etc/docker
cat >/etc/docker/daemon.json <<EOF
{
  "bip": "$docker_bridge/16",
  "dns": ["$docker_bridge"]
}
EOF
systemctl enable docker.service
systemctl restart docker.service
usermod -aG docker "$user"

# The jobs install packages and services, as on GitHub's runners.
sudoers=/etc/sudoers.d/mannie-runner
printf '%s ALL=(ALL:ALL) NOPASSWD: ALL\n' "$user" >"$sudoers"
chmod 0440 "$sudoers"
visudo -cf "$sudoers" >/dev/null || fail "visudo rejects $sudoers"

# rustup without a toolchain: each job installs the one that
# rust-toolchain.toml pins.
if [ ! -x "$home/.cargo/bin/rustup" ]; then
	runuser -u "$user" -- env HOME="$home" sh -c \
		'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none'
fi
# A failed download leaves the installer's shell with nothing to run.
[ -x "$home/.cargo/bin/rustup" ] || fail "rustup did not install for $user"

# The newest runner application, unregistered: each cycle registers it for one
# job. GitHub publishes the SHA-256 digest of every release asset (verified on
# 2026-10-16).
[ ! -e "$runner_folder/.runner" ] || fail "the runner in $runner_folder is registered; an image holds an unregistered runner"
release=$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest)
asset=$(printf '%s' "$release" | python3 -c '
import json
import sys

release = json.load(sys.stdin)
version = release["tag_name"].lstrip("v")
name = "actions-runner-linux-x64-" + version + ".tar.gz"
matches = [item for item in release["assets"] if item["name"] == name]
if not matches:
    sys.exit("setup-runner-image: the runner release " + version + " has no " + name)
print(version, matches[0]["browser_download_url"], matches[0]["digest"].split(":", 1)[1])
')
read -r version url digest <<EOF
$asset
EOF
archive=$(mktemp)
curl -fsSL -o "$archive" "$url"
echo "$digest  $archive" | sha256sum -c --status - || fail "the runner $version does not match the digest that GitHub publishes"
rm -rf "$runner_folder"
mkdir "$runner_folder"
tar -xzf "$archive" -C "$runner_folder"
chown -R "$user:$user" "$runner_folder"
rm -f "$archive"
# The runner's own script installs the libraries that .NET needs.
"$runner_folder/bin/installdependencies.sh"
echo "setup-runner-image: installed the runner $version in $runner_folder"

install -d -m 0755 /usr/local/libexec/mannie-runner
install -m 0755 "$here/start-runner.sh" /usr/local/libexec/mannie-runner/start-runner.sh
install -m 0644 "$here/mannie-runner.service" /etc/systemd/system/mannie-runner.service
# hv_fcopy_daemon writes files that every user may read, so only the runner's
# user may open the folder that holds them.
install -d -o "$user" -g "$user" -m 0700 "$inbox"
find "$inbox" -mindepth 1 -delete
systemctl daemon-reload
systemctl enable mannie-runner.service

# The daemon runs once the host's Guest services reach the guest.
systemctl start hv-fcopy-daemon.service || true
systemctl is-active --quiet hv-fcopy-daemon.service ||
	fail "hv-fcopy-daemon is not running: turn on Guest services for this machine on the host, boot the kernel of linux-generic, restart once after installing linux-cloud-tools-generic, then run this script again"
echo "setup-runner-image: done; power off with 'sudo poweroff' and take the clean checkpoint on the host"
