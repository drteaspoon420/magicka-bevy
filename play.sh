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
    cargo build --locked --offline -j1 --release --manifest-path "$REWRITE/Cargo.toml" -p magicka-runtime --bin magicka-server
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
  cargo build --locked --offline -j1 --release --manifest-path "$REWRITE/Cargo.toml" \
    -p magicka-viewer -p magicka-content --features magicka-viewer/debug-ui
fi

# Each export step keeps a manifest in $EXPORTS: its version, then every file
# it produced. A step runs again when its version changes (re-exporting
# everything) or when any listed file is missing (exporting only what is
# missing, --missing). The legacy argument is the version an existing cache
# had under the old marker files, so it is completed rather than rebuilt.
EXPORTS=$CACHE/exports
manifest_complete() {
  local file
  while IFS= read -r file; do
    [ -e "$file" ] || return 1
  done < <(tail -n +2 "$1")
}
export_step() { # export_step NAME VERSION LEGACY_VERSION command...
  local name=$1 version=$2 previous=$3
  shift 3
  local manifest=$EXPORTS/$name.txt
  if [ -f "$manifest" ]; then
    previous=$(head -n 1 "$manifest")
    [ "$previous" = "$version" ] && manifest_complete "$manifest" && return 0
  fi
  local missing=()
  if [ "$previous" = "$version" ]; then
    echo "== exporting missing $name =="
    missing=(--missing)
  else
    echo "== exporting $name =="
  fi
  mkdir -p "$EXPORTS"
  "$@" "${missing[@]}" --manifest "$manifest.tmp"
  { echo "$version"; cat "$manifest.tmp"; } > "$manifest"
  rm -f "$manifest.tmp"
}
model_step() { # model_step NAME VERSION LEGACY_VERSION model-gltf arguments...
  local name=$1 version=$2 previous=$3
  shift 3
  export_step "$name" "$version" "$previous" "$BIN/model-gltf" "$CONTENT" "$CACHE/textures" "$CACHE/gltf" "$@"
}
stamp() { cat "$CACHE/gltf/$1" 2>/dev/null || true; }
# Single-asset steps existed before versioning: any old output counts as 1.
exists() { [ -e "$CACHE/gltf/$1" ] && echo 1 || true; }

# Every model step below was bumped once on 2026-11-02: packed XNA Color
# COLOR0 vertex colours (VertexElementFormat.Color) are now exported.
export_step textures 1 "$([ -d "$CACHE/textures" ] && echo 1)" \
  "$BIN/tex-export" "$CONTENT" "$CACHE/textures"
# Bump when level-gltf output changes so existing caches are re-exported
# (2: liquid tangents/deep weight and magicka_liquid material extras;
#  3: spec/emissive material extras for the original deferred lighting;
#  4: material texture URIs; 5: lava material and initial freeze stream;
#  6: shadow-only (invisible, shadow-casting) meshes).
export_step levels 7 "$(stamp .level-export-version)" \
  "$BIN/level-gltf" "$CONTENT" "$CACHE/textures" "$CACHE/gltf" --all
# Custom scenery stays in the private cache; preserve hand-edited authored map data.
if [[ "${LEVEL##*/}" == confluence.lvl && ! -f "$CACHE/gltf/Custom/Confluence/spruce.glb" ]]; then
  python3 "$REWRITE/tools/build-confluence.py" --assets-only
fi
# 4: skinned SpecularMap (material map) extras.
# 5: native diffuse-alpha cutout (MASK).
export_step characters 6 "$(stamp .character-export-version)" \
  "$BIN/character-gltf" "$CONTENT" "$CACHE/textures" "$CACHE/gltf" --reference --all

model_step missiles 2 "$(exists Models/Missiles)" --missiles
# 2: native diffuse-alpha cutout (MASK).
model_step physics-props 3 "$(stamp .physics-props-exported)" --physics
# 4: item projectile models.
# 5: native diffuse-alpha cutout (MASK).
# 6: preserve native deferred material parameters on static equipment/projectiles.
model_step equipment 7 "$(stamp .all-items-exported)" --items
# 2: native diffuse-alpha cutout (MASK).
model_step gibs 3 "$(stamp .gibs-exported)" --gibs

# Books are not referenced by Item assets, so --items does not export them.
model_step magick-book 2 "$(exists Models/Items_Wizard/magickbook_major.glb)" \
  Models/Items_Wizard/magickbook_major
