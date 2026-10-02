# Display & input

Four host integrations that are all "write the machine, not the page": the
**interface scale**, **screen brightness**, **keyboard backlight**, the
**Touch Bar**, and **three-finger trackpad gestures**.

Source: [`src/services/display.rs`](../../src/services/display.rs),
[`keyboard_backlight.rs`](../../src/services/keyboard_backlight.rs),
[`screen_brightness.rs`](../../src/services/screen_brightness.rs),
[`backlight.rs`](../../src/services/backlight.rs),
[`touchbar.rs`](../../src/services/touchbar.rs),
[`crates/peakd/src/gestures.rs`](../../crates/peakd/src/gestures.rs).

---

## Interface scale

`DisplayService` persists the webview page-zoom choice; the kiosk shell
(`peakd`) reads it and applies it. The server never touches WebKit.

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
`available:false` and the action is a no-op; the Settings slider hides.

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

- **macOS** — `peakd-mac` puts a native `NSTouchBar` on the window
  ([`crates/peakd-mac/src/touchbar.rs`](../../crates/peakd-mac/src/touchbar.rs));
  each button evaluates a `touchbar:action` event in the page. A Mac with no
  Touch Bar hardware never shows the bar. Disable with `PEAKD_TOUCHBAR=0` or
  `peakd-mac --no-touchbar`.
- **Linux T2** — the kernel (`hid-appletb-*` / `apple-ib-tb`) plus the
  `tiny-dfr` daemon own the bar. `sudo scripts/touchbar/install-touchbar.sh`
  installs a Shiny icon row and copies the custom SVGs into `/etc/tiny-dfr`. It
  also installs a udev rule letting the desktop user (via the `video` group)
  write the `kbd_backlight` LED and the panel `backlight`.

`tiny-dfr` can only emit key codes, so each button sends a
**Ctrl+Alt+Shift+1…9** combo, which
[`web/js/touchbar.js`](../../web/js/touchbar.js) maps back to actions. (A combo
rather than F13–F24: on `us`/`es` the X keymap binds those codes to `XF86*`
keysyms, so the page would never see `F13`; digits are mapped everywhere and the
page matches the physical `event.code`.)

The page learns a bar exists from the kiosk shell's init flag or from the
server's probe:

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/touchbar` | Bearer/cookie | `{ "available": bool }` — same sysfs check as the shell, so **Automatic** works in a plain browser too. |

Buttons map to: tap-to-talk (barge-in included), stop the answer, output
mute/volume, screen brightness, previous/next workspace, keyboard backlight,
toggle the virtual keyboard, and open Settings. Per-user enablement lives in
**Settings → Touch Bar** (`Automatic` / `Always on` / `Off`). See
[T2 Mac](../deployment/t2-mac.md).

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
([`crates/peakd/src/gestures.rs`](../../crates/peakd/src/gestures.rs)) and
dispatches a `trackpad:gesture` event that
[`web/js/gestures.js`](../../web/js/gestures.js) maps onto the desktop. The
reader only *watches* the device — read-only, never `EVIOCGRAB` — so the pointer
and two-finger scrolling are untouched. macOS uses native gestures instead.

The reader distinguishes a scroll from a swipe by requiring fingers to move
*together* and by borrowing libinput's thumb detection (bottom-strip, size/
pressure outlier), since Apple's `bcm5974` never sends `MT_TOOL_PALM`. A pad
that reports neither size nor pressure falls back to contact-travel alone.

Permission: opening `/dev/input/event*` as an unprivileged user needs a udev
rule:

```bash
sudo scripts/install-touchpad-gestures.sh   # tags the touchpad with uaccess
```

Without it the shell logs that gestures are off and everything else is
unchanged. `PEAKD_TOUCHPAD=/dev/input/eventN` overrides discovery.

---

## Graceful degradation

| Missing | Effect |
|---|---|
| No backlight device | `available:false`; slider hidden; write is a no-op. |
| No Touch Bar | `auto` mode never activates; `install-touchbar.sh` exits doing nothing. |
| No readable touchpad | Gesture reader stays off; pointer/scroll unaffected. |
| No `tiny-dfr` | Linux T2 bar simply isn't driven. |
| Remote client | Reads report unavailable; all writes rejected. |
