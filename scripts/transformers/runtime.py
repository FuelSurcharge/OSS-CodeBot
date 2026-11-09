"""CodeBot's optional local safetensors runtime. No remote model code is executed."""
import argparse
import fnmatch
import hashlib
import hmac
import json
import queue
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path, PurePosixPath


class RuntimeFailure(Exception):
    def __init__(self, code):
        self.code = code


def emit(event, **fields):
    print(json.dumps({"event": event, **fields}, ensure_ascii=False), flush=True)


def error_code(error):
    if isinstance(error, RuntimeFailure):
        return error.code
    name = type(error).__name__
    if name in ("ImportError", "ModuleNotFoundError"):
        return "dependencies"
    if "Memory" in name:
        return "memory"
    if name in ("ValueError", "KeyError"):
        return "unsupported"
    return "load"


def selected_file(name):
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or "\\" in name:
        return False
    return any(fnmatch.fnmatch(name, pattern) for pattern in (
        "*.safetensors", "*.json", "*.model", "*.tiktoken",
        "vocab.txt", "merges.txt", "chat_template.jinja", "README.md", "LICENSE*",
    ))


def validate_folder(folder):
    folder = Path(folder)
    if not folder.is_dir() or not (folder / "config.json").is_file():
        raise RuntimeFailure("format")
    if not list(folder.glob("*.safetensors")):
        raise RuntimeFailure("format")
    config = json.loads((folder / "config.json").read_text(encoding="utf-8"))
    if not isinstance(config.get("model_type"), str):
        raise RuntimeFailure("format")
    index = folder / "model.safetensors.index.json"
    if index.exists():
        shards = set(json.loads(index.read_text(encoding="utf-8"))["weight_map"].values())
        for shard in shards:
            path = PurePosixPath(shard)
            if path.is_absolute() or ".." in path.parts or "\\" in shard or not (folder / shard).is_file():
                raise RuntimeFailure("format")
    return folder


def verify_file(path, sibling):
    lfs = getattr(sibling, "lfs", None)
    sha256 = getattr(lfs, "sha256", None) if lfs is not None else None
    blob = getattr(sibling, "blob_id", None)
    size = getattr(sibling, "size", None)
    if size is not None and path.stat().st_size != size:
        raise RuntimeFailure("integrity")
    if sha256:
        digest = hashlib.sha256()
    elif blob:
        digest = hashlib.sha1()
        digest.update(f"blob {path.stat().st_size}\0".encode())
    else:
        raise RuntimeFailure("integrity")
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != (sha256 or blob):
        raise RuntimeFailure("integrity")


def download():
    # Read the token from stdin, never argv, a saved config, or an interactive login.
    options = json.load(sys.stdin)
    from huggingface_hub import HfApi, hf_hub_download
    token = options.get("token") or False
    api = HfApi(endpoint="https://huggingface.co", token=token)
    emit("status", message="metadata")
    info = api.model_info(options["repo"], revision=options["revision"], files_metadata=True)
    if not info.sha:
        raise RuntimeFailure("integrity")
    siblings = [s for s in info.siblings if selected_file(s.rfilename)]
    names = {s.rfilename for s in siblings}
    if "config.json" not in names or not any("/" not in n and n.endswith(".safetensors") for n in names):
        raise RuntimeFailure("format")
    if any(s.size is None for s in siblings):
        raise RuntimeFailure("integrity")
    total = sum(s.size for s in siblings)
    if total > options["max_bytes"]:
        raise RuntimeFailure("size")
    received = 0
    root = None
    # Pin every download to the exact commit returned by the metadata request.
    for sibling in siblings:
        emit("progress", received=received, total=total)
        path = Path(hf_hub_download(
            options["repo"], sibling.rfilename, revision=info.sha, token=token,
            cache_dir=options["cache"], endpoint="https://huggingface.co",
        ))
        verify_file(path, sibling)
        if sibling.rfilename == "config.json":
            root = path.parent
        received += sibling.size
    validate_folder(root)
    emit("progress", received=received, total=total)
    emit("done", path=str(root))


def valid_messages(messages):
    return (isinstance(messages, list) and bool(messages) and len(messages) % 2 == 1 and
            all(isinstance(m, dict) and m.get("role") == ("user" if i % 2 == 0 else "assistant")
                and isinstance(m.get("content"), str) and bool(m["content"].strip())
                for i, m in enumerate(messages)))


def plain_chat(messages):
    if len(messages) == 1:
        return messages[0]["content"]
    return "\n\n".join(f'{m["role"]}: {m["content"]}' for m in messages) + "\n\nassistant:"


def generate_stream(model, tokenizer, torch, encoded, budget, write):
    """One owned generation; disconnect signals stopping at the next token step.

    Heartbeats detect a closed client even when the tokenizer buffers text.
    The caller keeps the inference lock until this generation thread has exited.
    """
    from transformers import TextIteratorStreamer, StoppingCriteria, StoppingCriteriaList
    stop = threading.Event()
    errors = []

    class Cancelled(StoppingCriteria):
        def __call__(self, *_args, **_kwargs):
            return stop.is_set()

    streamer = TextIteratorStreamer(tokenizer, skip_prompt=True, skip_special_tokens=True, timeout=0.2)

    def generate():
        try:
            with torch.inference_mode():
                model.generate(**encoded, max_new_tokens=budget, do_sample=False,
                    pad_token_id=tokenizer.pad_token_id if tokenizer.pad_token_id is not None else tokenizer.eos_token_id,
                    streamer=streamer, stopping_criteria=StoppingCriteriaList([Cancelled()]))
        except Exception as error:
            errors.append(error_code(error))
            streamer.end()

    worker = threading.Thread(target=generate, daemon=True)
    worker.start()
    try:
        while True:
            try:
                text = next(streamer)
            except queue.Empty:
                write(b": heartbeat\n\n")
                continue
            except StopIteration:
                break
            if text:
                value = {"choices": [{"delta": {"content": text}, "finish_reason": None}]}
                write(("data: " + json.dumps(value, ensure_ascii=False) + "\n\n").encode())
        if errors:
            write(b'data: {"error":"generation_failed"}\n\n')
        else:
            write(b'data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n')
    finally:
        stop.set()
        worker.join()


