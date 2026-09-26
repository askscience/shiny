# Terminal

This plugin adds a **Terminal** window: a real login shell running on the host
machine, connected to the app through a pseudo-terminal (PTY). It is not a
sandbox — commands run as the same user the Shiny server runs as.

The window keeps its shell alive while it is closed: reopening the window (or
reloading the page) replays the recent output and reattaches to the same
session. "New shell" starts a fresh session, "Kill session" terminates the
current one.

The plugin contributes no agent tools and no persona text; the terminal is a
human-facing surface only.
