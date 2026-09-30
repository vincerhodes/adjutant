//! Desktop notification seam. `Notifier` is the test seam (mock in tests);
//! the live impl wraps notify-rust over the session dbus. A missing
//! notification daemon is logged and swallowed — reminders degrade to
//! in-app-only, never panic.

/// Sends a desktop notification. Implementations must never panic.
pub trait Notifier {
    fn notify(&self, title: &str, body: &str);
}

/// Live notifier over notify-rust (XDG desktop notifications, Wayland-safe).
/// On Omarchy/Hyprland a notification daemon is normally present; when it is
/// not, `show()` errors are logged and swallowed — callers surface the
/// reminder in-app regardless.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&self, title: &str, body: &str) {
        if let Err(e) = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .show()
        {
            eprintln!("adjutant: desktop notification failed (no daemon?): {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MockNotifier {
        calls: std::cell::RefCell<Vec<(String, String)>>,
    }

    impl Notifier for MockNotifier {
        fn notify(&self, title: &str, body: &str) {
            self.calls
                .borrow_mut()
                .push((title.to_string(), body.to_string()));
        }
    }

    #[test]
    fn mock_notifier_records_calls_without_dbus() {
        let mock = MockNotifier::default();
        mock.notify("Standup", "in 10 minutes");
        assert_eq!(
            mock.calls.borrow().as_slice(),
            &[("Standup".to_string(), "in 10 minutes".to_string())]
        );
    }
}
