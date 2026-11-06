"""Publish the reviewed docs/tasks/*.md using the existing GitHub CLI identity.

Default: preview only. --apply: check all existing issues, then create missing tasks.
No credentials are read or stored by this script.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile


REPOSITORY = "FuelSurcharge/OSS-CodeBot"
TASK_DIR = Path(__file__).resolve().parents[1] / "docs" / "tasks"
MARKER = re.compile(r"<!-- codebot-task:([A-Z0-9-]+) -->")


def load_tasks():
    tasks = []
    for path in sorted(TASK_DIR.glob("*.md")):
        if path.name == "README.md":
            continue
        title, _, body = path.read_text(encoding="utf-8").partition("\n")
        marker = MARKER.search(body)
        if not title.startswith("# [") or not marker or path.stem != marker[1]:
            raise ValueError(f"Invalid task document: {path.name}")
        task_id = marker[1]
        if not title.startswith(f"# [{task_id}] "):
            raise ValueError(f"Title ID differs from marker: {path.name}")
        dependency_line = next(line for line in body.splitlines() if line.startswith("- 依存: "))
        dependencies = [] if dependency_line.startswith("- 依存: なし") else re.findall(
            r"\b(?:CB-\d{3}|FUT-(?:OS|DRV)-\d{3})\b", dependency_line)
        tasks.append({"id": task_id, "title": title[2:], "body": body.strip(), "dependencies": dependencies})
    ids = {task["id"] for task in tasks}
    if len(ids) != len(tasks) or not tasks:
        raise ValueError("Task IDs must be unique and nonempty")
    for task in tasks:
        if any(dep not in ids or dep == task["id"] for dep in task["dependencies"]):
            raise ValueError(f"Invalid dependency in {task['id']}")
    return tasks


def github(*arguments):
    result = subprocess.run(["gh", "api", *arguments], text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "GitHub API request failed")
    return json.loads(result.stdout)


def publish(tasks):
    # A failed read stops the operation before any Issue is created.
    existing = []
    page_number = 1
    while True:
        page = github(f"repos/{REPOSITORY}/issues?state=all&per_page=100&page={page_number}")
        if not isinstance(page, list):
            raise ValueError("Unexpected response while listing Issues")
        existing.extend(issue for issue in page if "pull_request" not in issue)
        if len(page) < 100:
            break
        page_number += 1
    mapped = {}
    for task in tasks:
        matches = [issue for issue in existing if (
            f"<!-- codebot-task:{task['id']} -->" in (issue.get("body") or "")
            or issue["title"].startswith(f"[{task['id']}] "))]
        if len(matches) > 1:
            raise ValueError(f"Multiple existing Issues for {task['id']}; resolve before publishing")
        if matches:
            mapped[task["id"]] = matches[0]["html_url"]
    pending = {task["id"]: task for task in tasks if task["id"] not in mapped}
    # Create dependencies first, including FUT-OS before FUT-DRV.
    while pending:
        ready = next((task for task in pending.values()
                      if all(dep in mapped for dep in task["dependencies"])), None)
        if ready is None:
            raise ValueError("Cyclic task dependencies")
        body = ready["body"]
        if ready["dependencies"]:
            body += "\n\n依存Issue:\n\n" + "\n".join(
                f"- {mapped[dep]}" for dep in ready["dependencies"])
        body += "\n\n作成根拠: 実装基準コミット " + (
            "https://github.com/FuelSurcharge/OSS-CodeBot/commit/8647f96"
            " とリポジトリ内の設計書・ロードマップ。\n")
        payload = {"title": ready["title"], "body": body}
        with tempfile.TemporaryDirectory(prefix="codebot-issues-") as directory:
            payload_path = Path(directory) / "issue.json"
            payload_path.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
            # Never retry a POST automatically: a lost response can hide success.
            issue = github("--method", "POST", f"repos/{REPOSITORY}/issues", "--input", str(payload_path))
        mapped[ready["id"]] = issue["html_url"]
        print(f"CREATED {ready['id']} {issue['html_url']}", flush=True)
        del pending[ready["id"]]
    for task in tasks:
        print(f"{task['id']} {mapped[task['id']]}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Create missing GitHub Issues")
    args = parser.parse_args()
    try:
        tasks = load_tasks()
        if args.apply:
            publish(tasks)
        else:
            for task in tasks:
                print(task["title"])
            print(f"Preview: {len(tasks)} tasks. No network requests or changes made.")
    except (OSError, ValueError, RuntimeError, StopIteration) as error:
        print(f"Stopped: {error}", file=sys.stderr)
        print("No automatic retry was performed. Check existing Issues before retrying.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
