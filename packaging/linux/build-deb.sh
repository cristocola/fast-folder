#!/usr/bin/env bash
# Build a Debian package from an unpacked release staging directory.
#
# Usage: build-deb.sh <staging-dir> <version> <output.deb>
#
# The staging directory is the one `release.yml` assembles for the tarball, so
# the package ships exactly what the archive ships: the binary, the licence,
# the completions for three shells, the man pages, the desktop entry and the
# icons. The binary is the static musl build, which is why the package declares
# no dependencies and installs on any Debian derivative old or new.
#
# `dpkg-deb --root-owner-group` writes every path as root:root, so this needs
# neither fakeroot nor a privileged runner.
set -euo pipefail

staging="${1:?usage: build-deb.sh <staging-dir> <version> <output.deb>}"
version="${2:?usage: build-deb.sh <staging-dir> <version> <output.deb>}"
out="${3:?usage: build-deb.sh <staging-dir> <version> <output.deb>}"

# Debian versions carry no leading v, and they must begin with a digit.
# `workflow_dispatch` dry runs pass `dev-<sha>`, which dpkg-deb refuses, so it
# becomes a version that sorts below every real one. The same reason the MSI
# uses 0.0.0 for a dry run.
debver="${version#v}"
case "$debver" in
  [0-9]*) ;;
  *) debver="0.0.0~$(printf '%s' "$debver" | tr '-' '.')" ;;
esac

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT

install -Dm755 "$staging/fastf" "$root/usr/bin/fastf"
install -Dm644 "$staging/LICENSE" "$root/usr/share/doc/fast-folder/copyright"
install -Dm644 "$staging/README.md" "$root/usr/share/doc/fast-folder/README.md"

install -Dm644 "$staging/completions/fastf.bash" \
  "$root/usr/share/bash-completion/completions/fastf"
install -Dm644 "$staging/completions/fastf.zsh" \
  "$root/usr/share/zsh/vendor-completions/_fastf"
install -Dm644 "$staging/completions/fastf.fish" \
  "$root/usr/share/fish/vendor_completions.d/fastf.fish"

# Debian policy wants man pages compressed, and `gzip -n` leaves the timestamp
# out so two builds of the same source produce the same bytes.
mkdir -p "$root/usr/share/man/man1"
for page in "$staging"/man/*.1; do
  gzip -9nc "$page" > "$root/usr/share/man/man1/$(basename "$page").gz"
done

install -Dm644 "$staging/fastf.desktop" "$root/usr/share/applications/fastf.desktop"
for size in 48 128 256; do
  install -Dm644 "$staging/icons/fastf-$size.png" \
    "$root/usr/share/icons/hicolor/${size}x${size}/apps/fastf.png"
done

mkdir -p "$root/DEBIAN"
installed_size="$(du -ks "$root/usr" | cut -f1)"
cat > "$root/DEBIAN/control" <<EOF
Package: fast-folder
Version: $debver
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Cristo Cola <kristokola@hotmail.com>
Homepage: https://github.com/cristocola/fast-folder
Installed-Size: $installed_size
Description: Project folder creator and manager with a terminal app and a CLI
 fast-folder creates project folders from templates and keeps them findable
 afterwards. You describe a folder structure once as a template; every project
 made from it gets a consistent name, the subfolders you always need, starter
 files with your answers written into them, a unique ID, and metadata that
 makes it searchable months later.
 .
 The command is fastf. Running it with no arguments opens a full screen
 terminal app over the whole library; every action it offers also has a
 command, so the same work fits into a script or a cron job.
 .
 This package carries the statically linked build, so it installs on any
 Debian derivative and depends on nothing at run time.
EOF

# lintian and `dpkg --verify` both read this; it costs one pass over the tree.
(cd "$root" && find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums)

dpkg-deb --build --root-owner-group "$root" "$out"
dpkg-deb --info "$out"
dpkg-deb --contents "$out"
