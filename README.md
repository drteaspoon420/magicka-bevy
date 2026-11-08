# magicka-bevy

A clean-room style reimplementation of the 2011 game *Magicka*, written in Rust on the
[Bevy](https://bevyengine.org/) engine. The goal is a preservation release that plays
like the original on modern systems, with additions such as larger multiplayer games,
dedicated servers and mods.

> **You need your own copy of Magicka.** This repository has no game assets: no
> models, textures, sounds, music, text or levels. The programs read the files from
> your own installation and convert them into a local cache on your machine.

## Disclaimer

**Not affiliated.** This is an unofficial fan project. It is not affiliated with,
endorsed by or connected to Paradox Interactive, Arrowhead Game Studios or any other
rights holder. "Magicka" is a trademark of its owners and is used here only to say
which game this software is compatible with.

**Made with AI.** Most of the code and documentation here was written with the help
of AI coding assistants, working from research into how the original behaves. All of
it was directed and reviewed by a person, but expect the mistakes and oddities of
that kind of work. Bug reports are welcome.

## Status

Has only been tested on Linux environment. Specifically on Bazzite.
Work in progress. Single-player campaign levels, spell casting, magicks, the menus
and online/LAN co-op are playable. Fidelity to the original varies: some systems
match it closely, while others are approximations or still missing.

## Requirements

- A Rust toolchain (stable, via [rustup](https://rustup.rs/)) and a C toolchain.
- Bevy's platform dependencies. On Linux these are ALSA and udev development
  packages plus Vulkan drivers; see
  [Bevy's Linux setup](https://github.com/bevyengine/bevy/blob/main/docs/linux_dependencies.md).
- FFmpeg on `PATH` (or `MAGICKA_FFMPEG=/path/to/ffmpeg`) to play voice acting.
- An installed copy of Magicka. The DLC content is used if it is present.

## Getting started

Link your game installation into the repository root as `Magicka` (the folder that
contains `Content/`), or point `MAGICKA_CONTENT` at its `Content` folder:

```sh
ln -s "/path/to/steamapps/common/Magicka" Magicka
# or: export MAGICKA_CONTENT="/path/to/steamapps/common/Magicka/Content"

./play.sh            # build, convert assets on first run, open the main menu
./play.sh --help     # levels, hosting, joining, mods and other options
```

The first run builds in release mode and converts the game's assets into `.cache/`.
This takes a while. Use `MAGICKA_CACHE` to put the cache somewhere else.

Default controls follow the original: hold left mouse to walk; `Q W E R A S D F` to
queue elements; right mouse to cast (hold Shift for an area cast); middle mouse to
cast on yourself; Space to skip cutscenes; Esc to pause.

## Multiplayer

```sh
./play.sh --host 0.0.0.0:17700              # play and host
./play.sh --join 192.168.1.10:17700          # join a host
./play.sh --server 0.0.0.0:17700 --level Havindr.lvl   # headless dedicated server
```

See [MULTIPLAYER.md](MULTIPLAYER.md) for lobbies, passphrases and server settings.
Some QA commands described there come from the private development environment and
are not included in this repository.

## Mods

Modding is Extremely experimental and untested. It was merely a test to make sure we can do it. API is very early one and practically untested.
Mods add definitions, server-side Lua and UI panels. Samples are in `mods/`, and
[docs/MODDING.md](docs/MODDING.md) explains the format. Check a mod with:

```sh
cargo run --locked -p magicka-script --bin magicka-modtool -- \
  --base game/base --mods mods --enable <id> validate
```

## Repository layout

| Path | Contents |
|---|---|
| `crates/magicka-xnb` | XNB container and LZX decompression |
| `crates/magicka-content` | Parsers for the game's content formats; glTF/texture exporters |
| `crates/magicka-sim` | Presentation-free gameplay rules: spells, damage, status effects |
| `crates/magicka-physics` | Physics |
| `crates/magicka-runtime` | Scenes, scripts, AI, UI state, dedicated server |
| `crates/magicka-network` | Networking, replication and lobbies |
| `crates/magicka-viewer` | The Bevy game client |
| `crates/magicka-script`, `magicka-mods`, `magicka-definitions` | Mod loading, Lua and definitions |
| `game/base`, `mods/`, `scenarios/` | Authored definitions, sample mods and test scenarios |
| `tools/` | Development and verification scripts |

## Development

```sh
cargo check --locked
cargo test --locked
```

Most tests run against the original game data, so the full suite needs your game
install: link it as `./Magicka` or set `MAGICKA_CONTENT` to its `Content` folder.
Without it, many tests fail with "file not found".

This repository is published from a separate, private research workspace. Commits
here are periodic snapshots of that workspace, so history is coarse.

## License

The source code is licensed under the [MIT license](LICENSE).

Third-party material keeps its own license. For example, the
controller prompt images embedded in `assets/input_prompts/` are from Kenney Input
Prompts (CC0). The license covers only this repository's contents. It grants no
rights to Magicka or its assets.
