"""The LiteAura plugin kit for Python.

A Python plugin is a process the server starts and talks to over a private
Unix socket (HTTP/1.1, protocol version 1). This module is the whole
contract: it serves the handshake, the lifecycle calls, the assistant's tool
calls and the window's route requests, and hands each one to a function of
the plugin.

It uses the standard library only. Do not edit it: a plugin's own code goes
in `engine/plugin.py`.

    from shiny_plugin import Plugin, BadRequest

    plugin = Plugin("notes")

    @plugin.tool("notes_add", step_label="Adding…")
    def notes_add(ctx, params):
        text = require_str(params, "text")
        ...
        return {"id": item_id, "text": text}     # what the assistant reads

    @plugin.route("GET", "/api/notes/items")
    def item_list(ctx, request):
        return {"items": [...]}                  # sent as {"success": true, "data": …}

    plugin.run()
"""

import json
import os
import socket
import socketserver
import sqlite3
import sys
import threading
import traceback
import urllib.parse
from http.server import BaseHTTPRequestHandler

PROTOCOL_MIN = 1
PROTOCOL_MAX = 1
KIT_VERSION = "1.0.0"

_SOCKET_ENV = "SHINY_PLUGIN_SOCKET"
_DIR_ENV = "SHINY_PLUGIN_DIR"
_ROUTE_TAG = "x-shiny-route-tag"
_MAX_BODY = 64 * 1024 * 1024


class PluginError(Exception):
    """An error with a kind the server understands and an HTTP status."""

    kind = "internal"
    status = 500

    def __init__(self, message):
        super().__init__(message)
        self.message = str(message)


class BadRequest(PluginError):
    """The caller sent something wrong (a missing or invalid parameter)."""

    kind = "bad_request"
    status = 400


class NotFound(PluginError):
    """What was asked for does not exist."""

    kind = "not_found"
    status = 404


class Unauthorized(PluginError):
    """The caller may not do this."""

    kind = "unauthorized"
    status = 401


def require_str(params, key):
    """A non-empty string parameter, or `BadRequest`."""
    value = params.get(key) if isinstance(params, dict) else None
    if not isinstance(value, str) or not value.strip():
        raise BadRequest(f"{key} required")
    return value


def log(*parts):
    """Write a line to the server's log (it prefixes the plugin's name)."""
    print(*parts, file=sys.stderr, flush=True)


class Context:
    """What a tool or a route gets about its call.

    `user_id` is the user the call is for: every row a plugin stores belongs
    to one, so always filter by it. `db` is the plugin's connection to the
    application database (its own tables come from `migrations/`).
    """

    def __init__(self, plugin, user_id, os_home=None, agent=None):
        self._plugin = plugin
        self.user_id = user_id
        self.os_home = os_home
        self.agent = agent or {}
        self.config = plugin.config
        self.plugin_dir = plugin.directory

    @property
    def db(self):
        return self._plugin._database()

    def query(self, sql, params=()):
        """Run a SELECT; rows come back as dicts."""
        cursor = self.db.execute(sql, params)
        columns = [column[0] for column in cursor.description or []]
        return [dict(zip(columns, row)) for row in cursor.fetchall()]

    def execute(self, sql, params=()):
        """Run a statement that changes rows; returns how many."""
        cursor = self.db.execute(sql, params)
        self.db.commit()
        return cursor.rowcount


