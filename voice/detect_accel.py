#!/usr/bin/env python3
"""Pick the best available accelerator for a local speech sidecar.

Usage:
    python3 voice/detect_accel.py supertonic   # ONNX Runtime execution providers
    python3 voice/detect_accel.py whisper      # CTranslate2 device/compute type
    python3 voice/detect_accel.py ggml         # ggml device name (qwentts.cpp)

The first two print shell ``export`` lines so a launcher can ``eval`` them; the
``ggml`` mode prints a bare device name (``Vulkan0`` / ``CUDA0`` / ``CPU``).

Diagnostics go to stderr. The requested priority everywhere is
**Vulkan → CUDA → CPU**, but each runtime supports a different subset:

  * qwentts.cpp / ggml   → Vulkan, CUDA, Metal, CPU  (a real Vulkan backend)
  * Supertonic / ORT     → CUDA / ROCm / OpenVINO, CPU (no Vulkan EP)
  * faster-whisper / CT2 → CUDA, CPU (no Vulkan backend)

So Vulkan is only actually usable by qwentts.cpp today; the others report why
they cannot take it.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys


def _run(cmd: list[str], timeout: float = 10.0) -> str:
    try:
        out = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            timeout=timeout,
            text=True,
        )
    except Exception:
        return ""
    return out.stdout if out.returncode == 0 else ""


def _vulkan_lines() -> list[str]:
    """Raw ``deviceName`` lines from vulkaninfo, in device order."""
    if not shutil.which("vulkaninfo"):
        return []
    text = _run(["vulkaninfo", "--summary"])
    names: list[str] = []
    for line in text.splitlines():
        if "deviceName" in line:
            names.append(line.split("=", 1)[-1].strip())
    return names


def vulkan_devices() -> list[str]:
    """Real (non-software) Vulkan devices, if any."""
    return [
        n
        for n in _vulkan_lines()
        if not any(s in n.lower() for s in ("llvmpipe", "lavapipe", "swiftshader", "software"))
    ]


def nvidia_gpus() -> list[str]:
    if not shutil.which("nvidia-smi"):
        return []
    return [l.strip() for l in _run(["nvidia-smi", "-L"]).splitlines() if l.strip()]


def _emit(pairs: dict[str, str]) -> None:
    for key, value in pairs.items():
        print(f"export {key}={_quote(value)}")


def _quote(value: str) -> str:
    return "'" + value.replace("'", "'\\''") + "'"


def supertonic() -> None:
    vulkan = vulkan_devices()
    nvidia = nvidia_gpus()

    try:
        import onnxruntime as ort  # type: ignore

        available = ort.get_available_providers()
    except Exception as exc:  # pragma: no cover - only when ORT is missing
        print(f"detect_accel: onnxruntime not importable ({exc}); using CPU", file=sys.stderr)
        available = []

    print(f"detect_accel: onnxruntime providers = {available}", file=sys.stderr)
    if vulkan:
        print(
            f"detect_accel: Vulkan present ({', '.join(vulkan)}) but ONNX Runtime "
            "has no Vulkan execution provider — using another backend",
            file=sys.stderr,
        )

    providers: list[str] = []
    if nvidia and "CUDAExecutionProvider" in available:
        providers.append("CUDAExecutionProvider")
    elif "ROCMExecutionProvider" in available:
        providers.append("ROCMExecutionProvider")
        if "MIGraphXExecutionProvider" in available:
            providers.append("MIGraphXExecutionProvider")
    elif "OpenVINOExecutionProvider" in available:
        providers.append("OpenVINOExecutionProvider")

    providers.append("CPUExecutionProvider")
    print(f"detect_accel: supertonic backend = {providers[0]}", file=sys.stderr)
    _emit({"SUPERTONIC_ONNX_PROVIDERS": ",".join(providers)})


def whisper() -> None:
    vulkan = vulkan_devices()
    nvidia = nvidia_gpus()

    requested = (os.environ.get("WHISPER_DEVICE") or "auto").strip().lower() or "auto"
    compute = (os.environ.get("WHISPER_COMPUTE_TYPE") or "").strip()

    device = requested
    if requested == "auto":
        device = "cpu"
        try:
            import ctranslate2  # type: ignore

            count = ctranslate2.get_cuda_device_count()
            if count > 0:
                device = "cuda"
            print(f"detect_accel: ctranslate2 CUDA devices = {count}", file=sys.stderr)
        except Exception as exc:
            print(f"detect_accel: ctranslate2 probe failed ({exc}); using CPU", file=sys.stderr)

    if vulkan:
        print(
            f"detect_accel: Vulkan present ({', '.join(vulkan)}) but CTranslate2 has "
            "no Vulkan backend — whisper.cpp (GGML_VULKAN) would be required",
            file=sys.stderr,
        )

    if not compute:
        compute = "float16" if device == "cuda" else "int8"
    print(f"detect_accel: whisper device = {device} ({compute})", file=sys.stderr)
    _emit({"WHISPER_DEVICE": device, "WHISPER_COMPUTE_TYPE": compute})


def ggml() -> None:
    """Pick a ggml device name for qwentts.cpp.

    Preference: an NVIDIA CUDA device, then a Vulkan device, then CPU.

    Notes on the Vulkan choice:
      * Device order matches ``vulkaninfo`` (and ggml's own enumeration), so the
        first real device maps to ``Vulkan0``.
      * Mesa's RADV on some AMD parts is unstable with ggml's fp16 compute
        shaders (a GPU ring timeout during synthesis). That is handled by the
        launcher disabling fp16 compute, not by skipping the GPU — an AMD GPU
        is still far faster than CPU. An Intel iGPU is *slower* than the CPU
        fallback here, so it is passed over when a discrete GPU exists.
    """
    vulkan = vulkan_devices()
    nvidia = nvidia_gpus()

    if nvidia:
        print(f"detect_accel: NVIDIA GPU(s) {nvidia} and CUDA → CUDA0", file=sys.stderr)
        print("CUDA0")
        return

    if vulkan:
        # Prefer a discrete GPU; an Intel iGPU is slower than these CPU kernels.
        chosen = 0
        for i, name in enumerate(vulkan):
            if any(s in name.lower() for s in ("radeon", "nvidia", "arc", "discrete")):
                chosen = i
                break
        print(
            f"detect_accel: Vulkan devices {vulkan} → Vulkan{chosen}",
            file=sys.stderr,
        )
        print(f"Vulkan{chosen}")
        return

    print("detect_accel: no usable CUDA/Vulkan device → CPU", file=sys.stderr)
    print("CPU")


def main() -> int:
    mode = sys.argv[1] if len(sys.argv) == 2 else ""
    if mode == "supertonic":
        supertonic()
    elif mode == "whisper":
        whisper()
    elif mode == "ggml":
        ggml()
    else:
        print("usage: detect_accel.py {supertonic|whisper|ggml}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
