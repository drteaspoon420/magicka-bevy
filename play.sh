#!/usr/bin/env bash
# Magicka rewrite demo: build the viewer, export any missing glTF assets from
# the retail files in target/, then open the main menu (or play an explicit level).
#
#   ./play.sh                     open the main menu
#   ./play.sh --skip-build        open the main menu using the existing build
#   ./play.sh Wizard_Castle.lvl   play another level (with its intro scripts)
#   ./play.sh Havindr.lvl --debug-menu   extra viewer flags pass through
#                                 (--debug-menu opens the debug menu at start)
#   ./play.sh --record            headless fire-spray capture → output/.cache/demo-cast/*.png
#   ./play.sh --mods boosted,example.arena   enable these mods (by Id)
#   ./play.sh --no-mods           play without mods
# Mods load from output/rewrite/mods; by default the ones listed in
# output/rewrite/mods/load_order.toml. Hosts and clients need the same mods.
# Mod UI panels: F4 (number keys pick).
#   CARGO_NET_OFFLINE=true ./play.sh   build using only cached dependencies
#
# Controls (original default bindings):
#   hold left mouse   walk toward the cursor (click also advances dialog)
#   Q W E R           conjure Water / Life / Shield / Cold
#   A S D F           conjure Lightning / Arcane / Earth / Fire
#   hold right mouse  cast (Force; hold Shift for Area), middle mouse = Self
#   F5 restart, Esc pause menu (death shows "Defeated" and restarts after ~6 s)
#   Space             skip a cutscene (first press shows the tip)
#   F1                experimental debug menu: level/scene switch, spawn NPCs,
#                     robe/staff/weapon/colour, teleport, triggers, heal/kill,
#                     skip cutscene
#   keypad            experimental camera: +/- zoom, 4/6 orbit, 8/2 tilt,
#                     1/3 7/9 pan, 0 lock/unlock target, . reset
# Try: F F + right mouse (fire spray), A A + right mouse (lightning),
#      D + hold/release right mouse (rock),
#      empty queue + hold/release right mouse (push), W + middle mouse (heal).
set -euo pipefail
cd "$(dirname "$0")"
LAB=$PWD
# Settings come from the environment first, then from ./.env (written by this
# script when you choose to save a found game install). Only these keys are
# read from .env, and the file is parsed, never executed.
ENV_FILE=$LAB/.env
ENV_KEYS=" MAGICKA_CONTENT MAGICKA_CACHE MAGICKA_FFMPEG "
if [ -f "$ENV_FILE" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    line=${line%$'\r'}
    [[ "$line" =~ ^[[:space:]]*(export[[:space:]]+)?([A-Z_]+)=(.*)$ ]] || continue
    key=${BASH_REMATCH[2]} value=${BASH_REMATCH[3]}
    [[ "$ENV_KEYS" == *" $key "* ]] || continue
    [ -z "${!key:-}" ] || continue  # the real environment wins
    if [[ "$value" =~ ^\"(.*)\"$ || "$value" =~ ^\'(.*)\'$ ]]; then value=${BASH_REMATCH[1]}; fi
    export "$key=$value"
  done < "$ENV_FILE"
fi
# MAGICKA_CONTENT / MAGICKA_CACHE override the retail Content folder and the
# converted-asset cache. In the public repository the workspace is the root
# and the game install is linked or copied to ./Magicka, saved in .env, or
# found in a Steam library.
if [ -f "$LAB/Cargo.toml" ]; then
  REWRITE=$LAB
  DEFAULT_CONTENT=$LAB/Magicka/Content
  CACHE=${MAGICKA_CACHE:-$LAB/.cache}
else
  REWRITE=$LAB/output/rewrite
  DEFAULT_CONTENT=$LAB/target/Magicka/Content
  CACHE=${MAGICKA_CACHE:-$LAB/output/.cache}
fi
CONTENT=

interactive() { [ -t 0 ] || [ "${MAGICKA_PLAY_INTERACTIVE:-}" = 1 ]; }
ask() { # ask "Question" default(y|n) -> status 0 for yes
  local reply hint='[y/N]'
  [ "$2" = y ] && hint='[Y/n]'
  printf '%s %s ' "$1" "$hint" >&2
  IFS= read -r reply || reply=
  reply=$(printf '%s' "$reply" | tr '[:upper:]' '[:lower:]')
  case "$reply" in
    y|yes) return 0 ;;
    n|no) return 1 ;;
    *) [ "$2" = y ] ;;
  esac
}
# Accept either the game install folder or its Content folder.
content_of() {
  local dir=${1%/}
  case "$dir" in "~"|"~/"*) dir=$HOME${dir#\~} ;; esac
  if [ -d "$dir/Content/Levels" ]; then printf '%s\n' "$dir/Content"
  elif [ -d "$dir/Levels" ] && [ "${dir##*/}" = Content ]; then printf '%s\n' "$dir"
  else return 1
  fi
}
# Every Steam library on this machine, from each Steam root's libraryfolders.vdf.
steam_libraries() {
  local root vdf
  for root in "$HOME/.steam/steam" "$HOME/.steam/root" "$HOME/.local/share/Steam" \
      "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam" \
      "$HOME/snap/steam/common/.local/share/Steam" \
      "$HOME/Library/Application Support/Steam" \
      "/c/Program Files (x86)/Steam" "/mnt/c/Program Files (x86)/Steam"; do
    [ -d "$root/steamapps" ] || continue
    printf '%s\n' "$root"
    vdf=$root/steamapps/libraryfolders.vdf
    [ -f "$vdf" ] || continue
    sed -n 's/^[[:space:]]*"path"[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p' "$vdf" | sed 's/\\\\/\\/g'
  done
}
find_steam_content() {
  local lib found seen=" "
  while IFS= read -r lib; do
    found=$(content_of "$lib/steamapps/common/Magicka") || continue
    found=$(cd "$found" && pwd -P)
    [[ "$seen" == *" $found "* ]] && continue
    seen="$seen$found "
    printf '%s\n' "$found"
  done < <(steam_libraries)
}
save_content() {
  if [[ "$1" == *'"'* || "$1" == *$'\n'* ]]; then
    echo "Not saving: the path contains a quote or newline." >&2; return 0
  fi
  local tmp=$ENV_FILE.tmp
  { [ -f "$ENV_FILE" ] && grep -vE '^[[:space:]]*(export[[:space:]]+)?MAGICKA_CONTENT=' "$ENV_FILE" || true
    printf 'MAGICKA_CONTENT="%s"\n' "$1"; } > "$tmp"
  mv "$tmp" "$ENV_FILE"
  echo "Saved to $ENV_FILE; later runs use it. Delete that line to choose again." >&2
}
offer_save() {
  if interactive && ask "Save this location to .env for future runs?" y; then save_content "$1"; fi
}
resolve_content() {
  if [ -n "${MAGICKA_CONTENT:-}" ]; then
    CONTENT=$MAGICKA_CONTENT
    [ -d "$CONTENT" ] || { echo "MAGICKA_CONTENT points to a missing folder: $CONTENT (set in the environment or $ENV_FILE)" >&2; exit 1; }
    return
  fi
  if [ -d "$DEFAULT_CONTENT" ]; then CONTENT=$DEFAULT_CONTENT; return; fi
  local found=() path
  mapfile -t found < <(find_steam_content)
  for path in "${found[@]}"; do
    if ! interactive; then
      CONTENT=$path
      echo "Using Magicka from Steam: $CONTENT (run interactively to save it to .env)" >&2
      return
    fi
    if ask "Found Magicka in Steam: $path
Use it?" y; then
      CONTENT=$path; offer_save "$path"; return
    fi
  done
  if interactive; then
    echo "Magicka was not found${found:+ (or no Steam install was chosen)}." >&2
    while :; do
      printf 'Path to your Magicka install (or its Content folder), empty to quit: ' >&2
      IFS= read -r path || path=
      [ -n "$path" ] || exit 1
      if CONTENT=$(content_of "$path"); then offer_save "$CONTENT"; return; fi
      echo "No Magicka Content folder (with Levels/) at: $path" >&2
    done
  fi
  echo "Magicka game files not found. Link the install as $LAB/Magicka, set MAGICKA_CONTENT," >&2
  echo "or run ./play.sh in a terminal to search Steam and choose." >&2
  exit 1
}

