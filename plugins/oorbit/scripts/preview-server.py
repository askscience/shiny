"""Serve the Oorbit window outside LiteAura, for previews and screenshots.

    python3 -I plugins/oorbit/scripts/preview-server.py [port]

Serves plugins/oorbit/web at /plugins/oorbit/ with the core's web-app
Content-Security-Policy (read from src/api/mod.rs, so a policy change shows up
here too), a parent page at / that frames it like the shell and later sends a
`shiny:context` switching to light mode, and a stub /api/oorbit/ping.
"""
import http.server
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
WEB = os.path.join(REPO, "plugins", "oorbit", "web")
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8791

source = open(os.path.join(REPO, "src", "api", "mod.rs"), encoding="utf-8").read()
match = re.search(r'const WEB_APP_CONTENT_SECURITY_POLICY: &str = "(.*?)";', source, re.S)
CSP = re.sub(r"\\\s*\n\s*", "", match.group(1))

PARENT = b"""<!doctype html><body style="margin:0;background:#000">
<iframe id=f data-shiny-locale="it" data-shiny-mode="dark" src="/plugins/oorbit/"
 style="width:100vw;height:100vh;border:0;display:block"></iframe>
<script>
const params = new URLSearchParams(location.search);
const after = Number(params.get('light') || 0);
if (after) f.addEventListener('load', () => setTimeout(() => f.contentWindow.postMessage(
  {type:'shiny:context', locale:'it', theme:'dusk', mode:'light'}, location.origin), after));
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
        elif self.path.startswith("/plugins/oorbit/"):
            self.path = self.path[len("/plugins/oorbit"):]
            super().do_GET()
        else:
            self.send_error(404)

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
