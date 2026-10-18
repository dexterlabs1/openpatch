#!/usr/bin/env python3
"""Release archive and user-space Linux upgrade contracts (no builds/network)."""
import argparse
import os
import pathlib
import subprocess
import tarfile
import tempfile
import unittest
import zipfile

import package_release


class ReleasePackageTests(unittest.TestCase):
    def package(self, temporary, platform):
        base = pathlib.Path(temporary)
        binaries = base / "prebuilt binaries"
        binaries.mkdir()
        suffix = ".exe" if platform == "windows-x86_64" else ""
        for name in ("openpatch-desktop", "openpatch"):
            (binaries / (name + suffix)).write_bytes((name + " version one").encode())
        args = argparse.Namespace(platform=platform, binary_dir=str(binaries),
                                  output=str(base / "release output"),
                                  build_number="10", source="https://example.test/source/revision")
        return package_release.package(args)

    def test_windows_archive_retains_both_executable_names_and_licenses(self):
        with tempfile.TemporaryDirectory() as temporary:
            archive = self.package(temporary, "windows-x86_64")
            self.assertEqual(archive.name, "OpenPatch-windows-x86_64.zip")
            with zipfile.ZipFile(archive) as package:
                prefix = "OpenPatch-windows-x86_64/"
                for name in ("openpatch.exe", "openpatch-desktop.exe", "Licenses/LICENSE-GPL-3.0.txt",
                             "Licenses/LICENSE-THIRD-PARTY", "Licenses/OFL-Barlow.txt"):
                    self.assertIn(prefix + name, package.namelist())
                self.assertIn(b"https://example.test/source/revision", package.read(prefix + "Licenses/SOURCE.txt"))
                self.assertTrue(any(name.startswith(prefix + "examples/") for name in package.namelist()))

    @unittest.skipUnless(os.name == "posix", "Linux installer requires POSIX tools")
    def test_linux_install_updates_binaries_preserves_user_files_with_spaces(self):
        with tempfile.TemporaryDirectory(prefix="openpatch package ") as temporary:
            archive = self.package(temporary, "linux-x86_64")
            self.assertEqual(archive.name, "OpenPatch-linux-x86_64.tar.gz")
            extract = pathlib.Path(temporary) / "extracted package"
            extract.mkdir()
            with tarfile.open(archive) as package:
                self.assertTrue(all(member.name.startswith("OpenPatch-linux-x86_64") for member in package))
                package.extractall(extract, filter="data")
            folder = extract / "OpenPatch-linux-x86_64"
            self.assertTrue(os.access(folder / "install.sh", os.X_OK))
            home = pathlib.Path(temporary) / "user home"
            destination = home / ".local/share/openpatch"
            destination.mkdir(parents=True)
            patch = destination / "my patch.json"
            patch.write_text("user patch")
            settings = home / ".config/openpatch/settings.json"
            settings.parent.mkdir(parents=True)
            settings.write_text("user settings")
            environment = dict(os.environ, HOME=str(home))
            for expected in (b"openpatch-desktop version one", b"updated desktop"):
                (folder / "openpatch-desktop").write_bytes(expected)
                subprocess.run(["sh", str(folder / "install.sh")], env=environment,
                               check=True, capture_output=True)
                self.assertEqual((destination / "openpatch-desktop").read_bytes(), expected)
                self.assertEqual(patch.read_text(), "user patch")
                self.assertEqual(settings.read_text(), "user settings")
            desktop = (home / ".local/share/applications/openpatch.desktop").read_text()
            self.assertIn(f'Exec="{destination}/openpatch-desktop"', desktop)
            self.assertEqual((home / ".local/bin/openpatch").resolve(), (destination / "openpatch").resolve())
            self.assertTrue(os.access(home / ".local/bin/openpatch-update", os.X_OK))


if __name__ == "__main__":
    unittest.main()
