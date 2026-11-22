### Python engine

This chat builds plugins in Python. The engine is `engine/plugin.py`; the template is the reference. `engine/shiny_plugin.py` is the kit that talks to the server: read it if you need to, never edit it.

- A tool: `@plugin.tool("<name>_verb", step_label="Doing…")` on a function `fn(ctx, params)`. `params` is a dict (`require_str(params, "text")` for a required string, `params.get("limit")` for an optional value). Return a dict: it is what the assistant reads.
- A route: `@plugin.route("GET", "/api/<name>/things")` on a function `fn(ctx, request)`. `request.json()` is the body, `request.query` the query string, `request.path_params` the `:id` parts of the path. Return a dict: the window receives `{"success": true, "data": <dict>}`. Each route needs its own function name.
- Data: `ctx.query(sql, params)` returns rows as dicts, `ctx.execute(sql, params)` changes rows. `ctx.user_id` is the user the call is for: always filter by it.
- Errors: `raise BadRequest("…")`, `raise NotFound("…")`. Anything else is reported as an internal error and printed to the log.
- Only the Python standard library is available: no `pip` packages. Split a long engine into more files in `engine/` and import them by name.
- Plugin code never runs twice at once: the kit serves one call at a time.

- `py_check` — check the engine: Python syntax and standard-library-only imports, each error with its file and line. params: `{}`. When it passes the plugin can be tested and packed; run it again after every change to the engine.
- `py_test` — start the engine as the server would, without installing it, and report whether it starts and which tools and routes it declares. params: `{}`. Run it after `py_check`, before packing.
