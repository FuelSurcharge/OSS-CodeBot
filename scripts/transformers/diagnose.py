"""Read-only, offline probe. Only allowlisted metadata leaves this process."""
import importlib.metadata
import json
import platform
import os

result = {"python": platform.python_version(), "packages": {}, "devices": {"cpu": False, "cuda": False, "mps": False}, "cuda_free": None, "cuda_total": None, "mps_recommended": None}
result.update(system_total=None, system_available=None)
try:
    if os.name == "nt":
        import ctypes
        class MemoryStatus(ctypes.Structure):
            _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [(name, ctypes.c_ulonglong) for name in ("total", "available", "page_total", "page_available", "virtual_total", "virtual_available", "extended")]
        status = MemoryStatus()
        status.length = ctypes.sizeof(status)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            result.update(system_total=status.total, system_available=status.available)
    elif platform.system() == "Darwin":
        import ctypes
        total = ctypes.c_uint64()
        length = ctypes.c_size_t(ctypes.sizeof(total))
        if ctypes.CDLL(None).sysctlbyname(b"hw.memsize", ctypes.byref(total), ctypes.byref(length), None, 0) == 0:
            result["system_total"] = total.value
    else:
        result["system_total"] = os.sysconf("SC_PHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
        if platform.system() == "Linux":
            result["system_available"] = os.sysconf("SC_AVPHYS_PAGES") * os.sysconf("SC_PAGE_SIZE")
except Exception:
    pass
for name in ("torch", "transformers", "accelerate", "safetensors", "huggingface_hub"):
    try:
        result["packages"][name] = importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        result["packages"][name] = None
try:
    import torch
    torch.empty(1, device="cpu")
    result["devices"]["cpu"] = True
    if torch.cuda.is_available():
        try:
            torch.empty(1, device="cuda")
            result["devices"]["cuda"] = True
            result["cuda_free"], result["cuda_total"] = torch.cuda.mem_get_info()
        except Exception:
            pass
    if hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
        try:
            torch.empty(1, device="mps")
            result["devices"]["mps"] = True
            if hasattr(torch.mps, "recommended_max_memory"):
                result["mps_recommended"] = torch.mps.recommended_max_memory()
        except Exception:
            pass
except Exception:
    pass
result["imports"] = {}
for name in ("transformers", "accelerate", "safetensors", "huggingface_hub"):
    try:
        __import__(name)
        result["imports"][name] = True
    except Exception:
        result["imports"][name] = False
print(json.dumps(result))
