# Installer plugin (live only)

The PEAK'D! installer is a normal native plugin plus a separate privileged helper.
The plugin runs as the live user and can only send a typed JSON plan to a Unix socket;
the helper runs as root, re-probes the machine and applies the plan.

This build implements:

- read-only disk/hardware probing (`lsblk`, DMI, T2 bridge detection);
- plan validation (target identity, mounted/removable/live-medium refusal, profile/kernel
  mismatch, minimum disk size);
- a **dry-run** apply with progress and cancellation.

The destructive engine is selected by the Phase 0 spike (curtin or a minimal custom
engine) and is deliberately not wired yet. `apply` refuses to run unless the helper was
started with `--dry-run`.

## Live-only guard

The plugin registers its routes only when:

```sh
SHINY_LIVE_INSTALLER=1
```

Without it, the plugin loads but stays inert. The ISO must set this variable for the
live session and include only the installer plugin (plus the baseline needed by the
live UI).

## Build and test

```sh
cargo build -p shiny-installer-helper -p shiny-installer-plugin
cargo test -p shiny-installer-protocol -p shiny-installer-helper
node plugins/installer/web/timeline-model.test.mjs
node --experimental-vm-modules plugins/installer/web/plugin.smoke.mjs
```

The JavaScript tests use synthetic disk fixtures and a DOM shim; they never open a
device or call the helper.

## Run the helper (live image)

```sh
sudo /usr/libexec/shiny/shiny-installer-helper \
  --socket /run/shiny/installer.sock --dry-run
```

The socket is `root:shiny 0660` when the `shiny` group exists. The plugin reads
`SHINY_INSTALLER_SOCKET` if set; otherwise it uses `/run/shiny/installer.sock`.

The sample unit is `scripts/shiny-installer-helper.service`; it is gated on
`/run/live/medium` and therefore does not run on an installed system.

## Web surface

`web/plugin.js` builds the five-step wizard (region, disk, account, system profile,
review) and the progress screen. `web/plugin.css` is loaded by the plugin itself,
because the core only builds the window chrome. The wizard uses the shared `ui/*`
components and theme tokens; the disk map and timeline are plugin-local layout, not
new global components.

The Destination step is a **timeline aligned “now / after”**, in the spirit of a video
editor: the disk is the track, existing partitions are locked clips, free extents are
dashed zones with “Install here”, and the new EFI/swap/home/root clips can be resized
by dragging their edges. The disk shelf on top shows every probed disk with its real
free space; the intent selector chooses between `manual` (free space), `erase` and
`existing` (one partition). The default is safe and ready: the best internal disk, the
largest usable gap and the recommended preset are preselected; a partition with an
existing filesystem is never selected automatically and needs an explicit two-step
confirmation.

`web/timeline-model.js` is the pure source of truth for spans, gaps, presets, resize
clamps and defaults. `web/timeline.js` renders the ruler, tracks, handles, zoom,
summary and exact-size panel; it uses the real absolute offsets and keeps existing
partitions read-only. `web/plugin.js` only translates the current wizard state into
the model and back. Probe/validation are real; the destructive engine is Phase B, so
apply remains dry-run. The Region step asks the helper for the live system's locale,
keyboard and timezone lists, so the wizard uses menus instead of free text.

The Profile step detects NVIDIA GPUs. The open driver is the default; the proprietary
driver and optional CUDA are separate opt-ins with separate license links and acceptance
records in `InstallPlan.licenses`. NVIDIA is never preinstalled in the generic profile,
there is no NVIDIA edition, and the full CUDA Toolkit is not shipped in the base image.
Secure Boot/MOK and hybrid/datacenter cases are surfaced in the UI and re-checked by the
helper. The Review step requires the exact disk serial plus an erase acknowledgement
before the helper's dry-run job can start. The footer's UI-only progress preview never
calls the helper.

## Routes

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/installer/health` | helper version, root/dry-run state and hardware |
| GET | `/api/installer/options` | system locales, keyboard layouts and timezones |
| GET | `/api/installer/probe` | whole disks + DMI/T2 facts |
| POST | `/api/installer/plan` | validate a typed `InstallPlan` |
| POST | `/api/installer/apply` | start a dry-run/real job |
| GET | `/api/installer/status?job=…` | job stage, progress and log |
| POST | `/api/installer/cancel` | request cancellation between stages |

All routes are authenticated and local-only (`is_remote_request` is refused).

## Probe and placement

- `probe` reads `lsblk` for the device tree and a single read-only
  `parted -m -s <disk> unit B print free` for the absolute partition starts and the
  free extents. `DiskInfo.start_bytes` is `0` when the offsets are unavailable; the UI
  then marks the map as approximate and disables free-space planning.
- In `manual` mode the plan carries `layout.free_extent` (the exact gap chosen by the
  user). The helper re-probes and rejects an extent that moved or is no longer there;
  the engine starts creating partitions at that extent instead of picking the largest
  free area on its own.
- `plugins/installer/web/timeline-model.test.mjs` and
  `plugins/installer/web/plugin.smoke.mjs` exercise the model and the real timeline
  against synthetic disks only.

## Safety

- The plugin never runs a shell and never opens a block device.
- The helper re-probes and matches the plan against the disk path, serial, model and
  size before applying anything.
- `apply` requires the user to type the exact target serial; the helper compares it with
  a fresh probe and aborts on a mismatch.
- Non-dry-run apply and `--apply-plan` refuse to run unless `/run/live/medium` exists,
  so a development machine cannot be used as an install target by accident.
- The live medium, mounted disks, removable/loop/optical/read-only devices and disks
  under 8 GiB are refused.
- Existing partitions are never resized or moved: only the chosen free extent or the
  one explicitly selected partition is written.
- The T2 profile is refused on non-T2 hardware; the generic profile warns on T2.
- The dry-run never writes to a disk; the real engine must be added only after the
  Phase 0 spike and destructive tests.

## Never test on the development disk

The UI/model/smoke tests use fixtures and a DOM shim only. Do **not** run
`shiny-installer-helper`, `--apply-plan`, `POST /api/installer/apply`, `parted`,
`wipefs`, `mkfs.*`, `mkswap`, `sfdisk`, `fdisk` or `dd` against a real `/dev/*` device
while developing. Do not set `SHINY_LIVE_INSTALLER=1` on the host. Any end-to-end
check of the destructive engine belongs to a disposable VM or USB stick, never to the
machine running this checkout.
