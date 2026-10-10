#!/bin/sh
# shiny-install-user.sh — decide which local account a Shiny install belongs to.
#
# The three installers (install-kiosk-greeter.sh, install-linux-auth.sh,
# install-linux-session.sh) all need the same answer: "which human account owns
# this deployment?". They used to each default to the literal "eev", which is
# correct on exactly one machine. On a distro build, or on any account with a
# different name, every one of them silently picked the wrong user: the auth
# allow-list named a uid that does not exist, the per-user server was installed
# into the wrong home, and the greeter's profile migration touched nothing.
# Those failures are quiet — nothing errors, the seat just never works.
#
# So the default is *derived*, never hardcoded. Resolution order:
#
#   1. $SHINY_USER (or $SHINY_SERVER_USER) — explicit override, always wins.
#   2. $SUDO_USER — who ran `sudo scripts/install-…`. This is the normal case
#      for a developer or an admin installing on their own machine.
#   3. The owner of the repository directory. Correct for a distro package
#      installed into a shared location owned by the service account, and the
#      same answer as (2) for a per-user checkout.
#   4. The owner of $HOME when the install is not run through sudo.
#   5. Otherwise: fail loudly, naming the override. Guessing here would pick
#      root and hand the whole deployment to uid 0.
#
# Only ever prints the account name on stdout. Diagnostics go to stderr so the
# caller can use it unfiltered:
#
#   SHINY_USER=$(. "$(dirname -- "$0")/shiny-install-user.sh")
#
# Prints nothing and exits 1 when it cannot decide; the callers already check
# `id "$SHINY_USER"` and give a good error, so there is no reason to guess.
set -u

# Where the repository lives. Callers may set SHINY_REPO first.
: "${SHINY_REPO:=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}"

emit() { printf '%s\n' "$1"; }

# 1. Explicit override. Normalise SERVER_USER onto the same name so the three
# installers cannot disagree about which variable they honour.
if [ -n "${SHINY_USER:-}" ]; then
    emit "$SHINY_USER"
    exit 0
fi
if [ -n "${SHINY_SERVER_USER:-}" ]; then
    emit "$SHINY_SERVER_USER"
    exit 0
fi

# 2. Whoever ran sudo. Absent when the installer is run as root directly.
if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ] && id "$SUDO_USER" >/dev/null 2>&1; then
    emit "$SUDO_USER"
    exit 0
fi

# 3. The account that owns the repo. `stat` rather than a `ls` parse.
owner=$(stat -c %U "$SHINY_REPO" 2>/dev/null || true)
if [ -n "$owner" ] && [ "$owner" != "root" ] && [ "$owner" != "nobody" ] && id "$owner" >/dev/null 2>&1; then
    emit "$owner"
    exit 0
fi

# 4. Not run through sudo and there is a real home to go by.
if [ "$(id -u)" -ne 0 ] && [ -n "${HOME:-}" ] && [ -d "${HOME}" ]; then
    emit "$(id -un)"
    exit 0
fi

# 5. Nothing to go on. Say why rather than defaulting to a name that only
#    exists on the machine this was written on.
echo "shiny-install-user: cannot tell which local account this install belongs to." >&2
echo "  Running as root with no SUDO_USER, and $SHINY_REPO is not owned by a" >&2
echo "  normal user account." >&2
echo "  Set it explicitly:" >&2
echo "    SHINY_USER=<name> sudo $SHINY_REPO/scripts/install-kiosk-greeter.sh" >&2
exit 1