#!/bin/sh
# @@NAME@@ — started by the LiteAura server (runtime = "process").
# Written by the plugin builder: the plugin's code is in ../engine/plugin.py.
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 -B -s "$here/../engine/plugin.py"
