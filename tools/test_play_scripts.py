#!/usr/bin/env python3
"""Tests for the launchers play.sh and play.ps1, run against both.

Each test runs a launcher in a throwaway public-repository layout with a fake
$HOME holding fake Steam libraries and fake workspace binaries that record how
they are called. Nothing is built and no real game files are needed.

    python3 tools/test_play_scripts.py

play.ps1 runs with `pwsh` from PATH, or in a container when
MAGICKA_TEST_PWSH_IMAGE names a PowerShell image, for example
mcr.microsoft.com/powershell:latest (podman, or docker via MAGICKA_TEST_ENGINE).
Without either, the PowerShell cases are skipped.
"""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
EXPORTS = HERE / "asset-exports.txt"
# Public repository: tools/../play.*. Research lab: output/rewrite/tools/../../../play.*.
LAUNCHER_DIR = next(d for d in (HERE.parent, HERE.parents[2]) if (d / "play.sh").is_file())
PWSH = shutil.which("pwsh")
PWSH_IMAGE = os.environ.get("MAGICKA_TEST_PWSH_IMAGE")
ENGINE = os.environ.get("MAGICKA_TEST_ENGINE", "podman")
SEP = "\x1f"

# Records "program<SEP>arg<SEP>arg..." per call, creates one output file per
# distinct call (ignoring --missing/--manifest) and lists it in --manifest.
FAKE_PROGRAM = r"""#!/usr/bin/env bash
log=%(log)s
{ printf '%%s' "${0##*/}"; for a in "$@"; do printf '\x1f%%s' "$a"; done; echo; } >> "$log"
manifest= key=()
while [ $# -gt 0 ]; do
  case "$1" in
    --manifest) manifest=$2; shift 2 ;;
    --missing) shift ;;
    *) key+=("$1"); shift ;;
  esac
done
out=%(outputs)s/$(printf '%%s|' "${key[@]}" | md5sum | cut -c1-16)
mkdir -p "${out%%/*}"
: > "$out"
if [ -n "$manifest" ]; then echo "$out" > "$manifest"; fi
"""


def make_game(folder: Path) -> Path:
    content = folder / "Content"
    (content / "Levels").mkdir(parents=True)
    return content


def export_steps(text):
    """(name, version, legacy, program, args) per step of asset-exports.txt."""
    for line in text.splitlines():
        fields = line.split()
        if fields and not fields[0].startswith("#"):
            yield fields[0], fields[1], fields[2], fields[3], fields[4:]


