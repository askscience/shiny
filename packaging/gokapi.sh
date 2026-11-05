# Gokapi release bundled with the liteaura package (sourced by scripts/build-deb.sh).
#
# Gokapi (https://github.com/Forceu/Gokapi) is AGPL-3.0. The package ships the
# unmodified upstream binaries in /usr/libexec/shiny/gokapi/, its license and a
# NOTICE in /usr/share/doc/liteaura/gokapi/, and the Corresponding Source next to
# the .deb on the same GitHub Release (AGPL-3.0 §6(d)).
#
# The two binaries come from different upstream commits, read from their
# embedded `vcs.revision`:
#   gokapi 2.2.4     -> tag v2.2.4      (9a4b289c604198c05589e34314e889d2c25386d8)
#   gokapi-cli 1.1.3 -> commit 311b6bef (311b6bef626318ed0e8f00d4fec2cbabb9335f63)
# so each gets its own source archive.
#
# plugins/files/src/share/mod.rs pins the same versions and hashes (its tests
# read this file to keep the two in sync). Bump both together, and re-check the
# headless bootstrap in share/server.rs against the new server sources.

GOKAPI_VERSION=2.2.4
GOKAPI_CLI_VERSION=1.1.3
GOKAPI_RELEASE_URL=https://github.com/Forceu/Gokapi/releases/download/v2.2.4

GOKAPI_SERVER_ZIP=gokapi-2.2.4_linux-amd64.zip
GOKAPI_SERVER_SHA256=51dbf724c94f8afa0b40fb1ef63a1fe418e7996e7b9536e1f0a5da695b8e18ef
GOKAPI_CLI_ZIP=gokapi-cli-1.1.3_linux-amd64.zip
GOKAPI_CLI_SHA256=5bfa76c1dbfc9c9d4e4dcca535b7942db71672e3e06336bf6bdfebe654225f64

GOKAPI_SERVER_REV=9a4b289c604198c05589e34314e889d2c25386d8
GOKAPI_SERVER_SRC_URL=https://github.com/Forceu/Gokapi/archive/refs/tags/v2.2.4.tar.gz
GOKAPI_SERVER_SRC_SHA256=de4ec69692818d0310c42a8091ae8fec38ba2c6199160f672434d860f3e18394
GOKAPI_CLI_REV=311b6bef626318ed0e8f00d4fec2cbabb9335f63
GOKAPI_CLI_SRC_URL=https://github.com/Forceu/Gokapi/archive/311b6bef626318ed0e8f00d4fec2cbabb9335f63.tar.gz
GOKAPI_CLI_SRC_SHA256=79bea04a8e28b4b7a07d2f8089ca08ac8dfb557d7c62e941af9a69d06b21d6bf
