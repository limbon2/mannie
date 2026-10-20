#!/bin/sh
# Use Ubuntu's package sources for Ubuntu dependencies. Hosted images also
# have third-party repositories whose outages must not block these packages.
set -eu

sources=/etc/apt/sources.list
if [ -f /etc/apt/sources.list.d/ubuntu.sources ]; then
  sources=/etc/apt/sources.list.d/ubuntu.sources
fi

apt-get -o Dir::Etc::sourcelist="$sources" -o Dir::Etc::sourceparts=- -o Acquire::Retries=3 update
exec apt-get -o Dir::Etc::sourcelist="$sources" -o Dir::Etc::sourceparts=- -o Acquire::Retries=3 \
  install -y --no-install-recommends "$@"
