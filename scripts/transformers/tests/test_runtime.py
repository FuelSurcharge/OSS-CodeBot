import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("codebot_runtime", Path(__file__).parents[1] / "runtime.py")
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class RuntimeTests(unittest.TestCase):
    def test_multi_turn_validation_and_template_free_transcript(self):
        messages = [{"role":"user", "content":"first"}, {"role":"assistant", "content":"answer"}, {"role":"user", "content":"second"}]
        self.assertTrue(runtime.valid_messages(messages))
        self.assertIn("assistant: answer", runtime.plain_chat(messages))
        self.assertIn("user: first", runtime.plain_chat(messages))
        self.assertTrue(runtime.plain_chat(messages).endswith("assistant:"))
        for invalid in [[], messages[:2], [None], [{"role":"system","content":"instructions"}], [{"role":"user","content":" "}]]:
            self.assertFalse(runtime.valid_messages(invalid))

    def test_only_data_assets_are_selected(self):
        for name in ["config.json", "model-00001-of-00002.safetensors", "tokenizer.json", "vocab.txt", "chat_template.jinja"]:
            self.assertTrue(runtime.selected_file(name))
        for name in ["model.bin", "modeling_custom.py", "tokenizer_custom.py", "tokenizer/model.py", "tokenizer.bin", "../config.json", "/config.json", "..\\config.json"]:
            self.assertFalse(runtime.selected_file(name))

    def test_model_index_cannot_reference_missing_or_external_shards(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            (folder / "config.json").write_text('{"model_type":"gpt2"}')
            (folder / "model.safetensors").write_bytes(b"test")
            for shard in ["missing.safetensors", "../model.safetensors", "/model.safetensors"]:
                (folder / "model.safetensors.index.json").write_text(json.dumps({"weight_map": {"weight": shard}}))
                with self.assertRaises(runtime.RuntimeFailure):
                    runtime.validate_folder(folder)

    def test_file_hash_mismatch_never_passes(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "model.safetensors"
            path.write_bytes(b"checkpoint data")
            entry = types.SimpleNamespace(size=path.stat().st_size, lfs=types.SimpleNamespace(sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
            runtime.verify_file(path, entry)
            path.write_bytes(b"corrupted data!")
            with self.assertRaises(runtime.RuntimeFailure):
                runtime.verify_file(path, entry)

    def test_snapshot_download_is_pinned_verified_and_does_not_report_tokens(self):
        with tempfile.TemporaryDirectory() as root:
            folder = Path(root)
            files = {"config.json": b'{"model_type":"gpt2"}', "model.safetensors": b"fake checkpoint", "tokenizer.json": b"{}"}
            entries = [types.SimpleNamespace(rfilename=name, size=len(data), lfs=types.SimpleNamespace(sha256=hashlib.sha256(data).hexdigest())) for name, data in files.items()]
            entries += [types.SimpleNamespace(rfilename="modeling_custom.py", size=100, lfs=None)]
            calls = []
            class Api:
                def __init__(self, **kwargs):
                    self.options = kwargs
                def model_info(self, repo, **kwargs):
                    self_outer.assertEqual(repo, "owner/model")
                    self_outer.assertTrue(kwargs["files_metadata"])
                    return types.SimpleNamespace(sha="verified-commit", siblings=entries)
            def download(repo, name, **kwargs):
                calls.append((name, kwargs))
                path = folder / name
                path.write_bytes(files[name])
                return str(path)
            self_outer = self
            fake = types.SimpleNamespace(HfApi=Api, hf_hub_download=download)
            options = {"repo": "owner/model", "revision": "main", "cache": root, "max_bytes": 1000, "token": "test-private-token"}
            output = io.StringIO()
            with patch.dict(sys.modules, {"huggingface_hub": fake}), patch("sys.stdin", io.StringIO(json.dumps(options))), contextlib.redirect_stdout(output):
                runtime.download()
            self.assertEqual(len(calls), 3)
            self.assertTrue(all(kwargs["revision"] == "verified-commit" for _, kwargs in calls))
            self.assertNotIn("test-private-token", output.getvalue())
            self.assertIn('"event": "done"', output.getvalue())


if __name__ == "__main__":
    unittest.main()