MODE=offline
ADDRESS=127.0.0.1:17700
LEVEL=
EXTRA=()
SKIP_BUILD=false
PRINT_CONTENT=false
EXPORT_ONLY=false
RECORD=false
MODS=load-order
while [ $# -gt 0 ]; do
  case "$1" in
    --server|--host|--join)
      [ "$MODE" = offline ] || { echo 'Choose only one of --server, --host or --join' >&2; exit 2; }
      MODE=${1#--}
      shift
      if [[ "${1:-}" == *:* ]]; then ADDRESS=$1; shift
      elif [ "$MODE" = join ]; then echo '--join requires an IP:port' >&2; exit 2
      fi ;;
    --level)
      [ $# -ge 2 ] || { echo '--level requires a file' >&2; exit 2; }
      LEVEL=$2; shift 2 ;;
    --record) RECORD=true; shift ;;
    --mods)
      [ $# -ge 2 ] || { echo '--mods requires a comma-separated list of mod Ids' >&2; exit 2; }
      MODS=$2; shift 2 ;;
    --no-mods) MODS=none; shift ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    --print-content) PRINT_CONTENT=true; shift ;;
    --export-only) EXPORT_ONLY=true; shift ;;
    --help|-h)
      cat <<'EOF'
Usage:
  ./play.sh                              main menu
  ./play.sh [level.lvl] [viewer options]
  ./play.sh --server [127.0.0.1:17700] [--level level.lvl] [--no-scripts]
  ./play.sh --host [127.0.0.1:17700] [--level level.lvl] [--debug-menu]
  ./play.sh --join 127.0.0.1:17700 [--name Name] [--character Wizard] [--robe-colour R,G,B]
            [--role player|spectator|dm|invader]
  ./play.sh --skip-build ...
  ./play.sh --export-only                build and refresh the asset cache, then exit
  ./play.sh --print-content              find the game files (Steam, .env), print the path, exit
  ./play.sh --mods id1,id2 ...           enable these mods instead of load_order.toml
  ./play.sh --no-mods ...                play without mods
