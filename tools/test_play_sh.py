#!/usr/bin/env python3
"""Tests for play.sh's game-file discovery: environment, .env, ./Magicka and Steam.

Runs `play.sh --print-content` (no build) in a throwaway public-repository layout
with a fake $HOME holding fake Steam libraries.

    python3 tools/test_play_sh.py
"""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Public repository: tools/../play.sh. Research lab: output/rewrite/tools/../../../play.sh.
PLAY = next(p for p in (HERE.parent / "play.sh", HERE.parents[2] / "play.sh") if p.is_file())


def make_game(folder: Path) -> Path:
    content = folder / "Content"
    (content / "Levels").mkdir(parents=True)
    return content


class PlayShContent(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        (self.repo / "play.sh").write_bytes(PLAY.read_bytes())
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

    def run_play(self, stdin="", interactive=True, env=None):
        environment = {
            "HOME": str(self.home),
            "PATH": os.environ["PATH"],
            "LANG": "C",
        }
        if interactive:
            environment["MAGICKA_PLAY_INTERACTIVE"] = "1"
        environment.update(env or {})
        return subprocess.run(
            ["bash", str(self.repo / "play.sh"), "--print-content"],
            input=stdin, capture_output=True, text=True, env=environment, timeout=30,
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


if __name__ == "__main__":
    unittest.main(verbosity=2)
