"""Collect pinned cargo-about notices and bind them to a target and Cargo.lock."""
import argparse
import json
from pathlib import Path
import subprocess
from package_release import TARGETS, notice_binding


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    generator = subprocess.check_output(["cargo", "about", "--version"], encoding="utf-8").strip()
    if generator != "cargo-about 0.9.3":
        parser.exit(1, "Install cargo-about 0.9.3 with --locked --features cli\n")
    binding = notice_binding(root, args.target)
    report = json.loads(subprocess.check_output(["cargo", "about", "generate", "--workspace", "--locked",
        "--target", args.target, "--format", "json", "--fail"], cwd=root, encoding="utf-8"))
    if binding != notice_binding(root, args.target):
        parser.exit(1, "Manifest, Cargo.lock or notice configuration changed while collecting; try again\n")
    report["codebot_report"] = binding
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("x", encoding="utf-8") as stream:
        json.dump(report, stream)
        stream.write("\n")
    print(args.out)


if __name__ == "__main__":
    main()