class Request:
    """A request from the plugin's window to one of its routes."""

    def __init__(self, method, target, headers, body):
        parsed = urllib.parse.urlsplit(target)
        self.method = method
        self.path = parsed.path
        self.query = {key: values[-1] for key, values in urllib.parse.parse_qs(parsed.query).items()}
        self.headers = headers
        self.body = body
        self.user_id = headers.get("x-shiny-user-id", "")
        self.os_home = headers.get("x-shiny-os-home") or None
        # Any of these marks a request that did not come from this machine.
        self.remote = any(
            headers.get(name)
            for name in ("x-shiny-remote", "x-forwarded-for", "x-forwarded-proto", "x-forwarded-host")
        )
        try:
            self.path_params = dict(json.loads(headers.get("x-shiny-path-params") or "[]"))
        except (ValueError, TypeError):
            self.path_params = {}

    def json(self):
        """The body as JSON (`{}` when there is none), or `BadRequest`."""
        if not self.body:
            return {}
        try:
            return json.loads(self.body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as error:
            raise BadRequest(f"invalid JSON body: {error}") from None


class Response:
    """A route's answer when it is not plain JSON data (a file, a status)."""

    def __init__(self, body=b"", status=200, content_type="application/octet-stream", headers=None):
        self.body = body.encode("utf-8") if isinstance(body, str) else bytes(body)
        self.status = status
        self.content_type = content_type
        self.headers = dict(headers or {})


class Plugin:
    """A plugin: its tools, its routes, and the loop that serves them."""

    def __init__(self, name, skills_file=None, persona="", context_lines=(), idle_stop=600):
        self.name = name
        self.directory = os.environ.get(_DIR_ENV) or os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        self.skills_file = skills_file or os.path.join("skills", f"{name}.md")
        self.persona = persona
        self.context_lines = list(context_lines)
        # Seconds without a request after which the server may stop the
        # process (it starts again on the next one). None: never, for a
        # plugin whose `on_load` starts work that must keep running.
        self.idle_stop = idle_stop
        self.config = {}
        self._tools = {}
        self._routes = {}
        self._on_load = None
        self._on_unload = None
        self._db = None
        # One call at a time: plugin code never runs concurrently with itself.
        self._lock = threading.Lock()

    # ── what a plugin declares ──────────────────────────────────────────

    def tool(self, name, step_label="Working…", doc=None, aliases=()):
        """Declare an assistant tool: `fn(ctx, params)` returning a dict."""

        def declare(function):
            if name in self._tools:
                raise ValueError(f"tool {name!r} is declared twice")
            self._tools[name] = {
                "function": function,
                "spec": {"name": name, "aliases": list(aliases), "step_label": step_label, "doc_fragment": doc},
            }
            return function

        return declare

    def route(self, method, path, public=False):
        """Declare a route of the window: `fn(ctx, request)`."""

        def declare(function):
            tag = function.__name__
            if tag in self._routes:
                raise ValueError(f"two routes are handled by functions named {tag!r}")
            self._routes[tag] = {
                "function": function,
                "decl": {
                    "method": method.upper(),
                    "path": path,
                    "auth": "public" if public else "auth",
                    "handler_tag": tag,
                },
            }
            return function

        return declare

    def on_load(self, function):
        """Run once when the plugin is loaded (start background work here)."""
        self._on_load = function
        return function

    def on_unload(self, function):
        """Run before the server stops the process."""
        self._on_unload = function
        return function

    # ── the database ────────────────────────────────────────────────────

    def _database(self):
        if self._db is None:
            url = str(self.config.get("database_url") or "")
            path = url.split("://", 1)[-1].split("?", 1)[0] if url else ":memory:"
            # Guarded by the plugin's lock, so sharing it across the
            # connection threads is safe.
            self._db = sqlite3.connect(path, timeout=5, check_same_thread=False)
            self._db.execute("PRAGMA busy_timeout = 5000")
        return self._db

    # ── the protocol ────────────────────────────────────────────────────

    def _hello(self, hello):
        theirs = hello.get("protocol") or {}
        low = max(int(theirs.get("min", 0)), PROTOCOL_MIN)
        high = min(int(theirs.get("max", 0)), PROTOCOL_MAX)
        welcome = {"manifest": {"name": self.name}, "sdk": f"python-{KIT_VERSION}"}
        if low > high:
            welcome["protocol"] = 0
            welcome["error"] = (
                f"no shared protocol version: host speaks {theirs.get('min')}..={theirs.get('max')}, "
                f"plugin {PROTOCOL_MIN}..={PROTOCOL_MAX}"
            )
            return welcome
        self.config = hello.get("config") or {}
        skills = ""
        try:
            with open(os.path.join(self.directory, self.skills_file), encoding="utf-8") as handle:
                skills = handle.read()
        except OSError:
            pass
        welcome.update(
            protocol=high,
            tools=[entry["spec"] for entry in self._tools.values()],
            routes=[entry["decl"] for entry in self._routes.values()],
            skills_md=skills,
            persona=self.persona,
            context_lines=self.context_lines,
            idle_stop_secs=self.idle_stop,
        )
        return welcome

    def _lifecycle(self, function):
        if function is None:
            return {}
        try:
            function(Context(self, ""))
            return {}
        except Exception as error:  # a failing hook must not take the process down
            traceback.print_exc()
            return {"error": str(error)}

    def _call_tool(self, call):
        name = call.get("tool", "")
        entry = self._tools.get(name)
        if entry is None:
            return {"error": {"kind": "not_found", "message": f"no tool named {name}"}}
        ctx = Context(self, call.get("user_id", ""), call.get("os_home"), call.get("context"))
        params = call.get("params")
        try:
            data = entry["function"](ctx, params if isinstance(params, dict) else {})
        except PluginError as error:
            return {"error": {"kind": error.kind, "message": error.message}}
        except Exception as error:
            traceback.print_exc()
            return {"error": {"kind": "internal", "message": f"{name} failed: {error}"}}
        return {"outcome": {"action": name, "result": "ok", "data": {} if data is None else data}}

    def _call_route(self, tag, request):
        entry = self._routes.get(tag or "")
        if entry is None:
            raise NotFound("no such plugin route")
        return entry["function"](Context(self, request.user_id, request.os_home), request)

    def _control(self, path, body):
        try:
            message = json.loads(body.decode("utf-8")) if body else {}
        except (ValueError, UnicodeDecodeError):
            message = {}
        if path == "/_shiny/hello":
            return self._hello(message)
        if path == "/_shiny/load":
            return self._lifecycle(self._on_load)
        if path == "/_shiny/unload":
            return self._lifecycle(self._on_unload)
        if path == "/_shiny/user-registered":
            return {}
        if path == "/_shiny/tool":
            return self._call_tool(message)
        return None

    # ── the loop ────────────────────────────────────────────────────────

    def run(self):
        """Serve until the server goes away. Never returns."""
        path = os.environ.get(_SOCKET_ENV)
        if not path:
            log(f"{_SOCKET_ENV} is not set: this program is started by the LiteAura server")
            sys.exit(2)
        try:
            os.unlink(path)
        except OSError:
            pass
        plugin = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_args):  # the server logs requests itself
                pass

            def address_string(self):
                return "host"

            def _body(self):
                if "chunked" in (self.headers.get("transfer-encoding") or "").lower():
                    chunks = []
                    total = 0
                    while True:
                        size = int(self.rfile.readline().split(b";", 1)[0].strip() or b"0", 16)
                        if size == 0:
                            while self.rfile.readline().strip():
                                pass
                            break
                        total += size
                        if total > _MAX_BODY:
                            raise BadRequest("request body too large")
                        chunks.append(self.rfile.read(size))
                        self.rfile.readline()
                    return b"".join(chunks)
                length = int(self.headers.get("content-length") or 0)
                if length > _MAX_BODY:
                    raise BadRequest("request body too large")
                return self.rfile.read(length) if length else b""

            def _send(self, status, body, content_type="application/json", headers=None):
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                for key, value in (headers or {}).items():
                    self.send_header(key, value)
                self.end_headers()
                self.wfile.write(body)

            def _json(self, status, value):
                self._send(status, json.dumps(value).encode("utf-8"))

            def _fail(self, status, message):
                self._json(status, {"success": False, "data": None, "error": message})

            def _serve(self):
                try:
                    body = self._body()
                except PluginError as error:
                    return self._fail(error.status, error.message)
                path = urllib.parse.urlsplit(self.path).path
                if path.startswith("/_shiny/"):
                    with plugin._lock:
                        answer = plugin._control(path, body)
                    if answer is None:
                        return self._fail(404, "no such control call")
                    return self._json(200, answer)
                headers = {key.lower(): value for key, value in self.headers.items()}
                request = Request(self.command, self.path, headers, body)
                try:
                    with plugin._lock:
                        result = plugin._call_route(headers.get(_ROUTE_TAG), request)
                except PluginError as error:
                    return self._fail(error.status, error.message)
                except Exception:
                    traceback.print_exc()
                    return self._fail(500, "plugin route failed")
                if isinstance(result, Response):
                    return self._send(result.status, result.body, result.content_type, result.headers)
                return self._json(200, {"success": True, "data": {} if result is None else result})

            do_GET = do_POST = do_PUT = do_DELETE = do_PATCH = _serve

        class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
            daemon_threads = True
            allow_reuse_address = True

        server = Server(path, Handler)
        # Only the account running the server may connect.
        os.chmod(path, 0o600)
        self._watch_host(path)
        try:
            server.serve_forever(poll_interval=0.5)
        finally:
            server.server_close()

    def _watch_host(self, path):
        """Exit when the server is gone, or took the socket away."""
        parent = os.getppid()
        try:
            inode = os.stat(path).st_ino
        except OSError:
            inode = None

        def watch():
            import time

            while True:
                time.sleep(2)
                try:
                    current = os.stat(path).st_ino
                except OSError:
                    current = None
                if os.getppid() != parent or current != inode:
                    os._exit(0)

        threading.Thread(target=watch, daemon=True).start()


# Kept importable for a plugin that wants to talk to another local service.
__all__ = [
    "Plugin", "Context", "Request", "Response", "PluginError", "BadRequest", "NotFound",
    "Unauthorized", "require_str", "log", "socket",
]
