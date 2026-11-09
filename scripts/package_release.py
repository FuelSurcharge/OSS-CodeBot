"""Build portable archives from already-built binaries (Python 3.11+, no extra modules)."""
import argparse
import hashlib
import html
import json
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

TARGETS = {
    "x86_64-unknown-linux-gnu": ("linux", 62),
    "x86_64-pc-windows-msvc": ("windows", 0x8664),
    "x86_64-apple-darwin": ("macos", 0x01000007),
    "aarch64-apple-darwin": ("macos", 0x0100000C),
}


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def notice_binding(root, target):
    inputs = [root / "Cargo.lock", root / "Cargo.toml", root / "about.toml",
              *sorted((root / "crates").glob("*/Cargo.toml"))]
    return {"target": target, "generator": "cargo-about 0.9.3",
            "inputs_sha256": {path.relative_to(root).as_posix(): sha256(path) for path in inputs}}


def check_binary(path, target):
    if not path.is_file() or path.is_symlink():
        raise ValueError(f"Missing regular binary: {path}")
    with path.open("rb") as stream:
        header = stream.read(64)
        platform, machine = TARGETS[target]
        if platform == "linux":
            valid = (header[:6] == b"\x7fELF\x02\x01" and len(header) >= 20
                     and struct.unpack_from("<H", header, 18)[0] == machine)
        elif platform == "macos":
            valid = (header[:4] == b"\xcf\xfa\xed\xfe" and len(header) >= 8
                     and struct.unpack_from("<I", header, 4)[0] == machine)
        else:
            valid = False
            if header[:2] == b"MZ" and len(header) == 64:
                offset = struct.unpack_from("<I", header, 60)[0]
                if 64 <= offset <= 16 * 1024 * 1024:
                    stream.seek(offset)
                    pe = stream.read(6)
                    valid = pe[:4] == b"PE\0\0" and len(pe) == 6 and struct.unpack_from("<H", pe, 4)[0] == machine
        if not valid:
            raise ValueError(f"Binary architecture does not match {target}: {path}")


