# Display & input

Host integrations that are all "write the machine, not the page": the
**interface scale**, **screen brightness**, **keyboard backlight**, the
**Touch Bar**, **mouse and touchpad options**, and **three-finger trackpad
gestures**.

Source: [`src/services/display.rs`](../../src/services/display.rs),
[`keyboard_backlight.rs`](../../src/services/keyboard_backlight.rs),
[`screen_brightness.rs`](../../src/services/screen_brightness.rs),
[`backlight.rs`](../../src/services/backlight.rs),
[`touchbar.rs`](../../src/services/touchbar.rs),
[`pointer.rs`](../../src/services/pointer.rs),
[`crates/liteaura/src/gestures.rs`](../../crates/liteaura/src/gestures.rs).

---

## Interface scale

`DisplayService` persists the webview page-zoom choice; the kiosk shell
(`liteaura`) reads it and applies it. The server never touches WebKit.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/display` | Bearer/cookie | Current scale. Remote → `available:false`. |
| `PUT` | `/api/display` | **loopback** | Body `{ "scale": "auto" | 1.25 }`. |

The value is a small file the shell watches/reloads. See
[display scale](#display-scale) in the kiosk docs and
[`docs/deployment/kiosk-shell.md`](../deployment/kiosk-shell.md).

---

## Screen brightness

`ScreenBrightnessService` reads/writes the panel backlight through the shared
sysfs backlight helper ([`backlight.rs`](../../src/services/backlight.rs)); on a
T2 Mac that is `gmux_backlight`, otherwise `/sys/class/backlight/*`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/screen/brightness` | Bearer/cookie | Current value + range + `available`. |
| `POST` | `/api/screen/brightness` | **loopback** | Body `{ "value": 0..100 }` (or the driver's raw range). |

On a machine with no controllable panel backlight the service reports
`available:false` and the action is a no-op; the battery quick menu shows the
slider disabled.

---

## Keyboard backlight

Same sysfs pattern for the keyboard LED (`kbd_backlight` on the T2 Mac;
`/sys/class/leds/*kbd*` elsewhere).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/keyboard/backlight` | Bearer/cookie | Current value + range + `available`. |
| `POST` | `/api/keyboard/backlight` | **loopback** | Set the value. |

Writing the LED needs permission; the Touch Bar installer adds a udev rule that
grants the `video` group access (see [Touch Bar](#touch-bar)).

Both write paths are plain sysfs, so they work in the bare matchbox kiosk with
no GTK or desktop daemon.

---

## Touch Bar

Shiny can drive the MacBook Pro T1/T2 Touch Bar without assuming the machine has
one: it is an **optional input surface** over actions the HUD and voice bar
already expose. On a normal PC nothing is registered and the feature is dormant.

There are two transports but one action vocabulary
([`web/js/touchbarShared.js`](../../web/js/touchbarShared.js)):

- **macOS** — `liteaura-mac` puts a native `NSTouchBar` on the window
  ([`crates/liteaura-mac/src/touchbar.rs`](../../crates/liteaura-mac/src/touchbar.rs));
  each button evaluates a `touchbar:action` event in the page. A Mac with no
  Touch Bar hardware never shows the bar. Disable with `LITEAURA_TOUCHBAR=0` or
  `liteaura-mac --no-touchbar`.
- **Linux T2** — the kiosk shell draws the bar itself, as a page of the app
  (see [The bar as a surface](#the-bar-as-a-surface)).
  `sudo scripts/touchbar/install-touchbar.sh` grants the access it needs and
  keeps `tiny-dfr` from starting. With `--classic` the bar stays with `tiny-dfr` and shows a
  fixed row of keys: each sends a **Ctrl+Alt+Shift+1…9** combo that
  [`web/js/touchbar.js`](../../web/js/touchbar.js) maps back to the action
  vocabulary (a combo rather than F13–F24, which the X keymap binds to
  `XF86*` keysyms), and Fn shows F1–F12.

### The bar as a surface

On a T2 MacBook the bar is a second display (`appletbdrm`, 60x2008, mounted
sideways) with its own touch digitizer. The shell renders
[`web/touchbar.html`](../../web/touchbar.html) on it:

| Piece | Where | What it does |
|---|---|---|
| Off-screen view | `crates/liteaura/shim/liteaura.cpp` (`liteaura_qt_bar_create`) | A web view in the scene, left of the window, on the app profile. Its picture is grabbed only while it changes: a burst after a touch, a script or a title change, and for as long as frames keep differing. |
| Panel, touch, Fn | [`crates/liteaura/src/touchbar.rs`](../../crates/liteaura/src/touchbar.rs) | A DRM dumb buffer and `DIRTYFB` per frame (turned a quarter); the digitizer's evdev events as `window.__shinyBarTouch`; the internal keyboard's `KEY_FN` as `window.__shinyBarFn`. |
| The bar page | [`web/js/touchbarSurface/bar.js`](../../web/js/touchbarSurface/bar.js) | Draws what it is told, handles taps, swipes and sliders. |
| The desktop side | [`web/js/touchbarHost.js`](../../web/js/touchbarHost.js) | Publishes a snapshot over a `BroadcastChannel` and does what was touched. The protocol is in [`barShared.js`](../../web/js/touchbarSurface/barShared.js). |

What is on the bar: the assistant (the dock's dot renderer, `voiceDots.js`,
with the effect and reactions from Settings → Appearance), the focused
window's controls, with **Dock on the Touch Bar** the whole dock, sliders for
brightness and volume, the battery and the clock. A chip's name or percentage
is on the bar only when the dock shows it. The dock's actions (updates,
clipboard, theme, Settings, Plugins) and the session (`sessionEntries()` in
`powerMenu.js`) sit behind one button that opens a menu on the bar. The dock is *mirrored* from
`#hud-header`: an item on the bar is a control of the real dock, and a tap is a
click on it, so its menu opens on the screen. With the setting on, the
on-screen dock is parked invisibly along the bottom edge
(`html[data-touchbar-dock='1']`) so those menus come up next to the bar; the
workspace switcher and the assistant's button are hidden as well. That button
stays in the document: a press and a release on the bar's button are replayed
on it (`pressOrb`), so the gesture rules in `sphere.js` (tap, double tap,
hold) are the only copy.

A window puts controls on the bar by exporting `touchBar()` from its surface
module, next to `contextMenu()` and with the same entries:

```js
export default {
  // …
  touchBar: () => [
    { id: 'refresh', label: 'Refresh', icon: 'ui/refresh', onClick: refresh },
    { id: 'grid', label: 'Grid', active: view === 'grid', showLabel: true, onClick: () => setView('grid') },
  ],
};
```

At most 12 entries are shown. When they change (not on focus changes, which
are followed already) dispatch `touchbar:refresh` on `window`. A window with no
`touchBar()` shows its `contextMenu()` entries; every window gets full screen
and close.

The desktop page also asks the shell for three things over IPC
(`liteaura:touchbar:{…}`): the bar's `backlight` (2 on, 1 dim after a minute
idle, 0 off after four), an `fkey` to type (the Fn layer), and a `reload`.

`LITEAURA_TOUCHBAR=0` leaves the bar alone. Off hardware,
`LITEAURA_TOUCHBAR_FAKE=2008x60 LITEAURA_TOUCHBAR_DUMP=/tmp/bar.png` renders
the surface with no panel and writes its frames to the file; the page also
works in a browser tab (a mouse is a finger, Alt is Fn).

The page learns a bar exists from the kiosk shell's init flag or from the
server's probe:

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/touchbar` | Bearer/cookie | `{ "available": bool }` — same sysfs check as the shell, so **Automatic** works in a plain browser too. |

The fixed rows (macOS, `--classic`) map to: tap-to-talk (barge-in included),
stop the answer, output mute/volume, screen brightness, previous/next
workspace, keyboard backlight, toggle the virtual keyboard, and open Settings.
Per-user enablement lives in **Settings → Touch Bar** (`Automatic` /
`Always on` / `Off`), with **Dock on the Touch Bar** when the shell draws the
bar. See
[T2 Mac](../deployment/t2-mac.md).

---

## Mouse & touchpad

**Settings → Mouse & touchpad** sets the options the Xorg `libinput` driver
exposes as XInput2 device properties. `PointerService` talks to the X server
over x11rb (no `xinput` binary, no desktop daemon), finding the display the
way the capture plugin does (`$DISPLAY`, else the lowest `/tmp/.X11-unix`
socket).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/pointer` | Bearer/cookie | Devices and their current values, read back from the X server. Remote → `available:false`. |
| `PUT` | `/api/pointer` | **loopback** | A partial change, e.g. `{ "touchpad": { "tap_to_click": true } }`; `null` returns an option to the driver default. Returns the new status. |

Options apply per class: every touchpad (a pointer with the tapping property)
gets the touchpad options, every other libinput pointer with an acceleration
property the mouse options.

| Class | Option | Property |
|---|---|---|
| touchpad | `tap_to_click` | `libinput Tapping Enabled` |
| touchpad | `natural_scroll` | `libinput Natural Scrolling Enabled` |
| touchpad, mouse | `speed` (-1…1) | `libinput Accel Speed` |
| touchpad | `disable_while_typing` | `libinput Disable While Typing Enabled` |
| touchpad | `click_method` (`fingers` / `areas`) | `libinput Click Method Enabled` |
| touchpad | `scroll_method` (`two_finger` / `edge`) | `libinput Scroll Method Enabled` |
| mouse | `accel_profile` (`adaptive` / `flat`) | `libinput Accel Profile Enabled` |
| mouse | `natural_scroll`, `left_handed` | `libinput Natural Scrolling Enabled`, `libinput Left Handed Enabled` |

The choice lives in `~/.config/liteaura/pointer.json` (`LITEAURA_POINTER_FILE`
overrides it) and holds only the options the user changed. Properties belong
to the device, so a background thread applies the file when the X server comes
up (it reconnects after every sign-in) and again on every XInput2 hierarchy
event that adds or enables a device: a mouse plugged in starts from its
defaults and gets the user's options a moment later. The Settings panel
re-reads the devices every few seconds while it is open, so a mouse plugged in
shows its group.

---

## Trackpad gestures

The kiosk recognises three-finger trackpad swipes:

| Swipe | Action |
|---|---|
| left | next workspace |
| right | previous workspace |
| up | window overview — every open window, live |
| down | plugin launcher |

Web engines do not deliver trackpad gestures to the page, so the Linux shell
reads the touchpad's evdev stream itself
([`crates/liteaura/src/gestures.rs`](../../crates/liteaura/src/gestures.rs)) and
dispatches a `trackpad:gesture` event that
[`web/js/gestures.js`](../../web/js/gestures.js) maps onto the desktop. The
reader only *watches* the device — read-only, never `EVIOCGRAB` — so the pointer
and two-finger scrolling are untouched. macOS uses native gestures instead.

The reader distinguishes a scroll from a swipe by requiring fingers to move
*together* and by borrowing libinput's thumb detection (bottom-strip, size/
pressure outlier), since Apple's `bcm5974` never sends `MT_TOOL_PALM`. A pad
that reports neither size nor pressure falls back to contact-travel alone.

Swipes are continuous — the UI follows the fingers and, on release, completes
or springs back. The reader emits a lifecycle:

| Phase | When | Payload |
|---|---|---|
| `begin` | three synchronised fingers moved 30% of the reference distance (12% of the pad's axis) and locked the dominant axis | direction, progress |
| `update` | the fingers moved (≥ 0.002 progress) | progress |
| `end` | fewer than three fingers remain | last progress, velocity |
| `cancel` | `SYN_DROPPED`, the device went away, or the fingers stopped moving together | — |

Every phase is dispatched as `detail: { phase, axis, direction, progress,
velocity }`. `direction` is the begin direction (kept for listeners of the
older discrete `{ direction }` detail); `progress` is signed along it, 1.0 = two
reference distances (a comfortable full swipe), clamped to ±1.5, negative once
the fingers come back past the start; `velocity` is progress per second over
the last 80 ms before the release (fingers that rested before lifting read
as a slow release). The synchronisation check that used to gate the discrete
swipe still runs once the swipe reaches the reference distance: a straggler
there cancels the gesture. There is no time limit any more — a slow,
deliberate swipe is fine. What the page does with the lifecycle is described
in [Overview and launcher](../core/desktop.md#continuous-gestures).

Delivery: the bridge coalesces updates (only the newest between two pump
ticks survives; `begin`/`end`/`cancel` are never dropped). The shell's pump
normally runs every 100 ms; on `begin` the reader thread calls
`liteaura_qt_pump_rate(16, wake)` — queued onto the GUI thread — so events are
delivered at frame rate while the fingers move, and on `end`/`cancel` it goes
back to 100 ms. Both transitions also wake the pump at once.

Permission: opening `/dev/input/event*` as an unprivileged user needs a udev
rule:

```bash
sudo scripts/install-touchpad-gestures.sh   # tags the touchpad with uaccess
```

Without it the shell logs that gestures are off and everything else is
unchanged. `LITEAURA_TOUCHPAD=/dev/input/eventN` overrides discovery.

---

## Graceful degradation

| Missing | Effect |
|---|---|
| No backlight device | `available:false`; slider hidden; write is a no-op. |
| No Touch Bar | `auto` mode never activates; `install-touchbar.sh` exits doing nothing. |
| No readable touchpad | Gesture reader stays off; pointer/scroll unaffected. |
| No X display / not libinput | `/api/pointer` reports `available:false` or no devices; the Settings section says so. |
| No `tiny-dfr` | Linux T2 bar simply isn't driven. |
| Remote client | Reads report unavailable; all writes rejected. |
