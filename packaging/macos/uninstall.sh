#!/bin/sh
# Removes only packaged commands/resources. Never touches deployment identities or state.
set -eu
if [ "$(id -u)" != 0 ]; then echo 'Run with sudo to remove the installed package.' >&2; exit 1; fi
for name in loramesh-mesh loramesh-keygen loramesh-radio loramesh-sim loramesh-mesh-sim; do
  rm -f "/usr/local/bin/$name"
done
rm -rf /usr/local/share/loramesh
pkgutil --forget org.loramesh.tools >/dev/null 2>&1 || true
