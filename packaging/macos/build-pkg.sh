#!/bin/sh
# Paquet macOS d'aNkou (ADR 0011) : aNkou.app, ankoud et ankou-cli, démon launchd.
# Usage : packaging/macos/build-pkg.sh <version> <dossier des binaires> <fichier .pkg>
set -eu
version=$1 bin=$2 out=$3
here=$(cd "$(dirname "$0")" && pwd)
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT

app="$root/Applications/aNkou.app/Contents"
mkdir -p "$app/MacOS" "$root/usr/local/bin" "$root/Library/LaunchDaemons"
sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Info.plist"
install -m 755 "$bin/ankou" "$app/MacOS/ankou"
install -m 755 "$bin/ankoud" "$bin/ankou-cli" "$root/usr/local/bin/"
install -m 644 "$here/fr.lpdnsoft.ankoud.plist" "$root/Library/LaunchDaemons/"

# L'application reste dans /Applications, même si une copie existe ailleurs.
components="$root.plist"
pkgbuild --analyze --root "$root" "$components"
plutil -replace 0.BundleIsRelocatable -bool false "$components"

pkgbuild --root "$root" --component-plist "$components" --scripts "$here/scripts" \
  --identifier fr.lpdnsoft.ankou --version "$version" --install-location / "$out"
rm -f "$components"
