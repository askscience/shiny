//! Touch Bar integration for the kiosk shell.
//!
//! One action vocabulary, two transports (the vocabulary lives in
//! `web/js/touchbarShared.js`):
//!
//! * **macOS** — this module puts a native `NSTouchBar` on the kiosk window.
//!   Each button queues its action name; the event loop drains the queue and
//!   evaluates a `touchbar:action` event in the page. A Mac without a Touch Bar
//!   simply never shows the bar, so this is safe on every Mac.
//! * **Linux T2** — the `tiny-dfr` daemon draws the bar and its buttons emit
//!   F13–F21 through uinput. The page maps those codes itself
//!   (`web/js/touchbar.js`); this module only *detects* the hardware so it can
//!   tell the page a bar exists. `scripts/touchbar/install-touchbar.sh`
//!   installs the row.
//!
//! Everything degrades to a no-op. On unsupported machines `install` does
//! nothing and the web layer stays dormant unless the user forces it on in
//! Settings.

use parking_lot::Mutex;
use std::sync::Arc;

/// The buttons, in bar order: `(action, label)`.
///
/// This must match `TOUCHBAR_ACTIONS` in `web/js/touchbarShared.js` — the Node
/// test `web/js/tests/touchbar.test.mjs` parses this constant and fails if the
/// two drift apart.
#[allow(dead_code)]
pub const BUTTONS: &[(&str, &str)] = &[
    ("talk", "Ask"),
    ("stop", "Stop"),
    ("mute", "Mute"),
    ("volume-down", "Vol-"),
    ("volume-up", "Vol+"),
    ("screen-down", "Scr-"),
    ("screen-up", "Scr+"),
    ("workspace-prev", "Prev"),
    ("workspace-next", "Next"),
    ("kbd-backlight-down", "Kbd-"),
    ("kbd-backlight-up", "Kbd+"),
    ("keyboard", "Keys"),
    ("settings", "Setup"),
];

/// Actions produced by a native Touch Bar, drained on the event-loop thread.
///
/// A plain queue rather than a direct call into the webview: AppKit can invoke
/// the button action at any point while the event loop is running, and
/// `WebView::evaluate_script` must happen on the loop, in step with the rest of
/// the shell's work.
#[derive(Clone, Default)]
pub struct TouchBarBridge {
    queue: Arc<Mutex<Vec<String>>>,
}

impl TouchBarBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an action from the native bar (main thread).
    #[allow(dead_code)]
    fn push(&self, action: &str) {
        self.queue.lock().push(action.to_string());
    }

    /// Take everything queued since the last call.
    fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.queue.lock())
    }
}

/// The JS the page runs for one queued action.
fn dispatch_script(action: &str) -> Option<String> {
    let payload = serde_json::to_string(action).ok()?;
    Some(format!(
        "window.dispatchEvent(new CustomEvent('touchbar:action', \
         {{ detail: {{ action: {payload} }} }}));"
    ))
}

/// Evaluate any queued Touch Bar actions in the page.
///
/// Called once per event-loop tick (the shell already polls at 100 ms). A
/// cheap no-op while the queue is empty, which is always the case off macOS.
pub fn pump(bridge: &TouchBarBridge, webview: &wry::WebView) {
    for action in bridge.drain() {
        let Some(script) = dispatch_script(&action) else {
            continue;
        };
        if let Err(err) = webview.evaluate_script(&script) {
            tracing::warn!(%err, %action, "could not dispatch a Touch Bar action");
        }
    }
}

/// Whether this machine has a Touch Bar we can drive.
///
/// On macOS the hardware cannot be queried, and installing an `NSTouchBar` is
/// harmless without one, so this reports `true` and the OS decides. On Linux
/// we look for the T2 `appletb` devices: the upstream `hid-appletb-bl`/
/// `hid-appletb-kbd` driver (kernel 6.15+) or the older `apple-ib-tb` driver.
pub fn host_has_touch_bar() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }

    #[cfg(target_os = "linux")]
    {
        const PATHS: &[&str] = &[
            // tiny-dfr's backlight device.
            "/sys/class/backlight/appletb_backlight",
            "/sys/class/leds/appletb_backlight",
            // The input half, and the legacy iBridge driver.
            "/sys/module/hid_appletb_kbd",
            "/sys/module/apple_ib_tb",
        ];
        PATHS.iter().any(|path| std::path::Path::new(path).exists())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// Initialization script telling the page whether a Touch Bar exists.
///
/// `None` off supported hardware, which leaves `window.__shinyTouchBar`
/// undefined and the web layer dormant — the graceful-degradation path.
pub fn init_script() -> Option<&'static str> {
    host_has_touch_bar().then_some("window.__shinyTouchBar = true;")
}