class PlayScriptCases:
    script = None  # set by the concrete classes below

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        (self.repo / "tools").mkdir(parents=True)
        shutil.copy(LAUNCHER_DIR / self.script, self.repo / self.script)
        shutil.copy(EXPORTS, self.repo / "tools" / "asset-exports.txt")
        (self.repo / "Cargo.toml").write_text("[workspace]\n")
        self.home = root / "home"
        self.home.mkdir()
        self.root = root

    def tearDown(self):
        self.tmp.cleanup()

    def steam_root(self, relative=".local/share/Steam", libraries=()):
        steam = self.home / relative
        (steam / "steamapps").mkdir(parents=True, exist_ok=True)
        entries = "".join(
            f'\t"{i + 1}"\n\t{{\n\t\t"path"\t\t"{lib}"\n\t\t"label"\t\t""\n\t}}\n'
            for i, lib in enumerate(libraries)
        )
        (steam / "steamapps" / "libraryfolders.vdf").write_text(
            f'"libraryfolders"\n{{\n\t"0"\n\t{{\n\t\t"path"\t\t"{steam}"\n\t}}\n{entries}}}\n'
        )
        return steam

    def command(self, args, environment):
        raise NotImplementedError

    def run_play(self, stdin="", interactive=True, env=None, args=("--print-content",)):
        environment = {"HOME": str(self.home), "LANG": "C"}
        if interactive:
            environment["MAGICKA_PLAY_INTERACTIVE"] = "1"
        environment.update(env or {})
        command, process_env = self.command(list(args), environment)
        return subprocess.run(
            command, input=stdin, capture_output=True, text=True, env=process_env, timeout=120,
        )

    def env_file(self):
        path = self.repo / ".env"
        return path.read_text() if path.exists() else None

    def assert_found(self, result, content):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(content))

    # --- precedence -------------------------------------------------------

    def test_environment_wins_over_env_file_and_steam(self):
        env_game = make_game(self.root / "env game")
        file_game = make_game(self.root / "file game")
        self.steam_root()
        make_game(self.home / ".local/share/Steam/steamapps/common/Magicka")
        (self.repo / ".env").write_text(f'MAGICKA_CONTENT="{file_game}"\n')
        result = self.run_play(env={"MAGICKA_CONTENT": str(env_game)})
        self.assert_found(result, env_game)

    def test_env_file_is_used_without_prompting(self):
        game = make_game(self.root / "saved game")
        (self.repo / ".env").write_text(f"# mine\nOTHER=1\nMAGICKA_CONTENT='{game}'\n")
        result = self.run_play(stdin="")
        self.assert_found(result, game)
        self.assertNotIn("?", result.stderr)

    def test_linked_magicka_folder_is_used_before_steam(self):
        game = make_game(self.repo / "Magicka")
        self.steam_root()
        make_game(self.home / ".local/share/Steam/steamapps/common/Magicka")
        self.assert_found(self.run_play(), game)

    def test_env_file_pointing_nowhere_is_an_error(self):
        (self.repo / ".env").write_text('MAGICKA_CONTENT="/nonexistent/Magicka/Content"\n')
        result = self.run_play()
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing folder", result.stderr)

    def test_env_file_is_parsed_not_executed(self):
        marker = self.root / "pwned"
        game = make_game(self.root / "game")
        (self.repo / ".env").write_text(
            f"touch {marker}\nMAGICKA_CACHE=$(touch {marker})\nPATH=/nowhere\n"
            f'MAGICKA_CONTENT="{game}"\n'
        )
        self.assert_found(self.run_play(), game)
        self.assertFalse(marker.exists())

    # --- Steam discovery --------------------------------------------------

    def test_steam_game_in_secondary_library_is_found_used_and_saved(self):
        library = self.root / "Games Drive/SteamLibrary"
        game = make_game(library / "steamapps/common/Magicka")
        self.steam_root(libraries=[library])
        result = self.run_play(stdin="y\ny\n")
        self.assert_found(result, game)
        self.assertIn("Found Magicka in Steam", result.stderr)
        self.assertEqual(self.env_file(), f'MAGICKA_CONTENT="{game}"\n')
        # Later runs use .env and ask nothing, even with Steam itself gone.
        import shutil
        shutil.rmtree(self.home / ".local/share/Steam")
        again = self.run_play(stdin="", interactive=True)
        self.assert_found(again, game)
        self.assertNotIn("Found Magicka", again.stderr)

    def test_enter_accepts_defaults(self):
        game = make_game(self.home / ".local/share/Steam/steamapps/common/Magicka")
        self.steam_root()
        self.assert_found(self.run_play(stdin="\n\n"), game)
        self.assertIsNotNone(self.env_file())

    def test_declining_to_save_leaves_no_env_file(self):
        game = make_game(self.home / ".local/share/Steam/steamapps/common/Magicka")
        self.steam_root()
        self.assert_found(self.run_play(stdin="y\nn\n"), game)
        self.assertIsNone(self.env_file())

    def test_saving_keeps_other_settings_and_replaces_old_content(self):
        game = make_game(self.home / ".local/share/Steam/steamapps/common/Magicka")
        self.steam_root()
        (self.repo / ".env").write_text("MAGICKA_FFMPEG=/opt/ffmpeg\n")
        self.assert_found(self.run_play(stdin="y\ny\n"), game)
        self.assertEqual(
            self.env_file(), f'MAGICKA_FFMPEG=/opt/ffmpeg\nMAGICKA_CONTENT="{game}"\n'
        )

    def test_flatpak_steam_is_searched(self):
        game = make_game(
            self.home / ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/Magicka"
        )
        self.steam_root(".var/app/com.valvesoftware.Steam/.local/share/Steam")
        self.assert_found(self.run_play(stdin="y\nn\n"), game)

    def test_same_library_reached_through_two_roots_is_offered_once(self):
        steam = self.steam_root()
        make_game(steam / "steamapps/common/Magicka")
        (self.home / ".steam").mkdir()
        (self.home / ".steam/steam").symlink_to(steam)
        result = self.run_play(stdin="y\nn\n")
        self.assertEqual(result.stderr.count("Found Magicka in Steam"), 1)

    def test_declined_steam_install_falls_back_to_typed_path(self):
        make_game(self.home / ".local/share/Steam/steamapps/common/Magicka")
        self.steam_root()
        other = make_game(self.root / "My Games/Magicka")
        result = self.run_play(stdin=f"n\n{self.root}/nowhere\n{other.parent}\ny\n")
        self.assert_found(result, other)
        self.assertIn("No Magicka Content folder", result.stderr)
        self.assertEqual(self.env_file(), f'MAGICKA_CONTENT="{other}"\n')

    def test_typed_path_may_be_the_content_folder_or_use_tilde(self):
        game = make_game(self.home / "games/Magicka")
        self.assert_found(self.run_play(stdin=f"{game}\nn\n"), game)
        self.assert_found(self.run_play(stdin="~/games/Magicka\nn\n"), game)

    def test_empty_answer_to_path_prompt_quits(self):
        result = self.run_play(stdin="\n")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")

    # --- non-interactive --------------------------------------------------

    def test_non_interactive_uses_steam_without_writing_env(self):
        game = make_game(self.home / ".local/share/Steam/steamapps/common/Magicka")
        self.steam_root()
        result = self.run_play(interactive=False)
        self.assert_found(result, game)
        self.assertIn("run interactively to save", result.stderr)
        self.assertIsNone(self.env_file())

    def test_non_interactive_without_game_explains_options(self):
        result = self.run_play(interactive=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("MAGICKA_CONTENT", result.stderr)
        self.assertIn("Steam", result.stderr)

    # --- asset exports and launching ---------------------------------------

    def install_fakes(self):
        self.content = make_game(self.repo / "Magicka")
        self.log = self.root / "calls.log"
        outputs = self.root / "outputs"
        binaries = self.repo / "target" / "release"
        binaries.mkdir(parents=True)
        programs = {step[3] for step in export_steps(EXPORTS.read_text())}
        for name in programs | {"magicka-viewer", "magicka-server"}:
            path = binaries / name
            path.write_text(FAKE_PROGRAM % {"log": f"'{self.log}'", "outputs": f"'{outputs}'"})
            path.chmod(0o755)

    def calls(self):
        if not self.log.exists():
            return []
        calls = [line.split(SEP) for line in self.log.read_text().splitlines()]
        self.log.unlink()
        return calls

    def expected_exports(self, cache, missing=(), text=None):
        calls = []
        for name, version, _legacy, program, args in export_steps(text or EXPORTS.read_text()):
            args = [a.replace("{content}", str(self.content)).replace("{cache}", str(cache)) for a in args]
            if name in missing:
                args.append("--missing")
            calls.append([program, *args, "--manifest", str(cache / "exports" / f"{name}.txt.tmp")])
        return calls

    def export(self, env=None):
        result = self.run_play(interactive=False, env=env, args=["--skip-build", "--export-only"])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("asset cache up to date", result.stdout)
        return result

    def test_first_run_exports_every_step_in_order_and_records_versions(self):
        self.install_fakes()
        cache = self.repo / ".cache"
        self.export()
        self.assertEqual(self.calls(), self.expected_exports(cache))
        for name, version, *_ in export_steps(EXPORTS.read_text()):
            manifest = (cache / "exports" / f"{name}.txt").read_text().splitlines()
            self.assertEqual(manifest[0], version)
            self.assertTrue(Path(manifest[1]).exists())
            self.assertFalse((cache / "exports" / f"{name}.txt.tmp").exists())

    def test_complete_cache_runs_nothing_and_missing_output_reexports_only_that_step(self):
        self.install_fakes()
        cache = self.repo / ".cache"
        self.export()
        self.calls()
        self.export()
        self.assertEqual(self.calls(), [])
        levels = (cache / "exports" / "levels.txt").read_text().splitlines()[1]
        Path(levels).unlink()
        self.export()
        expected = [c for c in self.expected_exports(cache, missing={"levels"}) if c[0] == "level-gltf"]
        self.assertEqual(self.calls(), expected)

    def test_version_bump_reexports_that_step_in_full(self):
        self.install_fakes()
        cache = self.repo / ".cache"
        self.export()
        self.calls()
        lines = (self.repo / "tools/asset-exports.txt").read_text()
        bumped = lines.replace("\ncharacters 6 ", "\ncharacters 99 ")
        self.assertNotEqual(bumped, lines)
        (self.repo / "tools/asset-exports.txt").write_text(bumped)
        self.export()
        expected = [c for c in self.expected_exports(cache, text=bumped) if c[0] == "character-gltf"]
        self.assertEqual(self.calls(), expected)
        self.assertEqual((cache / "exports/characters.txt").read_text().splitlines()[0], "99")

    def test_caches_from_before_manifests_are_completed_not_rebuilt(self):
        self.install_fakes()
        cache = self.repo / ".cache"
        (cache / "textures").mkdir(parents=True)
        (cache / "gltf/Models/Missiles").mkdir(parents=True)
        (cache / "gltf/.level-export-version").write_text("7\n")
        (cache / "gltf/.character-export-version").write_text("5\n")  # outdated
        self.export()
        # textures (1) and levels (7) match their current versions and are completed;
        # missiles' marker means version 1 but the step is now 2, so it is redone.
        self.assertEqual(self.calls(), self.expected_exports(cache, missing={"textures", "levels"}))

    def test_cache_path_with_spaces_and_ampersand(self):
        self.install_fakes()
        cache = self.root / "my cache & more"
        self.export(env={"MAGICKA_CACHE": str(cache)})
        self.assertEqual(self.calls(), self.expected_exports(cache))

    def test_menu_host_and_server_launch_with_forwarded_options(self):
        self.install_fakes()
        cache = self.repo / ".cache"
        self.export()
        self.calls()
        mods = str(self.repo / "mods")
        ok = lambda r: self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        ok(self.run_play(interactive=False, args=["--skip-build", "--debug-menu"]))
        self.assertEqual(self.calls(), [[
            "magicka-viewer", "--menu", str(self.content), "--root", str(cache),
            "--mods", mods, "--debug-menu"]])
        ok(self.run_play(interactive=False, args=[
            "--skip-build", "--host", "0.0.0.0:17701", "Wizard_Castle.lvl", "--mods", "boosted", "--name", "A B"]))
        self.assertEqual(self.calls(), [[
            "magicka-viewer", "--play", str(self.content), "--root", str(cache),
            "--level", "Wizard_Castle.lvl", "--mods", mods, "--enable", "boosted",
            "--name", "A B", "--network-host", "0.0.0.0:17701"]])
        ok(self.run_play(interactive=False, args=["--skip-build", "--server", "--no-mods", "--lobby", "players=8"]))
        self.assertEqual(self.calls(), [[
            "magicka-server", "--content", str(self.content), "--level",
            str(self.repo / "scenarios" / "feature-lab.lvl"), "--bind", "127.0.0.1:17700",
            "--lobby", "players=8"]])

    def test_join_requires_an_address_and_modes_are_exclusive(self):
        self.install_fakes()
        result = self.run_play(interactive=False, args=["--skip-build", "--join"])
        self.assertEqual(result.returncode, 2)
        self.assertIn("--join requires an IP:port", result.stderr)
        result = self.run_play(interactive=False, args=["--skip-build", "--host", "--server"])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.calls(), [])


class Bash(PlayScriptCases, unittest.TestCase):
    script = "play.sh"

    def command(self, args, environment):
        return ["bash", str(self.repo / self.script), *args], {**environment, "PATH": os.environ["PATH"]}


@unittest.skipUnless(PWSH or PWSH_IMAGE, "no pwsh on PATH and MAGICKA_TEST_PWSH_IMAGE unset")
class PowerShell(PlayScriptCases, unittest.TestCase):
    script = "play.ps1"

    def command(self, args, environment):
        script = str(self.repo / self.script)
        if PWSH:
            return [PWSH, "-NoProfile", "-File", script, *args], {**environment, "PATH": os.environ["PATH"]}
        container = [ENGINE, "run", "--rm", "-i", "--security-opt", "label=disable",
                     "-v", f"{self.root}:{self.root}"]
        for key, value in environment.items():
            container += ["-e", f"{key}={value}"]
        return [*container, PWSH_IMAGE, "pwsh", "-NoProfile", "-File", script, *args], dict(os.environ)


if __name__ == "__main__":
    unittest.main(verbosity=2)
