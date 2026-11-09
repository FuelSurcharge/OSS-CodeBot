"""Build the reviewed source sample; requires Rust's wasm32-unknown-unknown target."""
import hashlib
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
folder = root / "plugins" / "input-lines"
subprocess.run([
    "rustc", "--edition=2021", "--crate-type=cdylib", "--target=wasm32-unknown-unknown",
    "-C", "opt-level=z", "-C", "panic=abort", "-C", "strip=symbols",
    "--remap-path-prefix", str(root) + "=.",
    str(folder / "guest.rs"), "-o", str(folder / "plugin.wasm"),
], check=True)
(folder / "plugin.wasm").chmod(0o644)
manifest = {
    "manifest_version": 1,
    "api_version": 1,
    "id": "input-lines",
    "name": "入力の行数",
    "version": "0.1.0",
    "description": "渡した入力全体のLFによる行数を表示します。選択したファイルを含む場合も入力全体を数えます。外部通信やファイル変更は行いません。",
    "module": "plugin.wasm",
    "sha256": hashlib.sha256((folder / "plugin.wasm").read_bytes()).hexdigest(),
    "permissions": ["read-selected-files"],
}
(folder / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print("Built input-lines/plugin.wasm and updated its manifest SHA-256")
