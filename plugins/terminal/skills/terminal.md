# Terminal

This plugin adds a **Terminal** window: a real login shell running on the host
machine, connected to the app through a pseudo-terminal (PTY). It is not a
sandbox — commands run as the same user the Shiny server runs as.

## Tools

- `terminal_exec` — Run a shell command and return its output.
  params: `{ command: string }`

The command runs in a persistent shell of its own (not the Terminal window's
session), so successive calls share `cd`/environment state and nothing is ever
typed into a shell the user is working in. Use it for short, non-interactive
commands when the user asks for something on the machine (`ls`, `cat`, `grep`,
`df -h`, `git status`, …). It returns once the shell has been quiet for a
moment; a command still running after ~10 seconds returns the output so far
(`timed_out: true`) and keeps running. Don't use it for interactive programs
(editors, `htop`, REPLs) — open the Terminal window instead so the user can
drive them.

## Window

The window keeps its shell alive while it is closed: reopening the window (or
reloading the page) replays the recent output and reattaches to the same
session. "New shell" starts a fresh session, "Kill session" terminates the
current one.
