# Confluence

An authored three-lane Magicka map for a later multiplayer/modding experiment.
This is a rewrite map, not a level installed into the original game.

From the repository root:

```sh
./demo.sh "$PWD/scenarios/confluence.lvl"
# Optional inspection tools:
./demo.sh "$PWD/scenarios/confluence.lvl" --debug-menu
./demo.sh "$PWD/scenarios/confluence.lvl" --map-overview
```

The normal demo exports user-supplied assets and prepares this map's missing
private scenery. All original files remain read-only. Repeated launches retain
edits to the `.arena` source. The debug level catalogue also discovers this map.

The 160 × 160 m layout has opposing corner bases, top/mid/bottom lanes, jungle
paths, six clearings, twelve tower sites, two riverbank objective sites and two
raised jungle overlooks. Four spawn locators are reserved per side. These are
placement markers only: no teams, towers, minion waves, camp AI, victory rules,
scenario triggers or Lua integration are implemented here. The existing network
experiment still allows four players total; extra markers do not expand it.

Magicka's exported spruce/ash trees, rocks, fallen timber and checkpoint ruins
are kitbashed with the original grass, forest-floor, cobble, water and ice
textures. Ruins mark future structure/objective sites. Sizes, placement, palette
and layout are authored choices, not reconstructed retail level data. Colliders
for scenery are coarse authored boxes. Vegetation is presently static.

The diagonal river uses actual runtime liquid geometry and the existing water
shader/freeze stream. Its three lane crossings have 0.3 m wading depth; sections
between them have a 4.3 m channel. Cold can freeze the surface, ice supports
characters, and thawing restores water. The existing drowning rules apply to
unfrozen deep water. Deep-water navigation triangles have the existing WATER
property: agents with the relevant movement ability still need an ice query to
cross them. Ordinary routes use the shallow fords. This does not invent special
MOBA river or ice rules.

Both overlooks have 4 m raised centers and 12 m sloping approaches. Collision,
rendered terrain and navigation carry actual elevation. Low horizontal rays hit
the slope; rocks and beams retain their existing trajectory/aim behavior. No
high-ground damage/accuracy bonus is added. Actual beam-versus-rock tactical
balance and multiplayer traversal require playtesting.

The camera follows the existing camera-mesh confinement path, bounded to the
inner 148 × 148 m and retaining the overlooks' height. Named `camera_blue_jungle`,
`camera_red_jungle` and `camera_river` areas are available for future mod logic.
They do not activate scripted camera changes. Keypad +/- zooms, 4/6 orbits, 8/2
tilts, 1/3 and 7/9 pan, 0 toggles target lock, and decimal resets inspection.
`--map-overview` starts an unlocked inspection camera over the map center.

## Editing and Blender

`confluence.arena` is the authoritative input. The paired `.lvl` and `.xml` select
it without scripting. Named locators use `blue_spawn0..3`, `red_spawn0..3`,
`lane_{top,mid,bot}_{index}`, `tower_{top,mid,bot}_{index}`, `camp_0..5`,
`objective_0..1`, `highground_blue`, `highground_red`, `river_shallow` and
`river_deep`. `start0..3` supplies the current runtime's player spawns.

To regenerate the initial design (overwrites the three authored map files):

```sh
python3 tools/build-confluence.py --assets
```

To refresh only private scenery, preserving map edits:

```sh
python3 tools/build-confluence.py --assets-only
```

Scenery extraction reads the existing private Swamp scene export, writes compact
GLBs and a hash/bounds provenance manifest under the private cache. It does not
execute the original game. If the exported scene layout changes, inspect and
update the extraction selection instead of silently selecting another asset.

The Blender MCP workflow executes the following code in the connected Blender:

```python
import runpy
module = runpy.run_path('/absolute/project/tools/confluence-blender.py')
result = module['build']('/absolute/project')
```

This creates a new scene with grouped terrain, linked original scenery,
collision wire markers, gameplay markers, camera bounds, lighting and an overview
camera. It saves a private copy at `output/.cache/confluence/confluence.blend`.
The `.blend` is an inspection/kitbashing starting point; edits in Blender do not
yet round-trip to `.arena`. Terrain heights follow the runtime's triangle grid.
Use `.arena` edits and F5/relaunch to change the playable map.

## Checks

```sh
cargo test --locked --offline --release -j 1 --manifest-path Cargo.toml \
  -p magicka-runtime --test confluence --test authored_scenarios --test water_collision --test drowning
./bin/confluence-qa
# Reuse a built viewer:
./bin/confluence-qa --skip-build
```

The map tests check live loading without triggers, all lane/camp/objective
connections, spawn reachability, solid tree positions, camera confinement and
height, low-ray terrain obstruction, actual wading/drowning/ice support, and a
shorter ice-enabled navigation crossing. Captures check both normal player
framing and the overview. These checks demonstrate rewrite behavior; they do not
establish original-game visual parity, match balance or multiplayer completeness.
