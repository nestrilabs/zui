//! System notifications for a gpui_linux built without the `notifications` feature. Nothing is
//! shown and no response ever arrives; the API matches the notify-rust version so the platform
//! code compiles unchanged.

use gpui::{ForegroundExecutor, SystemNotification, SystemNotificationResponse};

pub(crate) struct SystemNotificationState;

impl SystemNotificationState {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn show(&self, _app_name: Option<&str>, notification: SystemNotification) {
        log::warn!(
            "system notification {:?} dropped: this build leaves out the notifications feature",
            notification.tag
        );
    }

    pub(crate) fn dismiss(&self, _tag: &str) {}

    pub(crate) fn on_response(
        &mut self,
        _executor: &ForegroundExecutor,
        _callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
    }
}
