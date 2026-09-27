#!/usr/bin/env python3
"""Launch ``supertonic serve`` with an ONNX Runtime execution-provider override.

The upstream CLI hardcodes ``DEFAULT_ONNX_PROVIDERS = ["CPUExecutionProvider"]``
(``supertonic/config.py`` even carries a TODO for the env var), so GPU
acceleration cannot be requested on the command line. This thin wrapper patches
the provider list *before* the model is loaded and then delegates to the real
``supertonic serve`` entrypoint.

Environment:
    SUPERTONIC_ONNX_PROVIDERS   comma-separated ORT providers, most preferred first
                                (set by ``voice/start_supertonic.sh``)
    SUPERTONIC_HOST / SUPERTONIC_PORT / SUPERTONIC_MODEL
"""

from __future__ import annotations

import os
import sys


def _providers() -> list[str]:
    raw = (os.environ.get("SUPERTONIC_ONNX_PROVIDERS") or "").strip()
    return [p.strip() for p in raw.split(",") if p.strip()]


def main() -> int:
    providers = _providers()
    if providers:
        # Both bindings exist: config holds the constant, loader imports it by
        # value at import time — patch the module the loader actually reads.
        from supertonic import config as _config

        _config.DEFAULT_ONNX_PROVIDERS = providers
        try:
            from supertonic import loader as _loader

            _loader.DEFAULT_ONNX_PROVIDERS = providers
        except Exception:  # pragma: no cover - loader layout changed
            pass
        print(f"supertonic_server: requesting ONNX providers {providers}", file=sys.stderr)

    host = os.environ.get("SUPERTONIC_HOST", "127.0.0.1")
    port = str(os.environ.get("SUPERTONIC_PORT", "7788"))
    model = os.environ.get("SUPERTONIC_MODEL", "supertonic-3")

    sys.argv = [
        "supertonic",
        "serve",
        "--host",
        host,
        "--port",
        port,
        "--model",
        model,
    ]

    from supertonic.cli import main as cli_main

    return cli_main()


if __name__ == "__main__":
    raise SystemExit(main())
