#!/bin/sh
# Install fast-folder into your home directory on any Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/cristocola/fast-folder/main/packaging/linux/install.sh | sh
#
# It downloads the statically linked release archive, checks it against the
# release's own SHA256SUMS, and unpacks it under ~/.local: the binary, the man
# pages, the shell completions, the desktop entry and the icons. Everything
# lands in your home directory, so it asks for no privileges and `rm` undoes
# it. Set FASTF_VERSION to pin a release and PREFIX to install somewhere else.
#
# On Debian and Ubuntu the .deb from the releases page is the better route:
# apt then owns the files and `apt remove fast-folder` takes them away again.
set -eu

repo="cristocola/fast-folder"
prefix="${PREFIX:-$HOME/.local}"
target="x86_64-unknown-linux-musl"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Linux" ] || die "this installer is for Linux; see the README for macOS and Windows"
case "$(uname -m)" in
  x86_64|amd64) ;;
  *) die "the published Linux binaries are x86_64; on $(uname -m) build from source with cargo" ;;
esac

for tool in curl tar sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
done

version="${FASTF_VERSION:-}"
if [ -z "$version" ]; then
  say "Looking up the latest release..."
  version=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$version" ] || die "could not read the latest release tag; set FASTF_VERSION=vX.Y.Z"
fi

archive="fastf-$version-$target.tar.gz"
base="https://github.com/$repo/releases/download/$version"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
cd "$tmp"

say "Downloading $archive ($version)..."
curl -fsSL -o "$archive" "$base/$archive" || die "could not download $archive"
curl -fsSL -o SHA256SUMS "$base/SHA256SUMS" || die "could not download SHA256SUMS"

say "Checking the download against SHA256SUMS..."
grep " $archive\$" SHA256SUMS > expected.txt || die "$archive is missing from SHA256SUMS"
sha256sum -c expected.txt >/dev/null || die "checksum mismatch; the download was corrupted or tampered with"

tar xzf "$archive"
cd "fastf-$version-$target"

say "Installing into $prefix..."
install -Dm755 fastf "$prefix/bin/fastf"
install -Dm644 LICENSE "$prefix/share/doc/fast-folder/copyright"
install -Dm644 completions/fastf.bash "$prefix/share/bash-completion/completions/fastf"
install -Dm644 completions/fastf.zsh "$prefix/share/zsh/site-functions/_fastf"
install -Dm644 completions/fastf.fish "$prefix/share/fish/vendor_completions.d/fastf.fish"
for page in man/*.1; do
  install -Dm644 "$page" "$prefix/share/man/man1/$(basename "$page")"
done
install -Dm644 fastf.desktop "$prefix/share/applications/fastf.desktop"
for size in 48 128 256; do
  install -Dm644 "icons/fastf-$size.png" \
    "$prefix/share/icons/hicolor/${size}x${size}/apps/fastf.png"
done

say ""
say "Installed $("$prefix/bin/fastf" --version) at $prefix/bin/fastf"

case ":$PATH:" in
  *":$prefix/bin:"*)
    say "Run fastf to start."
    ;;
  *)
    say ""
    say "Add $prefix/bin to your PATH, then run fastf:"
    say "  echo 'export PATH=\"$prefix/bin:\$PATH\"' >> ~/.bashrc && exec \$SHELL"
    ;;
esac
