#!/bin/sh
# Install fast-folder into your home directory on any Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/cristocola/fast-folder/main/packaging/linux/install.sh | sh
#
# It downloads the statically linked release archive, checks it against the
# release's own SHA256SUMS, and unpacks the binary, the man pages, the shell
# completions, the desktop entry and the icons. It puts the binary on your PATH
# for you.
#
# As root it installs into /usr/local, which is already on PATH. As anyone else
# it installs into ~/.local and adds ~/.local/bin to your shell profile.
# FASTF_VERSION pins a release, PREFIX chooses where it goes.
set -eu

repo="cristocola/fast-folder"
target="x86_64-unknown-linux-musl"

# root is installing for the machine, so /usr/local, where PATH already looks.
if [ "$(id -u)" = "0" ]; then
  prefix="${PREFIX:-/usr/local}"
else
  prefix="${PREFIX:-$HOME/.local}"
fi

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Linux" ] || die "this installer is for Linux; see the README for macOS and Windows"
case "$(uname -m)" in
  x86_64|amd64) ;;
  *) die "the published Linux binaries are x86_64; on $(uname -m) build from source with cargo" ;;
esac

for tool in tar sha256sum install; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
done

# curl for the people who pasted the one line above, wget for everyone else.
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL -o "$1" "$2"; }
  read_url() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$1" "$2"; }
  read_url() { wget -qO - "$1"; }
else
  die "curl or wget is required"
fi

version="${FASTF_VERSION:-}"
if [ -z "$version" ]; then
  say "Looking up the latest release..."
  version=$(read_url "https://api.github.com/repos/$repo/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$version" ] || die "could not read the latest release tag; set FASTF_VERSION=vX.Y.Z"
fi

archive="fastf-$version-$target.tar.gz"
base="https://github.com/$repo/releases/download/$version"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
cd "$tmp"

say "Downloading $archive ($version)..."
fetch "$archive" "$base/$archive" || die "could not download $archive"
fetch SHA256SUMS "$base/SHA256SUMS" || die "could not download SHA256SUMS"

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

# Desktop environments pick the new entry up sooner when the database knows.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi

# --- PATH ------------------------------------------------------------------
#
# Adding the line to the shell profiles is the job, so that `fastf` works from
# the next shell without anyone being told to go and edit a file. The marker
# makes it idempotent: running the installer again finds its own line and
# leaves the profile alone, and deleting the two lines undoes it.
marker="# added by the fast-folder installer"
line="export PATH=\"$prefix/bin:\$PATH\""
changed=""

add_to_profile() {
  file="$1"
  if [ -e "$file" ] && grep -qF "$marker" "$file" 2>/dev/null; then
    return 0
  fi
  printf '\n%s\n%s\n' "$marker" "$line" >> "$file" || return 0
  changed="$changed $file"
}

on_path=no
case ":$PATH:" in
  *":$prefix/bin:"*) on_path=yes ;;
esac

if [ "$on_path" = "no" ]; then
  # Every profile that exists, and `~/.profile` even when it does not, since
  # that is the one a login shell reads whatever the shell turns out to be.
  [ -e "$HOME/.profile" ] || : > "$HOME/.profile"
  add_to_profile "$HOME/.profile"
  [ -e "$HOME/.bashrc" ] && add_to_profile "$HOME/.bashrc"
  [ -e "$HOME/.bash_profile" ] && add_to_profile "$HOME/.bash_profile"
  [ -e "$HOME/.zshrc" ] && add_to_profile "$HOME/.zshrc"
  if [ -d "$HOME/.config/fish" ] || command -v fish >/dev/null 2>&1; then
    mkdir -p "$HOME/.config/fish/conf.d"
    fish_file="$HOME/.config/fish/conf.d/fast-folder.fish"
    if [ ! -e "$fish_file" ] || ! grep -qF "$marker" "$fish_file" 2>/dev/null; then
      printf '%s\nfish_add_path %s\n' "$marker" "$prefix/bin" >> "$fish_file"
      changed="$changed $fish_file"
    fi
  fi
fi

say ""
say "Installed $("$prefix/bin/fastf" --version) at $prefix/bin/fastf"

if [ "$on_path" = "yes" ]; then
  say "Run fastf to start."
elif [ -n "$changed" ]; then
  say "Added $prefix/bin to your PATH in:$changed"
  say "It is there in every new shell. In this one, run: . \"$HOME/.profile\""
else
  say "Run it as $prefix/bin/fastf, or put $prefix/bin on your PATH."
fi
