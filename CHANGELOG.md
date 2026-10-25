# Changelog

## [0.1.1] - 2026-10-25

### Added

- Live image configures apt against the public PEAK'D! repository and registers
  the payload in dpkg, so `apt upgrade` tracks `peakd` from the first boot.
- Native plugin windows follow updates: hidden windows rebuild silently, a
  visible window picks the new version when it is closed and reopened.
- Release publishing from `v*` tags: Debian package, signed APT suite `peakd` on
  GitHub Pages and a GitHub Release with the package and checksums.

### Fixed

- Apple T2 live audio: WirePlumber no longer fails with a missing `liblua5.4`,
  and the T2 audio stack is configured before optional components.
- Craft store waits for connectivity in the live image instead of caching the
  GitHub error for an hour.
- Release build installs `libclang` for the `btls-sys` build script.

## [0.1.0] - 2026-10-25

### Added

- Initial PEAK'D! live installer ISO for Apple T2 and generic hardware.
- `peakd` Debian package with the Qt shell, the `shiny` core, plugins and web UI.
- Signed APT repository served from GitHub Pages.
