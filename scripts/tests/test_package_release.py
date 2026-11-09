import copy
import importlib.util
import json
from pathlib import Path
import plistlib
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("package_release", Path(__file__).parents[1] / "package_release.py")
package_release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package_release)


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "project"
        self.root.mkdir()
        files = {"Cargo.toml": '[workspace.package]\nversion = "0.1.0"\n',
                 "Cargo.lock": "pinned dependencies\n", "about.toml": 'accepted = ["MIT"]\n',
                 "readme.md": "Read me", "CONTRIBUTING.md": "Contributing",
                 "docs/RELEASING.md": "Release instructions",
                 "config/api.example.toml": 'provider = "openai"',
                 "config/local.toml": "SECRET-MUST-NOT-SHIP",
                 "scripts/build_sample_plugin.py": "sample builder",
                 "assets/OFL.txt": "Noto terms", "assets/fonts/line-seed-jp/OFL.txt": "LINE terms",
                 "assets/fonts/biz-udp-gothic/OFL.txt": "BIZ terms"}
        for name in ("runtime.py", "diagnose.py", "requirements.txt"):
            files["scripts/transformers/" + name] = "runtime source"
        for name in ("manifest.json", "guest.rs", "README.md", "plugin.wasm"):
            files["plugins/input-lines/" + name] = "sample source"
        for name, content in files.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        dependency = Path(self.temp.name) / "dependency-source"
        dependency.mkdir()
        (dependency / "Cargo.toml").write_text("package metadata")
        (dependency / "LICENSE").write_text("Mozilla Public License")
        (dependency / "src").mkdir()
        (dependency / "src/lib.rs").write_text("// MPL source available with the binary")
        self.metadata = {"workspace_members": ["codebot"], "packages": [
            {"id": "codebot", "source": None, "license": None},
            {"id": "dependency", "name": "dependency", "version": "1.0.0", "source": "registry", "license": "MPL-2.0",
             "manifest_path": str(dependency / "Cargo.toml")}]}
        self.report = {"crates": [{"license": "MPL-2.0", "package": {"name": "dependency", "version": "1.0.0"}}],
                       "licenses": [{"name": "Mozilla Public License", "text": "<notice> & copyright",
                                     "used_by": [{"crate": {"name": "dependency", "version": "1.0.0"}}]}]}

    def build(self, target, development=True, profile="release", out="out", report=None, git_dirty=""):
        binaries = Path(self.temp.name) / "binaries" / target
        binaries.mkdir(parents=True, exist_ok=True)
        platform, machine = package_release.TARGETS[target]
        data = bytearray(256)
        if platform == "linux":
            data[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<H", data, 18, machine)
        elif platform == "macos":
            data[:4] = b"\xcf\xfa\xed\xfe"
            struct.pack_into("<I", data, 4, machine)
        else:
            data[:2] = b"MZ"
            struct.pack_into("<I", data, 60, 128)
            data[128:132] = b"PE\0\0"
            struct.pack_into("<H", data, 132, machine)
        suffix = ".exe" if platform == "windows" else ""
        for name in ("codebot-cli", "codebot-gui"):
            (binaries / (name + suffix)).write_bytes(data)
        if report is None:
            report = copy.deepcopy(self.report)
            report["codebot_report"] = package_release.notice_binding(self.root, target)
        with patch.object(package_release.subprocess, "check_output", side_effect=["a" * 40 + "\n", git_dirty]):
            return package_release.package(self.root, binaries, Path(self.temp.name) / out,
                                           target, profile, development, report, self.metadata)

    def unpack(self, archive):
        folder = Path(self.temp.name) / ("unpacked-" + archive.name)
        folder.mkdir()
        if archive.name.endswith(".tar.gz"):
            with tarfile.open(archive) as stream:
                stream.extractall(folder, filter="data")
        else:
            with zipfile.ZipFile(archive) as stream:
                stream.extractall(folder)
        return next(folder.iterdir())

    def test_all_platform_layouts_checksums_sources_and_secret_exclusion(self):
        for target, (platform, _) in package_release.TARGETS.items():
            with self.subTest(target=target):
                archive = self.build(target, out=target)
                self.assertTrue(archive.with_name(archive.name + ".sha256").read_text().startswith(package_release.sha256(archive)))
                folder = self.unpack(archive)
                resources = folder / "CodeBot.app/Contents/Resources" if platform == "macos" else folder
                self.assertTrue((resources / "config/api.example.toml").is_file())
                self.assertFalse((resources / "config/local.toml").exists())
                self.assertTrue((resources / "licenses/crates/dependency-1.0.0/src/lib.rs").is_file())
                self.assertIn("&lt;notice&gt; &amp; copyright", (resources / "THIRD_PARTY.html").read_text())
                self.assertTrue((resources / "LICENSE-STATUS.txt").is_file())
                manifest = json.loads((folder / "package-manifest.json").read_text())
                for relative, expected in manifest["files"].items():
                    self.assertEqual(package_release.sha256(folder / relative), expected)
                self.assertNotIn(str(self.root), (folder / "package-manifest.json").read_text())
                if platform == "macos":
                    with (folder / "CodeBot.app/Contents/Info.plist").open("rb") as stream:
                        self.assertEqual(plistlib.load(stream)["CFBundleExecutable"], "codebot-gui")
                    self.assertTrue((folder / "CodeBot.app/Contents/MacOS/codebot-cli").is_file())
                if platform == "linux":
                    self.assertTrue((folder / "CodeBot.sh").stat().st_mode & 0o111)
                    self.assertTrue((folder / "bin/codebot-cli").stat().st_mode & 0o111)

    def test_license_gate_never_picks_a_license_and_public_archives_require_release(self):
        target = "x86_64-unknown-linux-gnu"
        with self.assertRaisesRegex(ValueError, "license is undecided"):
            self.build(target, development=False)
        self.assertFalse((Path(self.temp.name) / "out").exists())
        (self.root / "LICENSE").write_text("An owner-selected license. " * 10)
        self.metadata["packages"][0]["license"] = "MIT"
        with self.assertRaisesRegex(ValueError, "profile release"):
            self.build(target, development=False, profile="debug")
        with self.assertRaisesRegex(ValueError, "Commit the reviewed sources"):
            self.build(target, development=False, git_dirty=" M readme.md")
        archive = self.build(target, development=False)
        folder = self.unpack(archive)
        self.assertTrue((folder / "LICENSE").is_file())
        self.assertFalse((folder / "LICENSE-STATUS.txt").exists())

    def test_stale_or_wrong_target_reports_and_unknown_licenses_fail_without_artifacts(self):
        target = "x86_64-unknown-linux-gnu"
        report = copy.deepcopy(self.report)
        report["codebot_report"] = package_release.notice_binding(self.root, target)
        (self.root / "Cargo.lock").write_text("changed dependencies")
        with self.assertRaisesRegex(ValueError, "fresh notices"):
            self.build(target, report=report)
        report["codebot_report"] = package_release.notice_binding(self.root, "aarch64-apple-darwin")
        with self.assertRaisesRegex(ValueError, "fresh notices"):
            self.build(target, report=report)
        report["codebot_report"] = package_release.notice_binding(self.root, target)
        report["crates"][0]["license"] = "Unknown"
        with self.assertRaisesRegex(ValueError, "every dependency"):
            self.build(target, report=report)
        self.assertEqual(list((Path(self.temp.name) / "out").iterdir()), [])

    def test_existing_artifacts_and_mismatched_binary_architectures_are_preserved(self):
        target = "x86_64-unknown-linux-gnu"
        archive = self.build(target)
        before = package_release.sha256(archive)
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.build(target)
        self.assertEqual(before, package_release.sha256(archive))
        linux = Path(self.temp.name) / "binaries" / target / "codebot-cli"
        for other in ("x86_64-pc-windows-msvc", "aarch64-apple-darwin"):
            with self.assertRaisesRegex(ValueError, "architecture"):
                package_release.check_binary(linux, other)
        linux.write_bytes(b"not an executable")
        with self.assertRaisesRegex(ValueError, "architecture"):
            package_release.check_binary(linux, target)


if __name__ == "__main__":
    unittest.main()
