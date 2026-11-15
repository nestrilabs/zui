//! The AccessKit adapter the X11 and Wayland windows call. With the `a11y` feature this is
//! accesskit_unix's adapter. Without it the same calls are accepted and do nothing, so windows
//! expose no accessibility tree to assistive technology.

#[cfg(feature = "a11y")]
pub(crate) use accesskit_unix::Adapter;

#[cfg(not(feature = "a11y"))]
pub(crate) struct Adapter;

#[cfg(not(feature = "a11y"))]
impl Adapter {
    pub(crate) fn new<A, B, C>(_activation: A, _action: B, _deactivation: C) -> Self {
        Self
    }

    pub(crate) fn update_window_focus_state(&mut self, _is_focused: bool) {}

    pub(crate) fn update_if_active(&mut self, _updater: impl FnOnce() -> accesskit::TreeUpdate) {}

    pub(crate) fn set_root_window_bounds(&mut self, _outer: accesskit::Rect, _inner: accesskit::Rect) {}
}