# Assatur's staff (StaffRenderData; a static Model drawn at the staff joint).
model_step assatur-staff 2 "$(exists Models/Bosses/assatur/assatur_staff.glb)" \
  Models/Bosses/assatur/assatur_staff
# Skinned magick models drawn by their entities (Tornado, Summon Phoenix).
model_step magick-models 2 "$(exists Models/Magicks/Phoenix.glb)" Models/Magicks/tornado \
  "Models/Magicks/tornado_debri@Models/Magicks/tornado" Models/Magicks/Phoenix
# Wave pillars: earthBarrier0's mesh with rockpillar_animation's clips.
model_step wave-pillars 2 "$(exists "Models/Effects/earthBarrier0_mesh+rockpillar_animation.glb")" \
  "Models/Effects/earthBarrier0_mesh@@Models/Effects/rockpillar_animation"

# Magicka's animated mouse cursors (Cursors/*.ani next to Content).
export_step cursors 1 "$([ -f "$CACHE/cursors/cursors.json" ] && echo 1)" \
  "$BIN/cursor-export" "$CONTENT/.." "$CACHE/cursors"

# Summon Elemental's ElementalEgg draws the elemental mesh (posed at runtime).
model_step elemental 2 "$(exists Models/Characters/Elemental/elemental_mesh.glb)" \
  Models/Characters/Elemental/elemental_mesh
# Death is a dedicated entity with an independently attached scythe.
model_step death 2 "$(exists Models/Bosses/death/death.glb)" \
  Models/Bosses/Death/Death Models/Bosses/Death/death_Scythe
# Jormungandr is a dedicated boss, outside the character-template export.
model_step jormungandr 2 "$(exists Models/Bosses/jormungandr/jormungandr.glb)" \
  Models/Bosses/Jormungandr/Jormungandr
# Grimnir's mesh uses the separate animation model's skeleton and clips.
model_step grimnir 2 "$(exists Models/Bosses/grimnir/grimnir_mesh.glb)" \
  Models/Bosses/grimnir/grimnir_mesh@Models/Bosses/grimnir/grimnir_animation
# The Machine boss (Havindr) and the bound king, posed at runtime.
model_step the-machine 2 "$(exists Models/Bosses/themachine/themachine.glb)" \
  Models/Bosses/theMachine/themachine Models/Characters/Human/King_animation
# Fafnir (Ruins) and its falling ceiling debris.
model_step fafnir 2 "$(exists Models/Bosses/fafnir/fafnir_mesh.glb)" \
  "Models/Bosses/fafnir/fafnir_mesh@Models/Bosses/fafnir/fafnir_animation" Models/Bosses/fafnir/Ceiling_Debri
# Tsar bosses: Parker (Tsar_WizardCastle), Cthulhu and its tentacles (Tsar_Rlyeh).
model_step tsar-bosses 2 "$(exists Models/Bosses/parker/parker.glb)" \
  "Models/Bosses/parker/parker@Models/Bosses/parker/parker_animation" \
  "Models/Bosses/cthulhu/cthulhu_mesh@Models/Bosses/cthulhu/cthulhu_animations" \
  "Models/Bosses/cthulhu/tentacle_mesh@Models/Bosses/cthulhu/tentacle_animations"

# Spell mines, barriers and other effect models.
# 2: magicka_skinned extras (mine overlay tint, material map).
# 5: barrier rune discs and the weapon IceBlade.
# 6: campaign dispensers.
# 7: Entangle_web (Parker's webs).
model_step effect-models 8 "$(stamp .effect-models-version)" Models/Effects/mine \
  "Models/Effects/earthBarrier0_mesh@Models/Effects/earthBarrier_animation" \
  "Models/Effects/iceBarrier0_mesh@Models/Effects/iceBarrier_animation" \
  Models/Effects/icebarrier_geyser Models/Effects/icebarrier_tesla \
  Models/Effects/iceBarrier_earth Models/Effects/earthBarrier_volcano \
  Models/Effects/combined_armor Models/Effects/armor_aura \
  Models/Effects/runesArcane Models/Effects/runesLife Models/Effects/Ice_Blade \
  Models/Effects/SpiderHole Models/Effects/GoblinTent Models/Effects/SpiderEggs \
  Models/Effects/Entangle_web

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
