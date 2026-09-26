//! `ViewHost` implementation on top of the Qt shim.

use crate::browse::{CssRect, ViewHost};
use crate::shim;

pub struct QtHost;

impl ViewHost for QtHost {
    fn create(&mut self, id: &str, url: &str, rect: Option<CssRect>, visible: bool) {
        let (x, y, w, h) = rect.map(CssRect::to_logical).unwrap_or((0, 0, 0, 0));
        shim::view_create(id, url, x, y, w, h, visible);
    }

    fn navigate(&mut self, id: &str, url: &str) {
        shim::view_navigate(id, url);
    }

    fn set_bounds(&mut self, id: &str, rect: CssRect) {
        let (x, y, w, h) = rect.to_logical();
        shim::view_bounds(id, x, y, w, h);
    }

    fn set_visible(&mut self, id: &str, visible: bool) {
        shim::view_visible(id, visible);
    }

    fn back(&mut self, id: &str) {
        shim::view_back(id);
    }

    fn forward(&mut self, id: &str) {
        shim::view_forward(id);
    }

    fn reload(&mut self, id: &str) {
        shim::view_reload(id);
    }

    fn focus(&mut self, id: &str) {
        shim::view_focus(id);
    }

    fn close(&mut self, id: &str) {
        shim::view_close(id);
    }
}