def serve(args):
    state = {"status": "loading", "code": None}
    model = tokenizer = torch = None
    inference_lock = threading.Lock()

    def load():
        nonlocal model, tokenizer, torch
        try:
            import torch as torch_module
            from transformers import AutoModelForCausalLM, AutoTokenizer
            torch = torch_module
            torch.set_num_threads(args.threads)
            folder = validate_folder(args.model)
            device = args.device
            if device == "auto":
                device = "cuda" if torch.cuda.is_available() else (
                    "mps" if hasattr(torch.backends, "mps") and torch.backends.mps.is_available() else "cpu")
            if device == "cuda" and not torch.cuda.is_available():
                raise RuntimeFailure("device")
            if device == "mps" and not (hasattr(torch.backends, "mps") and torch.backends.mps.is_available()):
                raise RuntimeFailure("device")
            tokenizer = AutoTokenizer.from_pretrained(folder, local_files_only=True, trust_remote_code=False)
            model = AutoModelForCausalLM.from_pretrained(
                folder, local_files_only=True, trust_remote_code=False,
                use_safetensors=True, torch_dtype="auto",
                device_map="auto" if args.device == "auto" and device == "cuda" else {"": device},
            )
            model.eval()
            state["status"] = "ready"
        except Exception as error:
            state.update(status="error", code=error_code(error))

    class Handler(BaseHTTPRequestHandler):
        def reply(self, status, payload):
            raw = json.dumps(payload, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def do_GET(self):
            if self.path != "/health":
                return self.reply(404, {"error": "not_found"})
            self.reply(200 if state["status"] == "ready" else 503, state)

        def do_POST(self):
            if self.path != "/v1/chat/completions":
                return self.reply(404, {"error": "not_found"})
            if not hmac.compare_digest(self.headers.get("Authorization", ""), "Bearer " + args.api_key):
                return self.reply(401, {"error": "unauthorized"})
            if state["status"] != "ready":
                return self.reply(503, state)
            streaming = False
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if not 0 < size <= 1024 * 1024:
                    return self.reply(413, {"error": "input_size"})
                request = json.loads(self.rfile.read(size))
                messages = request["messages"]
                if not valid_messages(messages):
                    return self.reply(400, {"error": "input_format"})
                budget = request.get("max_tokens", 1024)
                if not isinstance(budget, int) or not 0 < budget < args.ctx_size:
                    return self.reply(400, {"error": "context_limit"})
                with inference_lock, torch.inference_mode():
                    if tokenizer.chat_template:
                        encoded = tokenizer.apply_chat_template(messages, add_generation_prompt=True,
                            tokenize=True, return_tensors="pt", return_dict=True)
                    else:
                        encoded = tokenizer(plain_chat(messages), return_tensors="pt")
                    if encoded["input_ids"].shape[-1] + budget > args.ctx_size:
                        return self.reply(400, {"error": "context_limit"})
                    encoded = {k: v.to(model.device) for k, v in encoded.items()}
                    if request.get("stream") is True:
                        self.send_response(200)
                        self.send_header("Content-Type", "text/event-stream")
                        self.send_header("Cache-Control", "no-cache")
                        self.send_header("Connection", "close")
                        self.end_headers()
                        streaming = True
                        self.close_connection = True

                        def write(raw):
                            self.wfile.write(raw)
                            self.wfile.flush()

                        generate_stream(model, tokenizer, torch, encoded, budget, write)
                        return
                    tokens = model.generate(**encoded, max_new_tokens=budget, do_sample=False,
                        pad_token_id=tokenizer.pad_token_id if tokenizer.pad_token_id is not None else tokenizer.eos_token_id)
                    text = tokenizer.decode(tokens[0, encoded["input_ids"].shape[-1]:], skip_special_tokens=True)
                self.reply(200, {"choices": [{"message": {"role": "assistant", "content": text}}]})
            except (BrokenPipeError, ConnectionResetError):
                pass
            except Exception as error:
                if not streaming:
                    self.reply(500, {"error": error_code(error)})

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.daemon_threads = True
    threading.Thread(target=load, daemon=True).start()
    server.serve_forever()


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("download")
    local = sub.add_parser("serve")
    local.add_argument("--model", required=True)
    local.add_argument("--host", choices=["127.0.0.1"], default="127.0.0.1")
    local.add_argument("--port", type=int, required=True)
    local.add_argument("--api-key", required=True)
    local.add_argument("--alias", default="codebot-local")
    local.add_argument("--ctx-size", type=int, default=4096)
    local.add_argument("--threads", type=int, default=4)
    local.add_argument("--device", choices=["auto", "cpu", "cuda", "mps"], default="auto")
    args = parser.parse_args()
    try:
        if args.command == "download":
            download()
        else:
            serve(args)
    except Exception as error:
        emit("error", code=error_code(error))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