Hosts (--server/--host) accept --lobby players=8,hotjoin=off,spectators=on,dm=on,invaders=on.
Four players and open hot join are the defaults (Versus locks hot join).
Networking defaults to the authored feature lab. Games are open unless MAGICKA_PASSPHRASE
or --passphrase-file FILE makes them private; see MULTIPLAYER.md.
--server runs without a display and skips all asset exports. --host is a playable host.
The server owns player state, pickups and respawns. Stop a headless server with Ctrl+C.
EOF
      exit 0 ;;
    *.lvl)
      if [ -z "$LEVEL" ]; then LEVEL=$1; else EXTRA+=("$1"); fi
      shift ;;
    *) EXTRA+=("$1"); shift ;;
  esac
done
START_MENU=false
if [ "$MODE" = offline ] && [ -z "$LEVEL" ] && [ "$RECORD" = false ]; then
  START_MENU=true
fi
if [ -z "$LEVEL" ]; then
  if [ "$MODE" = offline ]; then LEVEL=Havindr.lvl
  else LEVEL=$REWRITE/scenarios/feature-lab.lvl
  fi
fi
BIN=$REWRITE/target/release

MOD_ARGS=()
case "$MODS" in
  none) ;;
  load-order) MOD_ARGS=(--mods "$REWRITE/mods") ;;
  *) MOD_ARGS=(--mods "$REWRITE/mods" --enable "$MODS") ;;
esac

resolve_content
if [ "$PRINT_CONTENT" = true ]; then printf '%s\n' "$CONTENT"; exit 0; fi

if [ "$MODE" = server ]; then
  if [ "$SKIP_BUILD" = false ]; then
    cargo build --locked -j1 --release --manifest-path "$REWRITE/Cargo.toml" -p magicka-runtime --bin magicka-server
  fi
  exec "$BIN/magicka-server" --content "$CONTENT" --level "$LEVEL" --bind "$ADDRESS" "${MOD_ARGS[@]}" "${EXTRA[@]}"
fi

# Dialog and character voices are xWMA, decoded by the runtime with FFmpeg.
# Nix does not expose packages in the store on PATH automatically.
if [ -z "${MAGICKA_FFMPEG:-}" ] && ! command -v ffmpeg >/dev/null 2>&1; then
  if command -v nix-shell >/dev/null 2>&1; then
    echo '== preparing FFmpeg for voice acting (Nix) =='
    MAGICKA_FFMPEG=$(nix-shell -p ffmpeg-headless --run 'command -v ffmpeg')
    export MAGICKA_FFMPEG
  else
    echo 'Voice acting requires FFmpeg on PATH or MAGICKA_FFMPEG=/path/to/ffmpeg.' >&2
  fi