/// Owns the native bar and its action target for the shell's lifetime.
///
/// The window retains the bar, but its items hold only a *weak* reference to
/// the target, so the target has to be owned here or the buttons would go
/// dead. A unit on every platform without a native bar.
#[cfg(target_os = "macos")]
#[allow(dead_code)] // both fields exist for their retain/drop effect
pub struct TouchBarHandle {
    _bar: objc2::rc::Retained<objc2_app_kit::NSTouchBar>,
    _target: objc2::rc::Retained<macos::TouchBarTarget>,
}

#[cfg(not(target_os = "macos"))]
pub struct TouchBarHandle;

/// Put a native Touch Bar on the kiosk window, if this is macOS.
#[cfg(target_os = "macos")]
pub fn install(
    window: &tao::window::Window,
    bridge: TouchBarBridge,
) -> Option<TouchBarHandle> {
    macos::install(window, bridge)
}

#[cfg(not(target_os = "macos"))]
pub fn install(
    _window: &tao::window::Window,
    _bridge: TouchBarBridge,
) -> Option<TouchBarHandle> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{TouchBarBridge, TouchBarHandle, BUTTONS};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker};
    use objc2_app_kit::{
        NSButtonTouchBarItem, NSTouchBar, NSTouchBarItem, NSWindow,
    };
    use objc2_foundation::{NSArray, NSSet, NSString};
    use tao::platform::macos::WindowExtMacOS;

    /// Prefix that turns an item identifier back into an action name.
    const IDENTIFIER_PREFIX: &str = "com.shiny.touchbar.";

    pub(super) struct TouchBarIvars {
        bridge: TouchBarBridge,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements and this class
        // does not implement `Drop`. The ivars are an `Arc<Mutex<..>>`, so they
        // are `Send + Sync` and the class may be used from any thread — though
        // AppKit only ever calls it on the main one.
        #[unsafe(super(NSObject))]
        #[name = "ShinyTouchBarTarget"]
        #[ivars = TouchBarIvars]
        pub(super) struct TouchBarTarget;

        impl TouchBarTarget {
            /// Fired by any of the bar's buttons; the sender's identifier says
            /// which one. Queues the action for the event loop to dispatch.
            #[unsafe(method(shinyTouchBarAction:))]
            fn shiny_touch_bar_action(&self, sender: &NSTouchBarItem) {
                let identifier = sender.identifier().to_string();
                let action = identifier
                    .strip_prefix(IDENTIFIER_PREFIX)
                    .unwrap_or(&identifier)
                    .to_string();
                self.ivars().bridge.push(&action);
            }
        }

        unsafe impl NSObjectProtocol for TouchBarTarget {}
    );

    impl TouchBarTarget {
        fn new(bridge: TouchBarBridge) -> Retained<Self> {
            let this = Self::alloc().set_ivars(TouchBarIvars { bridge });
            unsafe { msg_send![super(this), init] }
        }
    }

    pub(super) fn install(
        window: &tao::window::Window,
        bridge: TouchBarBridge,
    ) -> Option<TouchBarHandle> {
        let mtm = MainThreadMarker::new()?;

        let ns_window_ptr = window.ns_window() as *mut NSWindow;
        if ns_window_ptr.is_null() {
            tracing::warn!("no NSWindow yet; skipping the Touch Bar");
            return None;
        }
        let ns_window: &NSWindow = unsafe { &*ns_window_ptr };

        let target = TouchBarTarget::new(bridge);
        // `target` is a custom class, so its pointer is an object pointer like
        // any other; the constructor wants it erased to `&AnyObject`.
        let target_object: &AnyObject =
            unsafe { &*(Retained::as_ptr(&target) as *const AnyObject) };

        let mut identifiers: Vec<Retained<NSString>> = Vec::with_capacity(BUTTONS.len());
        let mut items: Vec<Retained<NSTouchBarItem>> = Vec::with_capacity(BUTTONS.len());
        for (action, label) in BUTTONS {
            let identifier = NSString::from_str(&format!("{IDENTIFIER_PREFIX}{action}"));
            let title = NSString::from_str(label);
            let item = unsafe {
                NSButtonTouchBarItem::buttonTouchBarItemWithIdentifier_title_target_action(
                    &identifier,
                    &title,
                    Some(target_object),
                    Some(sel!(shinyTouchBarAction:)),
                    mtm,
                )
            };
            identifiers.push(identifier);
            items.push(item.into_super());
        }

        let bar = NSTouchBar::new(mtm);
        bar.setDefaultItemIdentifiers(&NSArray::from_retained_slice(&identifiers));
        bar.setTemplateItems(&NSSet::from_retained_slice(&items));
        // `setTouchBar` is declared on `NSResponder`, which `NSWindow` derefs
        // to. On a Mac without Touch Bar hardware this simply has no effect.
        ns_window.setTouchBar(Some(&bar));

        println!("peakd: Touch Bar installed ({} buttons)", BUTTONS.len());
        Some(TouchBarHandle {
            _bar: bar,
            _target: target,
        })
    }
}
