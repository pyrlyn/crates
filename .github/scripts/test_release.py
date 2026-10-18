"""Release-helper contracts without a registry, credentials, or a publish."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parent


class ReleaseHelpers(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.metadata = self.root / "metadata.json"
        self.package = {
            "name": "demo",
            "version": "1.2.3",
            "manifest_path": str(self.root / "rust/demo/Cargo.toml"),
            "publish": None,
            "dependencies": [],
        }
        cargo = self.root / "cargo"
        cargo.write_text('#!/bin/sh\ncat "$RELEASE_TEST_METADATA"\n')
        cargo.chmod(0o755)

    def release(self, *args):
        self.metadata.write_text(json.dumps({
            "workspace_root": str(self.root), "packages": [self.package]
        }))
        return subprocess.run(
            ["bash", str(SCRIPTS / "release-crate.sh"), *args],
            env={**os.environ, "PATH": f'{self.root}{os.pathsep}{os.environ["PATH"]}',
                 "RELEASE_TEST_METADATA": str(self.metadata)},
            capture_output=True, text=True, check=False,
        )

    def test_crate_changelog_is_resolved_under_rust(self):
        result = self.release("demo", "1.2.3")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("changelog=rust/demo/CHANGELOG.md\n", result.stdout)
        self.assertIn("version=1.2.3\n", result.stdout)

    def test_unknown_and_invalid_names_are_rejected(self):
        for name in ["missing", "../demo", "demo;exit"]:
            result = self.release(name)
            self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_wrong_tag_version_is_rejected(self):
        result = self.release("demo", "1.2.4")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match", result.stderr)

    def test_unpublishable_package_is_rejected(self):
        self.package["publish"] = []
        result = self.release("demo")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not publishable", result.stderr)

    def test_versionless_runtime_dependency_is_rejected_but_dev_is_allowed(self):
        self.package["dependencies"] = [{
            "name": "helper", "source": None, "path": "/helper", "req": "*",
            "kind": "normal",
        }]
        result = self.release("demo")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("without a version", result.stderr)
        self.package["dependencies"][0]["kind"] = "dev"
        result = self.release("demo")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_changelog_extracts_exact_version_and_prerelease(self):
        text = ("## [Unreleased]\nfuture\n## [1.2.30]\nwrong\n"
                "## [1.2.3-rc.1]\npreview\n## [1.2.3](link)\nreleased\n"
                "## [1.2.2]\nold\n")
        for version, expected in [("1.2.3", "released\n"),
                                  ("1.2.3-rc.1", "preview\n"), ("2.0.0", "")]:
            result = subprocess.run(
                ["bash", str(SCRIPTS / "changelog-section.sh"), version],
                input=text, capture_output=True, text=True, check=True,
            )
            self.assertEqual(result.stdout, expected)


if __name__ == "__main__":
    unittest.main()