def copy_file(source, destination):
    if source.is_symlink() or not source.is_file():
        raise ValueError(f"Expected a regular source file: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o644)


def write_notices(report, destination):
    if not report.get("licenses") or not report.get("crates"):
        raise ValueError("The cargo-about report has no dependency licenses")
    for item in report["crates"]:
        if item.get("license") in (None, "Unknown", "Ignore"):
            raise ValueError("Resolve every dependency license before packaging")
    sections = ["<!doctype html><html lang='en'><meta charset='utf-8'><title>Third-party notices</title>",
                "<h1>Third-party notices</h1><p>Generated from Cargo.lock with cargo-about. "
                "Additional font terms and original license files are in licenses/.</p>"]
    for license in report["licenses"]:
        if not license.get("text", "").strip():
            raise ValueError("A dependency license has no text")
        sections.append("<h2>" + html.escape(license["name"]) + "</h2><ul>")
        for user in license["used_by"]:
            crate = user["crate"]
            sections.append("<li>" + html.escape(f"{crate['name']} {crate['version']}") + "</li>")
        sections.extend(["</ul><pre>", html.escape(license["text"]), "</pre>"])
    sections.append("</html>")
    destination.write_text("\n".join(sections), encoding="utf-8")


def dependency_material(metadata, destination):
    """Preserve copyright/notice files and sources of dependencies requiring MPL notices."""
    for package in metadata["packages"]:
        if not package.get("source"):
            continue
        folder = Path(package["manifest_path"]).parent
        name = f"{package['name']}-{package['version']}"
        for path in sorted(folder.rglob("*")):
            relative = path.relative_to(folder)
            if not path.is_file() or path.is_symlink():
                continue
            license_file = (len(relative.parts) <= 3 and path.name.lower().startswith(
                ("license", "licence", "copying", "notice", "copyright", "unlicense", "ofl")))
            # egui's embedded fonts have additional terms in UFL.txt/Hack-Regular.txt/etc.
            font_terms = package["name"] == "epaint_default_fonts" and path.suffix == ".txt"
            mpl_source = "MPL-2.0" in (package.get("license") or "") and not any(
                part.startswith(".") for part in relative.parts)
            if license_file or font_terms or mpl_source:
                copy_file(path, destination / name / relative)


def package(root, binaries, out, target, profile, development, report, metadata):
    platform, _ = TARGETS[target]
    version = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]
    if not development:
        members = set(metadata["workspace_members"])
        if (not (root / "LICENSE").is_file() or (root / "LICENSE").is_symlink()
                or len((root / "LICENSE").read_text(encoding="utf-8").strip()) < 50
                or any(not (p.get("license") or p.get("license_file"))
                       for p in metadata["packages"] if p["id"] in members)):
            raise ValueError("CodeBot's license is undecided. Set LICENSE and each workspace package's license first; use --development for an internal verification archive.")
        if profile != "release":
            raise ValueError("Distribution archives require --profile release")
    if report.get("codebot_report") != notice_binding(root, target):
        raise ValueError("Collect fresh notices for this target and Cargo.lock with scripts/collect_notices.py")
    suffix = ".exe" if platform == "windows" else ""
    for name in ("codebot-cli", "codebot-gui"):
        check_binary(binaries / (name + suffix), target)
    name = f"CodeBot-{version}{'-development' if development else ''}-{target}"
    extension = ".tar.gz" if platform == "linux" else ".zip"
    out.mkdir(parents=True, exist_ok=True)
    archive = out / (name + extension)
    checksum = out / (archive.name + ".sha256")
    if archive.exists() or checksum.exists():
        raise ValueError("Archive or checksum already exists; choose a new output folder")
    with tempfile.TemporaryDirectory(prefix=".package-", dir=out) as temp:
        stage = Path(temp) / name
        stage.mkdir()
        if platform == "macos":
            contents = stage / "CodeBot.app" / "Contents"
            gui_destination = contents / "MacOS" / "codebot-gui"
            contents.mkdir(parents=True)
            with (contents / "Info.plist").open("wb") as stream:
                plistlib.dump({"CFBundleExecutable": "codebot-gui", "CFBundleName": "CodeBot",
                               "CFBundleIdentifier": "io.github.fuelsurcharge.codebot",
                               "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version,
                               "CFBundleVersion": version, "LSMinimumSystemVersion": "11.0",
                               "NSHighResolutionCapable": True}, stream)
            resources = contents / "Resources"
            resources.mkdir()
        else:
            gui_destination = stage / "bin" / ("codebot-gui" + suffix)
            resources = stage
        for source, destination in [(binaries / ("codebot-gui" + suffix), gui_destination),
                                    (binaries / ("codebot-cli" + suffix), stage / "bin" / ("codebot-cli" + suffix))]:
            copy_file(source, destination)
            destination.chmod(0o755)
        if platform == "macos":
            copy_file(binaries / "codebot-cli", contents / "MacOS" / "codebot-cli")
            (contents / "MacOS" / "codebot-cli").chmod(0o755)
            (stage / "START-HERE.txt").write_text("GUI: CodeBot.app\nCLI: bin/codebot-cli or CodeBot.app/Contents/MacOS/codebot-cli\nDocumentation, dependency notices and licenses: CodeBot.app/Contents/Resources/\nUnsigned verification build; see docs/RELEASING.md before installation.\n", encoding="utf-8")
        if platform == "linux":
            launcher = stage / "CodeBot.sh"
            launcher.write_text('#!/bin/sh\nset -eu\nCODEBOT_PACKAGE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexec "$CODEBOT_PACKAGE_DIR/bin/codebot-gui" "$@"\n', encoding="utf-8")
            launcher.chmod(0o755)
        for source in [root / "readme.md", root / "Cargo.lock", root / "CONTRIBUTING.md"]:
            copy_file(source, resources / source.name)
        for source in (root / "docs").rglob("*.md"):
            copy_file(source, resources / source.relative_to(root))
        for source in (root / "config").glob("*.example.toml"):
            copy_file(source, resources / "config" / source.name)
        for filename in ("runtime.py", "diagnose.py", "requirements.txt"):
            copy_file(root / "scripts" / "transformers" / filename, resources / "scripts" / "transformers" / filename)
        copy_file(root / "scripts" / "build_sample_plugin.py", resources / "scripts" / "build_sample_plugin.py")
        for source in (root / "plugins" / "input-lines").iterdir():
            if source.name in ("manifest.json", "plugin.wasm", "guest.rs", "README.md"):
                copy_file(source, resources / "plugins" / "input-lines" / source.name)
        for source, relative in [(root / "assets" / "OFL.txt", "noto-sans-jp/OFL.txt"),
                                 (root / "assets" / "fonts" / "line-seed-jp" / "OFL.txt", "line-seed-jp/OFL.txt"),
                                 (root / "assets" / "fonts" / "biz-udp-gothic" / "OFL.txt", "biz-udp-gothic/OFL.txt")]:
            copy_file(source, resources / "licenses" / "fonts" / relative)
            copy_file(source, resources / source.relative_to(root))
        dependency_material(metadata, resources / "licenses" / "crates")
        write_notices(report, resources / "THIRD_PARTY.html")
        if development:
            (resources / "LICENSE-STATUS.txt").write_text("INTERNAL VERIFICATION BUILD. CodeBot's own license and distribution terms remain undecided. Not a public release. Dependencies retain their respective license terms.\n", encoding="utf-8")
        else:
            copy_file(root / "LICENSE", resources / "LICENSE")
        if platform != "macos":
            (stage / "START-HERE.txt").write_text(f"GUI: {'CodeBot.sh or bin/codebot-gui' if platform == 'linux' else 'bin/codebot-gui.exe'}\nCLI: bin/codebot-cli{suffix}\nInstructions: docs/RELEASING.md\nDependency notices: THIRD_PARTY.html and licenses/\nNo Rust installation is needed to run these binaries. Local AI needs a separately installed runtime and model.\n", encoding="utf-8")
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, encoding="utf-8").strip()
        source_dirty = bool(subprocess.check_output(["git", "-c", "core.fsmonitor=false", "status", "--porcelain"], cwd=root, encoding="utf-8").strip())
        if source_dirty and not development:
            raise ValueError("Commit the reviewed sources before building distribution archives")
        files = {str(p.relative_to(stage)).replace(os.sep, "/"): sha256(p)
                 for p in sorted(stage.rglob("*")) if p.is_file()}
        (stage / "package-manifest.json").write_text(json.dumps({"version": version, "target": target,
            "profile": profile, "development": development, "commit": commit, "source_dirty": source_dirty, "files": files}, indent=2) + "\n", encoding="utf-8")
        temporary_archive = Path(temp) / ("archive" + extension)
        if platform == "linux":
            with tarfile.open(temporary_archive, "w:gz") as stream:
                stream.add(stage, arcname=name)
        else:
            with zipfile.ZipFile(temporary_archive, "w", zipfile.ZIP_DEFLATED) as stream:
                for path in sorted(stage.rglob("*")):
                    if path.is_file():
                        stream.write(path, path.relative_to(stage.parent))
        digest = sha256(temporary_archive)
        created_archive = False
        try:
            with archive.open("xb") as stream, temporary_archive.open("rb") as source:
                created_archive = True
                shutil.copyfileobj(source, stream)
            with checksum.open("x", encoding="utf-8") as stream:
                stream.write(f"{digest}  {archive.name}\n")
        except Exception:
            if created_archive:
                archive.unlink(missing_ok=True)
            raise
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--profile", choices=("debug", "release"), default="release")
    parser.add_argument("--binaries", type=Path)
    parser.add_argument("--out", type=Path, default=Path("dist"))
    parser.add_argument("--development", action="store_true")
    parser.add_argument("--notices", type=Path, required=True, help="target-bound JSON from scripts/collect_notices.py")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked",
        "--filter-platform", args.target], cwd=root, encoding="utf-8"))
    try:
        result = package(root, args.binaries or root / "target" / args.target / args.profile,
                         args.out, args.target, args.profile, args.development,
                         json.loads(args.notices.read_text(encoding="utf-8")), metadata)
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, f"Packaging failed: {error}\n")
    print(result)


if __name__ == "__main__":
    main()
