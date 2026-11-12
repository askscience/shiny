"""Serve the Oorbit window outside LiteAura, for previews and screenshots.

    cargo build --release -p oorbit-sim --example preview
    python3 -I plugins/oorbit/scripts/preview-server.py [port] [state-file]

Serves plugins/oorbit/web at /plugins/oorbit/ with the core's web-app
Content-Security-Policy (read from src/api/mod.rs, so a policy change shows up
here too), and a parent page at / that frames it like the shell (`?light=MS`
switches to the light theme after MS milliseconds, `?lang=en` picks the
language). The game's routes run the real simulation through the `preview`
example on a local state file (a fresh planet each run unless one is given).
`POST /preview/skip?hours=N` moves the preview's clock forward.
"""
import http.server
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
WEB = os.environ.get("OORBIT_WEB") or os.path.join(REPO, "plugins", "oorbit", "web")
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8791
STATE = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
    os.environ.get("TMPDIR", "/tmp"), "oorbit-preview-%d.json" % os.getpid())
PREVIEW = os.path.join(REPO, "target", "release", "examples", "preview")
OFFSET_MS = [0]

source = open(os.path.join(REPO, "src", "api", "mod.rs"), encoding="utf-8").read()
match = re.search(r'const WEB_APP_CONTENT_SECURITY_POLICY: &str = "(.*?)";', source, re.S)
CSP = re.sub(r"\\\s*\n\s*", "", match.group(1))

PARENT = b"""<!doctype html><body style="margin:0;background:#000">
<iframe id=f data-shiny-mode="dark" src="/plugins/oorbit/"
 style="width:100vw;height:100vh;border:0;display:block"></iframe>
<script>
const params = new URLSearchParams(location.search);
f.dataset.shinyLocale = params.get('lang') || 'it';
const after = Number(params.get('light') || 0);
if (after) f.addEventListener('load', () => setTimeout(() => f.contentWindow.postMessage(
  {type:'shiny:context', locale: f.dataset.shinyLocale, theme:'dusk', mode:'light'}, location.origin), after));
</script></body>"""


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, ".wasm": "application/wasm"}

    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=WEB, **kwargs)

    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path == "/" or self.path.startswith("/?"):
            self._send(200, "text/html", PARENT)
        elif self.path.startswith("/api/oorbit/ping"):
            body = {"success": True, "data": {"plugin": "oorbit", "version": "preview", "user": "preview"}}
            self._send(200, "application/json", json.dumps(body).encode())
        elif self.path.startswith("/api/oorbit/catalog"):
            self._game("catalog")
        elif self.path.startswith("/api/oorbit/state"):
            self._game("state")
        elif self.path.startswith("/plugins/oorbit/"):
            self.path = self.path[len("/plugins/oorbit"):]
            super().do_GET()
        else:
            self.send_error(404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode() if length else ""
        if self.path.startswith("/preview/skip"):
            hours = float(self.path.split("hours=")[-1]) if "hours=" in self.path else 1.0
            OFFSET_MS[0] += int(hours * 3_600_000)
            self._send(200, "application/json", json.dumps({"offset_hours": OFFSET_MS[0] / 3_600_000}).encode())
        elif self.path.startswith("/api/oorbit/found"):
            self._game("found", json.loads(body or "{}").get("climate", "temperate"))
        elif self.path.startswith("/api/oorbit/action"):
            self._game("action", body)
        elif self.path.startswith("/api/oorbit/reset"):
            self._game("reset")
        else:
            self.send_error(404)

    def _game(self, command, arg=None):
        now = int(time.time() * 1000) + OFFSET_MS[0]
        args = [PREVIEW, STATE, str(now), command] + ([arg] if arg is not None else [])
        out = subprocess.run(args, capture_output=True, text=True).stdout.strip()
        reply = json.loads(out or '{"success": false, "error": "preview binary failed"}')
        self._send(200 if reply.get("success") else 400, "application/json", json.dumps(reply).encode())

    def _send(self, status, kind, body):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.end_headers()
        self.wfile.write(body)

    def end_headers(self):
        if not self.path.startswith("/api") and not self.path.startswith("/?") and self.path != "/":
            self.send_header("Content-Security-Policy", CSP)
            self.send_header("X-Frame-Options", "SAMEORIGIN")
        super().end_headers()


http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
