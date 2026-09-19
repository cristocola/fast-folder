#!/bin/sh
# Install fast-folder on any Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/cristocola/fast-folder/main/packaging/linux/install.sh | sh
#
# It downloads the statically linked release archive, checks it against the
# release's own SHA256SUMS, and unpacks the binary, the man pages, the shell
# completions, the desktop entry and the icons.
#
# Where it goes decides whether `fastf` works the moment this script ends, in
# the terminal that ran it. A script piped into `sh` cannot change that
# terminal's PATH, so the only places that work at once are ones already on it:
#
#   - As root: /usr/local, which every shell already looks in.
#   - An update goes where the last install went.
#   - If ~/.local/bin is already on your PATH: there.
#   - If you can use sudo and there is a terminal to ask in, it asks once:
#     /usr/local (works right away, here and in the app menu; sudo asks for your
#     password) or ~/.local (no password; works from the next terminal you open).
#   - With nobody to ask (a script, CI): ~/.local.
#
# A ~/.local install adds ~/.local/bin to your shell profiles so every new
# terminal has it. FASTF_VERSION pins a release. FASTF_INSTALL=system or
# FASTF_INSTALL=user answers the question in advance, and PREFIX installs
# somewhere else entirely.
set -eu

repo="cristocola/fast-folder"
target="x86_64-unknown-linux-musl"
system_prefix="/usr/local"
user_prefix="$HOME/.local"

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
# `redirect` prints where a URL sends you, which is how the latest tag is found.
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL -o "$1" "$2"; }
  read_url() { curl -fsSL "$1"; }
  redirect() { curl -fsSLI -o /dev/null -w '%{url_effective}' "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$1" "$2"; }
  read_url() { wget -qO - "$1"; }
  redirect() {
    wget -q -S --max-redirect=0 -O /dev/null "$1" 2>&1 \
      | tr -d '\r' | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1
  }
else
  die "curl or wget is required"
fi

# --- where it goes ---------------------------------------------------------

on_path() {
  case ":$PATH:" in
    *":$1:"*) return 0 ;;
  esac
  return 1
}

# Piped into `sh`, the script's stdin is the download, so a question has to be
# asked on the terminal itself. Opening /dev/tty fails when there is none.
have_tty() { (: </dev/tty) 2>/dev/null; }

# Whether sudo is worth offering: it answers without a password, or the user is
# in a group that sudo admits on Debian, Ubuntu, Mint (sudo), Arch and Fedora
# (wheel) or older Ubuntu (admin). A wrong guess only costs a failed sudo,
# after which the install goes to ~/.local anyway.
can_sudo() {
  command -v sudo >/dev/null 2>&1 || return 1
  sudo -n true 2>/dev/null && return 0
  case " $(id -Gn 2>/dev/null) " in
    *" sudo "*|*" wheel "*|*" admin "*) return 0 ;;
  esac
  return 1
}

ask_system() {
  {
    say ""
    say "Where should fastf go?"
    say "  /usr/local  works in this terminal and the app menu right away; sudo asks for your password"
    say "  ~/.local    just for you, no password; works from the next terminal you open"
    printf 'Install it in /usr/local? [Y/n] '
  } >/dev/tty
  reply=""
  read -r reply </dev/tty || reply=""
  case "$reply" in
    [Nn]*) return 1 ;;
  esac
  return 0
}

# `why` is kept for the one case that needs it: a system install the user was
# asked about can fall back to ~/.local when sudo fails, anything else cannot.
why=""
if [ -n "${PREFIX:-}" ]; then
  prefix=$PREFIX
elif [ "$(id -u)" = "0" ]; then
  prefix=$system_prefix
else
  case "${FASTF_INSTALL:-}" in
    system) prefix=$system_prefix ;;
    user) prefix=$user_prefix ;;
    "")
      if [ -x "$user_prefix/bin/fastf" ]; then
        prefix=$user_prefix
      elif [ -x "$system_prefix/bin/fastf" ]; then
        prefix=$system_prefix why=update
      elif on_path "$user_prefix/bin"; then
        prefix=$user_prefix
      elif can_sudo && have_tty && ask_system; then
        prefix=$system_prefix why=asked
      else
        prefix=$user_prefix
      fi
      ;;
    *) die "FASTF_INSTALL is 'system' or 'user', not '$FASTF_INSTALL'" ;;
  esac
fi

# Root for the system directories, when they are not ours to write already.
as_root=""
if [ "$prefix" = "$system_prefix" ] && [ "$(id -u)" != "0" ] && ! [ -w "$system_prefix/bin" ]; then
  command -v sudo >/dev/null 2>&1 || die "installing into $system_prefix needs root, and there is no sudo; run it as root or with FASTF_INSTALL=user"
  say "Writing to $system_prefix needs sudo."
  if sudo -v; then
    as_root="sudo"
  elif [ "$why" = "asked" ]; then
    say "sudo did not work, so it goes in $user_prefix instead."
    prefix=$user_prefix
  elif [ "$why" = "update" ]; then
    die "fastf is installed in $system_prefix and updating it needs sudo; run this again in a terminal, or as root"
  else
    die "sudo did not work, so nothing was installed"
  fi
