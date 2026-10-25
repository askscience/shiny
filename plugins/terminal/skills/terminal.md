# Terminal

This plugin adds a **Terminal** window: a real login shell running on the host
machine, connected to the app through a pseudo-terminal (PTY). It is not a
sandbox — commands run as the same user the Shiny server runs as.

## Tools

- `terminal_exec` — Run a shell command and return its output.
  params: `{ command: string }`

The command runs in a persistent shell of its own (never the Terminal window's
shells), so successive calls share `cd`/environment state and nothing is ever
typed into a shell the user is working in. Use it for short, non-interactive
commands when the user asks for something on the machine (`ls`, `cat`, `grep`,
`df -h`, `git status`, …). It returns once the shell has been quiet for a
moment; a command still running after ~10 seconds returns the output so far
(`timed_out: true`) and keeps running. Don't use it for interactive programs
(editors, `htop`, REPLs) — open the Terminal window instead so the user can
drive them.

Every command is shown to the user in the chat with **Allow / Deny / Allow for
this chat** before it runs; the turn waits for that answer. A command that could
destroy the system — wiping root or a system directory, writing raw to a disk,
repartitioning, rebooting, a fork bomb — is refused by a hard-coded guard and
never offered at all, even in a chat where commands are allowed automatically.
When a command is denied or blocked, nothing ran: do not retry it.

## Window

The window keeps its shells alive while it is closed: the tab list lives in
`sessionStorage`, and reopening the window (or reloading the page) replays each
tab's recent output and reattaches to the same session. The tabs sit in the
window's top bar (one shell each); closing a tab terminates its shell, a shell
that exits closes its tab, and closing the last tab opens a fresh one.
