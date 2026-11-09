"""Optional real torch check using synthetic weights, no network or trained model.

Run with a Transformers environment and a folder from create_fixture.py.
"""
import importlib.util
import sys
import threading
from pathlib import Path

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

spec = importlib.util.spec_from_file_location("codebot_runtime", Path(__file__).parents[1] / "runtime.py")
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)
torch.set_num_threads(1)
folder = sys.argv[1]
model = AutoModelForCausalLM.from_pretrained(folder, local_files_only=True, trust_remote_code=False)
tokenizer = AutoTokenizer.from_pretrained(folder, local_files_only=True, trust_remote_code=False)
encoded = tokenizer("hello local model", return_tensors="pt")

# Cancellation must stop generation, release its thread, and allow another turn.
original = model.generate
generated = []


def count_generated(**kwargs):
    tokens = original(**kwargs)
    generated.append(tokens.shape[-1] - encoded["input_ids"].shape[-1])
    return tokens


model.generate = count_generated
before = set(threading.enumerate())
received = []


def disconnect(raw):
    received.append(raw)
    if raw.startswith(b"data:"):
        raise BrokenPipeError("test disconnect")


try:
    runtime.generate_stream(model, tokenizer, torch, encoded, 200, disconnect)
    raise AssertionError("expected a disconnect")
except BrokenPipeError:
    pass
assert received and 0 < generated[0] < 200, generated
assert set(threading.enumerate()) == before, "owned generation thread leaked"
events = []
runtime.generate_stream(model, tokenizer, torch, encoded, 8, events.append)
assert generated[-1] == 8
assert any(b'"content"' in event for event in events)
assert events[-1].endswith(b"data: [DONE]\n\n")
assert set(threading.enumerate()) == before
print("Real torch streaming, disconnect cancellation and next generation passed.")