fi

echo "== building (release) =="
# debug-ui adds the experimental egui developer menu (F1) to the demo build.
export RUST_MIN_STACK="${RUST_MIN_STACK:-33554432}"
if [ "$SKIP_BUILD" = false ]; then
  cargo build --locked -j1 --release --manifest-path "$REWRITE/Cargo.toml" \
    -p magicka-viewer -p magicka-content --features magicka-viewer/debug-ui
fi

# Asset export steps and their versions live in tools/asset-exports.txt, shared
# with play.ps1; that file explains the manifest and version rules.
EXPORTS=$CACHE/exports
manifest_complete() {
  local file
  while IFS= read -r file; do
    [ -e "$file" ] || return 1
  done < <(tail -n +2 "$1")
}
legacy_version() { # legacy_version exists:PATH|stamp:PATH|-
  case "$1" in
    exists:*) [ -e "$CACHE/${1#exists:}" ] && echo 1 || true ;;
    stamp:*) cat "$CACHE/${1#stamp:}" 2>/dev/null || true ;;
  esac
}
export_step() { # export_step NAME VERSION LEGACY PROGRAM ARGUMENTS...
  local name=$1 version=$2 previous
  previous=$(legacy_version "$3")
  local program=$BIN/$4
  shift 4
  local args=() arg
  for arg in "$@"; do
    arg=${arg//\{content\}/"$CONTENT"}
    args+=("${arg//\{cache\}/"$CACHE"}")
  done
  local manifest=$EXPORTS/$name.txt
  if [ -f "$manifest" ]; then
    previous=$(head -n 1 "$manifest")
    [ "$previous" = "$version" ] && manifest_complete "$manifest" && return 0
  fi
  if [ "$previous" = "$version" ]; then
    echo "== exporting missing $name =="
    args+=(--missing)
  else
    echo "== exporting $name =="
  fi
  mkdir -p "$EXPORTS"
  "$program" "${args[@]}" --manifest "$manifest.tmp"
  { echo "$version"; cat "$manifest.tmp"; } > "$manifest"
  rm -f "$manifest.tmp"
}
while read -r -a step <&3; do
  [ "${#step[@]}" -eq 0 ] || [[ "${step[0]}" == \#* ]] && continue
  export_step "${step[@]}"
done 3< "$REWRITE/tools/asset-exports.txt"
# Custom scenery stays in the private cache; preserve hand-edited authored map data.
if [[ "${LEVEL##*/}" == confluence.lvl && ! -f "$CACHE/gltf/Custom/Confluence/spruce.glb" ]]; then
  python3 "$REWRITE/tools/build-confluence.py" --assets-only
fi

if [ "$EXPORT_ONLY" = true ]; then
  echo "== asset cache up to date =="
  exit 0
fi

if [ "$RECORD" = true ]; then
  OUT=$CACHE/demo-cast
  rm -rf "$OUT"
  "$BIN/magicka-viewer" --play "$CONTENT" --root "$CACHE" --level Havindr.lvl \
    --no-scripts --autopilot "$OUT" --autopilot-script cast
  CHECK=$LAB/output/evidence/particles/check_cast_autopilot.py
  if [ -f "$CHECK" ]; then python3 "$CHECK" "$OUT"; fi
  echo "frames: $OUT/0*.png (06/07 = spraying)"
  exit 0
fi

if [ "$START_MENU" = true ]; then
  echo "== main menu =="
  exec "$BIN/magicka-viewer" --menu "$CONTENT" --root "$CACHE" "${MOD_ARGS[@]}" "${EXTRA[@]}"
fi
case "$MODE" in
  host) EXTRA+=(--network-host "$ADDRESS") ;;
  join) EXTRA+=(--network-join "$ADDRESS") ;;
esac
echo "== playing $LEVEL =="
exec "$BIN/magicka-viewer" --play "$CONTENT" --root "$CACHE" --level "$LEVEL" "${MOD_ARGS[@]}" "${EXTRA[@]}"
