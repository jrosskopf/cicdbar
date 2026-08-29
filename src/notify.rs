//! Desktop notifications over D-Bus (`org.freedesktop.Notifications`).
//!
//! Spoken directly with `zbus` rather than through `notify-rust`, whose
//! default backend links libdbus and would break the static musl build.
//!
//! A failed notification must never break the widget: every error is reported
//! to the caller for the tooltip and nothing more.

use zbus::blocking::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low = 0,
    Normal = 1,
    Critical = 2,
}

pub struct Notifier {
    conn: Option<Connection>,
}

impl Notifier {
    /// Connect to the session bus.
    pub fn connect() -> anyhow::Result<Notifier> {
        let conn = Connection::session()?;
        Ok(Notifier { conn: Some(conn) })
    }

    /// A notifier that does nothing: `--no-notify`, `--demo`, or
    /// `enabled = false`. Works with no bus present at all.
    pub fn disabled() -> Notifier {
        Notifier { conn: None }
    }

    pub fn is_enabled(&self) -> bool {
        self.conn.is_some()
    }

    /// Send (or, with `replaces`, update in place) a notification.
    ///
    /// Deliberately sends no `x-dunst-stack-tag`: **mako 1.11.0 crashes**
    /// when a notification carrying that hint is replaced via `replaces_id`.
    /// Reproducible from `busctl` alone, so it is not something this crate
    /// can work around by sending it differently -- and `replaces_id` is the
    /// standard mechanism for exactly the coalescing the tag would give us,
    /// so nothing is lost by omitting it.
    pub fn send(
        &self,
        replaces: Option<u32>,
        summary: &str,
        body: &str,
        urgency: Urgency,
        _stack_tag: &str,
    ) -> anyhow::Result<u32> {
        let Some(conn) = &self.conn else { return Ok(0) };

        let hints: std::collections::HashMap<&str, zbus::zvariant::Value> =
            [("urgency", zbus::zvariant::Value::U8(urgency as u8))]
                .into_iter()
                .collect();

        // Critical notifications stay until dismissed; the rest expire on the
        // daemon's default timeout.
        let timeout: i32 = if urgency == Urgency::Critical { 0 } else { -1 };

        let reply = conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "cicdbar",
                replaces.unwrap_or(0),
                "",
                summary,
                body,
                Vec::<&str>::new(),
                hints,
                timeout,
            ),
        )?;
        Ok(reply.body().deserialize::<u32>()?)
    }
}
