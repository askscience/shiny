"""@@TITLE@@ — @@DESCRIPTION@@

The plugin's engine: the assistant's tools and the routes its window calls.
`shiny_plugin.py` (next to this file) is the kit that talks to the server; it
is not edited.

- A tool is a function `fn(ctx, params)` declared with `@plugin.tool`. Its
  name is what the assistant calls and must start with `@@IDENT@@_`. What it
  returns (a dict) is what the assistant reads.
- A route is a function `fn(ctx, request)` declared with `@plugin.route`; its
  path must be under `/api/@@NAME@@/`. What it returns is sent to the window
  as `{"success": true, "data": …}`.
- `ctx.user_id` is the user the call is for. Every row belongs to a user:
  always filter by it. `ctx.query(sql, params)` returns rows as dicts,
  `ctx.execute(sql, params)` changes rows. The table comes from
  `migrations/001_init.sql`.
- Raise `BadRequest("…")` or `NotFound("…")` for a caller's mistake.
- Only the Python standard library is available.
"""

import uuid

from shiny_plugin import BadRequest, Plugin, require_str

plugin = Plugin("@@NAME@@")

MAX_TEXT = 500


def add_item(ctx, text):
    """Add one item for the user and return it."""
    text = text.strip()[:MAX_TEXT]
    if not text:
        raise BadRequest("text required")
    item_id = str(uuid.uuid4())
    ctx.execute(
        "INSERT INTO @@IDENT@@_items (id, user_id, text) VALUES (?, ?, ?)",
        (item_id, ctx.user_id, text),
    )
    return {"id": item_id, "text": text}


def list_items(ctx, limit=50):
    """The user's items, newest first."""
    limit = max(1, min(int(limit), 200))
    return ctx.query(
        "SELECT id, text, created_at FROM @@IDENT@@_items WHERE user_id = ? "
        "ORDER BY created_at DESC, rowid DESC LIMIT ?",
        (ctx.user_id, limit),
    )


@plugin.tool("@@IDENT@@_add", step_label="Adding…")
def tool_add(ctx, params):
    return add_item(ctx, require_str(params, "text"))


@plugin.tool("@@IDENT@@_list", step_label="Reading the list…")
def tool_list(ctx, params):
    items = list_items(ctx, params.get("limit") or 50)
    return {"count": len(items), "items": items}


@plugin.route("GET", "/api/@@NAME@@/items")
def item_list(ctx, request):
    return {"items": list_items(ctx, 200)}


@plugin.route("POST", "/api/@@NAME@@/items")
def item_create(ctx, request):
    body = request.json()
    return {"item": add_item(ctx, str(body.get("text") or ""))}


if __name__ == "__main__":
    plugin.run()
