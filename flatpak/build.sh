#!/bin/bash
# Build the Flatpak bundle from a built .deb:  flatpak/build.sh path/to/Any\ Two\ Keys_0.1.1_amd64.deb
# Needs flatpak-builder and the GNOME 51 runtime + SDK (flatpak install flathub org.gnome.Platform//51 org.gnome.Sdk//51).
# Leaves flatpak/out/AnyTwoKeys-<version>-linux-x86_64.flatpak; install it with `flatpak install --user <file>`.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
deb=${1:?path to the .deb}
ver=$(dpkg-deb -f "$deb" Version)
cp "$deb" "$here/anytwokeys.deb"
rm -rf "$here/build" "$here/repo"; mkdir -p "$here/out"
flatpak-builder --force-clean --repo="$here/repo" "$here/build" "$here/com.anytwokeys.app.yml"
flatpak build-bundle "$here/repo" "$here/out/AnyTwoKeys-$ver-linux-x86_64.flatpak" com.anytwokeys.app
rm -f "$here/anytwokeys.deb"; rm -rf "$here/build" "$here/repo" "$here/.flatpak-builder"
ls -la "$here/out/AnyTwoKeys-$ver-linux-x86_64.flatpak"
