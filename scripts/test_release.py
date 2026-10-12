"""Regression tests for release validation and package contents (no Rust build)."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent


def manifest(root, version="0.1.0"):
    (root / "core").mkdir(exist_ok=True)
    (root / "ui").mkdir(exist_ok=True)
    (root / "core/Cargo.toml").write_text(f'[workspace.package]\nversion = "{version}"\n')
    (root / "ui/Cargo.toml").write_text(f'[package]\nversion = "{version}"\n')


class ReleaseValidation(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        manifest(self.root)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("add", ".")
        self.git("commit", "-m", "fixture")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.git("tag", "v0.1.0")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, stderr=subprocess.DEVNULL)

    def validate(self, tag):
        return subprocess.run(
            [sys.executable, str(SCRIPTS / "validate-release.py"), tag],
            cwd=self.root, env={**os.environ, "GITHUB_OUTPUT": str(self.root / "output")},
            text=True, capture_output=True,
        )

    def test_matching_tag(self):
        result = self.validate("v0.1.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("tag=v0.1.0\nsha=", (self.root / "output").read_text())

    def test_rejects_invalid_tag(self):
        self.assertNotEqual(self.validate("v0.1.0; echo injected").returncode, 0)

    def test_rejects_version_mismatch(self):
        self.git("tag", "v2.0.0")
        self.assertNotEqual(self.validate("v2.0.0").returncode, 0)

    def test_rejects_wrong_checkout(self):
        self.git("commit", "--allow-empty", "-m", "new head")
        self.assertNotEqual(self.validate("v0.1.0").returncode, 0)

    def test_rejects_commit_outside_main(self):
        self.git("commit", "--allow-empty", "-m", "unmerged")
        self.git("tag", "v0.1.1")
        manifest(self.root, "0.1.1")
        self.assertNotEqual(self.validate("v0.1.1").returncode, 0)


class LinuxPackage(unittest.TestCase):
    def test_archive_contains_binary_notices_and_matching_checksum(self):
        spec = importlib.util.spec_from_file_location("package_linux", SCRIPTS / "package-linux.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest(root)
            target = root / "target/release"
            target.mkdir(parents=True)
            (target / "minmark").write_bytes(b"test executable")
            (target / "minmark").chmod(0o755)
            for filename in ["LICENSE", "NOTICE", "README.md", "BUILDING.md", "CHANGELOG.md"]:
                (root / filename).write_text(filename)
            dep = root / "font-dependency"
            dep.mkdir()
            (dep / "NOTICE").write_text("font license and copyright")
            metadata = {"packages": [{"name": "typst-assets", "version": "0.13.1",
                        "source": "registry", "manifest_path": str(dep / "Cargo.toml"),
                        "license": "Apache-2.0"}]}
            with patch.object(module, "ROOT", root), \
                 patch.dict(os.environ, {"CARGO_TARGET_DIR": str(root / "target")}), \
                 patch.object(module.platform, "system", return_value="Linux"), \
                 patch.object(module.platform, "machine", return_value="x86_64"), \
                 patch.object(module.subprocess, "check_output", side_effect=["libc found\n", json.dumps(metadata)]):
                module.main()
            archive = root / "dist/minmark-0.1.0-linux-x86_64.tar.gz"
            self.assertEqual((root / "dist/SHA256SUMS").read_text(),
                             f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
            with tarfile.open(archive) as tar:
                prefix = "minmark-0.1.0-linux-x86_64/"
                self.assertEqual(tar.getmember(prefix + "minmark").mode & 0o111, 0o111)
                self.assertEqual(tar.extractfile(prefix + "third-party/typst-assets-0.13.1/NOTICE").read(),
                                 b"font license and copyright")
                self.assertIn(prefix + "INSTALL.txt", tar.getnames())


if __name__ == "__main__":
    unittest.main()
