#!/usr/bin/env bash
# Bump both AUR PKGBUILDs to a new released version and regenerate .SRCINFO.
#
# Usage: ./update.sh 1.0.0
#
# Run AFTER the GitHub release for v<version> exists (updpkgsums downloads
# the tarballs to compute checksums). Then copy PKGBUILD + .SRCINFO from each
# package dir into its AUR clone and push (see PUBLISHING.md).

set -euo pipefail

version="${1:?usage: ./update.sh <version, e.g. 1.0.0>}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

for pkg in fast-folder fast-folder-bin; do
    dir="$here/$pkg"
    echo "==> $pkg $version"
    sed -i "s/^pkgver=.*/pkgver=$version/" "$dir/PKGBUILD"
    sed -i "s/^pkgrel=.*/pkgrel=1/" "$dir/PKGBUILD"
    (cd "$dir" && updpkgsums && makepkg --printsrcinfo > .SRCINFO)
    echo "    PKGBUILD + .SRCINFO updated"
done

cat <<EOF

Done. Next steps (per package). The AUR clones live beside this repo:
  aur=~/Projects/2026-05-13_fast_folder_ID0052/aur
  cp $here/fast-folder/{PKGBUILD,.SRCINFO}     \$aur/fast-folder/
  cp $here/fast-folder-bin/{PKGBUILD,.SRCINFO} \$aur/fast-folder-bin/
  cd \$aur/fast-folder     && git add -A && git commit -m "fast-folder $version-1"     && git push
  cd \$aur/fast-folder-bin && git add -A && git commit -m "fast-folder-bin $version-1" && git push
EOF