fi

# --- download and check ----------------------------------------------------

# The latest tag comes from where github.com/<repo>/releases/latest redirects
# to (.../releases/tag/vX.Y.Z). The REST API says the same, but allows sixty
# unauthenticated requests an hour per address, which one office, campus or
# carrier-grade NAT shares, so it is only asked when the redirect says nothing.
version="${FASTF_VERSION:-}"
if [ -z "$version" ]; then
  say "Looking up the latest release..."
  version=$(redirect "https://github.com/$repo/releases/latest" 2>/dev/null \
    | sed -n 's|.*/releases/tag/\(v[^/?#]*\)$|\1|p')
  if [ -z "$version" ]; then
    version=$(read_url "https://api.github.com/repos/$repo/releases/latest" 2>/dev/null \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  fi
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

# --- install ---------------------------------------------------------------

put() { $as_root install "$@"; }

say "Installing into $prefix..."
put -Dm755 fastf "$prefix/bin/fastf"
put -Dm644 LICENSE "$prefix/share/doc/fast-folder/copyright"
put -Dm644 completions/fastf.bash "$prefix/share/bash-completion/completions/fastf"
put -Dm644 completions/fastf.zsh "$prefix/share/zsh/site-functions/_fastf"
put -Dm644 completions/fastf.fish "$prefix/share/fish/vendor_completions.d/fastf.fish"
for page in man/*.1; do
  put -Dm644 "$page" "$prefix/share/man/man1/$(basename "$page")"
done
# The menu entry names the binary by its full path. A desktop session reads
# PATH once, at login, and ~/.local/bin was not on it then, so `Exec=fastf`
# would do nothing from the menu until the next login.
sed "s|^Exec=fastf|Exec=$prefix/bin/fastf|" fastf.desktop > fastf.desktop.installed
put -Dm644 fastf.desktop.installed "$prefix/share/applications/fastf.desktop"
for size in 48 128 256; do
  put -Dm644 "icons/fastf-$size.png" \
    "$prefix/share/icons/hicolor/${size}x${size}/apps/fastf.png"
done

# Desktop environments pick the new entry up sooner when the database knows.
if command -v update-desktop-database >/dev/null 2>&1; then
  $as_root update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi

# --- PATH ------------------------------------------------------------------
#
# When the binary lands somewhere PATH does not already look, the job is to add
# it to the shell profiles, so that every new terminal has `fastf` without
# anyone being told to go and edit a file. The exact line is what is looked
# for, so running the installer again leaves a profile that has it alone, and
# deleting the marked two lines undoes it.
marker="# added by the fast-folder installer"
line="export PATH=\"$prefix/bin:\$PATH\""
changed=""

add_to_profile() {
  file="$1"
  if [ -e "$file" ] && grep -qF "$line" "$file" 2>/dev/null; then
    return 0
  fi
  printf '\n%s\n%s\n' "$marker" "$line" >> "$file" || return 0
  changed="$changed $file"
}

on_path_now=no
on_path "$prefix/bin" && on_path_now=yes

if [ "$on_path_now" = "no" ]; then
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
    if ! grep -qF "fish_add_path $prefix/bin" "$fish_file" 2>/dev/null; then
      printf '%s\nfish_add_path %s\n' "$marker" "$prefix/bin" >> "$fish_file"
      changed="$changed $fish_file"
    fi
  fi
fi

# --- what to do next ---------------------------------------------------------

# The first `fastf` on PATH, if any: another copy ahead of this one would be
# the one that runs, and nobody would guess why.
first_on_path() {
  (
    set -f
    IFS=:
    for dir in $PATH; do
      [ -n "$dir" ] && [ -x "$dir/fastf" ] && { printf '%s\n' "$dir/fastf"; exit 0; }
    done
    exit 1
  )
}

say ""
say "Installed $("$prefix/bin/fastf" --version) at $prefix/bin/fastf"

if [ "$on_path_now" = "no" ]; then
  [ -n "$changed" ] && say "Added $prefix/bin to your PATH in:$changed"
  say "Every terminal you open from now on has it: open a new one and run fastf."
  say "In this one, which started before, run it as $prefix/bin/fastf."
elif first=$(first_on_path) && [ "$first" != "$prefix/bin/fastf" ]; then
  # Only a copy that is ahead of this one on the same PATH shadows it. When
  # $prefix/bin was not on PATH, the line just added puts it in front.
  other=$("$first" --version 2>/dev/null || echo "another fastf")
  say "Note: $first ($other) comes before this one on your PATH, so that is"
  say "the one \`fastf\` runs. Remove it, or run this one as $prefix/bin/fastf."
else
  say "Run fastf to start."
fi
