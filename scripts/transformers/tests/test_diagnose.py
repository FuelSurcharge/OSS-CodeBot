import builtins
import contextlib
import importlib.metadata
import io
import json
from pathlib import Path
import runpy
import types
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).parents[1] / "diagnose.py"


class DiagnoseTests(unittest.TestCase):
    def run_probe(self, importer, version):
        output = io.StringIO()
        with patch("builtins.__import__", side_effect=importer), patch("importlib.metadata.version", side_effect=version), contextlib.redirect_stdout(output):
            runpy.run_path(str(SCRIPT))
        return json.loads(output.getvalue())

    def test_missing_packages_and_import_failures_do_not_echo_exception_values(self):
        real_import = builtins.__import__

        def importer(name, *args, **kwargs):
            if name in ["torch", "transformers", "accelerate", "safetensors", "huggingface_hub"]:
                raise ImportError("private-test-token in exception")
            return real_import(name, *args, **kwargs)

        def version(name):
            if name == "torch":
                return "2.6.0"
            raise importlib.metadata.PackageNotFoundError(name)

        result = self.run_probe(importer, version)
        self.assertEqual(result["packages"]["torch"], "2.6.0")
        self.assertIsNone(result["packages"]["transformers"])
        self.assertFalse(any(result["devices"].values()))
        self.assertFalse(any(result["imports"].values()))
        self.assertNotIn("private-test-token", json.dumps(result))

    def test_devices_require_successful_allocation(self):
        real_import = builtins.__import__

        def empty(*args, device):
            if device == "cuda":
                raise RuntimeError("private-test-device-failure")

        torch = types.SimpleNamespace(empty=empty, cuda=types.SimpleNamespace(is_available=lambda: True),
            backends=types.SimpleNamespace(mps=types.SimpleNamespace(is_available=lambda: True)),
            mps=types.SimpleNamespace(recommended_max_memory=lambda: 1000))

        def importer(name, *args, **kwargs):
            if name == "torch":
                return torch
            if name in ["transformers", "accelerate", "safetensors", "huggingface_hub"]:
                return types.SimpleNamespace()
            return real_import(name, *args, **kwargs)

        result = self.run_probe(importer, lambda name: "2.6.0")
        self.assertEqual(result["devices"], {"cpu": True, "cuda": False, "mps": True})
        self.assertEqual(result["mps_recommended"], 1000)
        self.assertIsNone(result["cuda_total"])
        self.assertNotIn("private-test-device-failure", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
