# Authored scenarios

These are clean, authored levels for the rewrite. They reuse the runtime's level
and scene XML host and reference user-supplied character/item assets. They do
not modify retail content and are not levels exported for the original game.

From the repository root, launch the feature lab with:

```sh
./play.sh "$PWD/scenarios/feature-lab.lvl" --debug-menu
```

F1 opens the debug launcher. The Levels window includes these scenarios, and
Other entities spawns additional props, books, equipment and particles. To scan
your own scenario directory, pass `--scenarios /absolute/path` to the viewer.
A custom `.lvl` can also be passed directly using its absolute path.

The lab starts with a teleport book, a clock staff, a falling bathball and an
idle goblin. A central wall exercises collision and navigation around an
obstacle. The portal area at x=-7, z=12 changes to the corridor scene; its portal
at x=-6, z=7 returns to the lab. Both scenes create an in-memory checkpoint.
F5 restarts the level; checkpoint restart is available in the debug tools.
The portals currently have no visible marker. Coordinates can be inspected or
reached with the debug teleport controls.

Edit `lab.arena` or `corridor.arena`, then restart the level to rebuild geometry,
collision and navigation. The v1 `.arena` XML format supports:

| Element | Required attributes | Meaning |
| --- | --- | --- |
| Arena | version="1" | Format root |
| Floor | center="x,y,z", size="width,depth", cell, clearance, height | Floor and navigation grid; clearance expands obstacle footprints, height filters overhead obstacles |
| Box | center="x,y,z", size="width,height,depth" | Axis-aligned visible/collision obstacle |
| Locator | name, position="x,y,z" | Named spawn/action transform; start0..3 provide player slots |
| Area | name, center="x,y,z", size="width,height,depth" | Trigger volume |
| DirectionalLight | name, direction="x,y,z", diffuse="r,g,b", ambient="r,g,b" | Scene lighting |

Dimensions and vectors must be finite, and dimensions/cell/height positive.
Clearance is nonnegative. Navigation is capped at 100 cells per axis. Scene
scripts live in `.xml`; the `.lvl` names scenes and the initial player locator
prefix. See the supplied files for working spawn, checkpoint and transition
examples. Scene/action identifiers should be ASCII.

Reproduce checks and captures:

```sh
cargo test --offline --manifest-path Cargo.toml -p magicka-runtime --test authored_scenarios
./bin/viewer-qa feature-lab
./bin/viewer-qa --skip-build feature-lab-moved
```

The moved fixture copies only authored files into private cache, shifts the wall
and player spawn, and captures the result. Source and retail files stay intact.
The viewer renders the collision triangles directly; there is no cached floor
GLB to become stale after edits.

This is the first feature lab, not complete coverage of every subsystem. More
stations, liquids, breakable behavior, custom Blender scenery and networking
remain to be added. Existing runtime limitations still apply to casts, AI and
items; loading this level does not establish original-game parity.

## Confluence three-lane map

[Confluence](CONFLUENCE.md) is a larger authored forest arena with original
Magicka scenery, three lanes, six jungle clearings, shallow river fords, a deep
freezable channel, raised ground, navigation and camera bounds. It is a layout
sandbox without scenario scripts or match rules.

The v1 authoring format also supports `Surface` (textured, terrain-draped visual
rectangle), `Scenery` (private-cache GLB placement), `River` (diagonal channel with
named ford coordinates), `Rise` (raised center and sloped apron) and
`CameraBounds` (camera-mesh footprint). `Box visible="false"` retains collision
and navigation blocking while hiding the debug box. See the Confluence source
and generator for complete attribute examples. Asset paths are cache-relative
and cannot contain parent traversal. Presentation metadata is validated during
runtime loading. River and rise dimensions are design inputs.
