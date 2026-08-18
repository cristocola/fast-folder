#!/usr/bin/env bash
#
# Drive the guided TUI against a disposable library.
#
# Builds from source every run, so you always test what is in the working tree.
# The sandbox is isolated by FASTF_INSTALL_DIR + HOME, so nothing here can see
# or touch your real config, /mnt/proj, /mnt/base, or the installed fastf.
#
#   dev/tui-sandbox.sh             build (debug) + open the TUI
#   dev/tui-sandbox.sh --release   same, optimized (slower build, LTO)
#   dev/tui-sandbox.sh --reset     delete the fixture; next run reseeds it
#   dev/tui-sandbox.sh --shell     print env exports for ad-hoc CLI commands
#
# See dev/README.md for what the fixture contains and why.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SANDBOX="${FASTF_SANDBOX:-$REPO/target/tui-sandbox}"
PROFILE=debug
CARGO_FLAGS=()

for arg in "$@"; do
  case "$arg" in
    --release) PROFILE=release; CARGO_FLAGS=(--release) ;;
    --reset)   rm -rf "$SANDBOX"; echo "sandbox wiped: $SANDBOX"; exit 0 ;;
    --shell)
      echo "export FASTF_INSTALL_DIR=$SANDBOX/config"
      echo "export HOME=$SANDBOX/home"
      echo "# then run: $REPO/target/$PROFILE/fastf <args>"
      exit 0 ;;
    -h|--help) sed -n '3,14p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

cargo build "${CARGO_FLAGS[@]}" --manifest-path "$REPO/Cargo.toml" >/dev/null
BIN="$REPO/target/$PROFILE/fastf"

export FASTF_INSTALL_DIR="$SANDBOX/config"
export HOME="$SANDBOX/home"

# Every fixture command runs through the binary under test, so the fixture can
# never describe a library shape the current code would not produce.
seed() {
  local projects="$SANDBOX/projects" archive="$SANDBOX/archive"
  mkdir -p "$SANDBOX"/{config,home} "$projects" "$archive"

  "$BIN" config set base-dir "$projects" >/dev/null
  "$BIN" config set bases    "$archive"  >/dev/null
  # Two pages out of six projects, so paging is always exercised.
  "$BIN" config set recent-default-limit 4 >/dev/null
  "$BIN" config set default-template client-project >/dev/null
  "$BIN" config set editor "${EDITOR:-nano}" >/dev/null

  new_general() { "$BIN" new general --name="$1" --yes --no-post >/dev/null; }
  new_client()  {
    "$BIN" new client-project --client="$1" --project="$2" --tier="$3" \
      --yes --no-post >/dev/null
  }

  new_general Client_Reel          # ID0001 — gets the big payload
  new_general Album_Art            # ID0002 — multi-tagged
  new_client  Acme Launch_Film Client     # ID0003 — auto-tags from `tier`
  new_general Doc_Cut              # ID0004 — untagged, no journal
  new_client  Globex Sizzle Internal   # ID0005

  # A sixth project in the *second* base, so moves and base labels are real.
  "$BIN" new general --name=Old_Session --base-dir="$archive" --yes --no-post >/dev/null

  "$BIN" tag add ID0001 draft         >/dev/null
  "$BIN" tag add ID0002 draft         >/dev/null
  "$BIN" tag add ID0002 urgent        >/dev/null
  "$BIN" tag add ID0002 client/Acme   >/dev/null
  "$BIN" tag add ID0002 needs-review   >/dev/null   # 4 tags: exercises the "+1" truncation
  "$BIN" tag add ID0003 urgent        >/dev/null
  "$BIN" note add ID0001 "first cut sent to client" >/dev/null
  "$BIN" note add ID0001 "revision 2 approved"      >/dev/null

  # Spread the creation dates. Everything seeded in one run is stamped with the
  # same timestamp, which makes newest-first ordering and `created>` filters
  # untestable — so backdate a few and reindex, since the per-base cache holds
  # its own copy of `created`.
  backdate() {
    local dir stamp
    dir=$(find "$projects" "$archive" -maxdepth 1 -type d -name "*$1*" | head -1)
    stamp="$2T09:00:00Z"
    sed -i -E "s/^created: .*/created: \"$stamp\"/" "$dir/PROJECT_INFO.md"
  }
  backdate ID0001 2026-02-11
  backdate ID0002 2026-04-02
  backdate ID0003 2026-05-27
  backdate ID0006 2025-11-30
  "$BIN" reindex >/dev/null

  # Big enough that the on-demand size walk is visible instead of instant.
  local big
  big=$(find "$projects" -maxdepth 1 -type d -name '*ID0001*' | head -1)
  head -c 40000000 /dev/urandom > "$big/payload.bin"

  # An unregistered folder, so Register has something real to point at.
  mkdir -p "$projects/loose_folder_no_metadata"
  echo "notes" > "$projects/loose_folder_no_metadata/README.md"

  echo "seeded: 6 projects across 2 bases, 1 unregistered folder"
  echo "        $projects"
  echo "        $archive"
  echo
}

[[ -d "$SANDBOX/config" ]] || seed

exec "$BIN"
