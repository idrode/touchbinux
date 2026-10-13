//! The desktop's activity for `[idle]` (`activity_from` "desktop"), as the
//! compositor tells it through ext-idle-notify-v1: one notification per stage
//! with its time, each saying "idled" (no input on any seat device for that
//! long) and "resumed" (input again). Nothing else: no keys, no pointer, no
//! device is opened or read here, and the only objects bound are the notifier
//! and a seat (the protocol needs one to name; its own events are ignored).
//!
//! The daemon runs as root and connects to the session user's Wayland socket
//! (`user::session_env` finds and checks it). Everything is non-blocking and
//! driven by the main loop's epoll: `connect` only sends the first requests, and
//! `read` handles what came in, creating the notifications once the compositor
//! has listed its globals. A compositor without the protocol is an error then,
//! after which the bar's activity alone counts.
//!
//! Notifications made with `get_idle_notification` obey idle inhibitors: while
//! one is active (a video playing fullscreen, a Hyprland `idleinhibit` rule) the
//! compositor says nothing is idle, and if one starts after "idled", it says
//! "resumed".

use crate::idle::Stage;
use anyhow::{Context, Result, anyhow, bail};
use std::{io::ErrorKind, os::fd::BorrowedFd, os::unix::net::UnixStream, path::Path, time::Duration};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    backend::{Backend, WaylandError},
    protocol::{wl_callback, wl_registry, wl_seat},
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

const NOTIFIER: &str = "ext_idle_notifier_v1";

/// What the compositor said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEvent {
    /// No desktop input for this stage's time.
    Idled(Stage),
    /// Desktop input again (sent by every notification that had idled).
    Resumed,
}

/// A connection to the compositor with a notification per stage.
pub struct DesktopIdle {
    conn: Connection,
    backend: Backend,
    queue: EventQueue<Handler>,
    handler: Handler,
}

/// The connection's state, filled by the event handlers.
struct Handler {
    timeouts: Vec<(Stage, Duration)>,
    seat: Option<wl_seat::WlSeat>,
    notifier: Option<ExtIdleNotifierV1>,
    /// The first globals have all come (the `sync` after `get_registry`).
    listed: bool,
    notifications: Vec<ExtIdleNotificationV1>,
    events: Vec<DesktopEvent>,
}

impl DesktopIdle {
    /// Connects to the Wayland socket at `socket`, for the stages and times in
    /// `timeouts` (at least one). Doesn't wait for an answer.
    pub fn connect(socket: &Path, timeouts: Vec<(Stage, Duration)>) -> Result<DesktopIdle> {
        if timeouts.is_empty() {
            bail!("no stages to notify");
        }
        let stream = UnixStream::connect(socket).with_context(|| format!("connecting to {}", socket.display()))?;
        let conn = Connection::from_socket(stream).map_err(|e| anyhow!("{}: {e}", socket.display()))?;
        let queue = conn.new_event_queue();
        let qh = queue.handle();
        let display = conn.display();
        display.get_registry(&qh, ());
        // Answered after every global has been announced.
        display.sync(&qh, ());
        let d = DesktopIdle {
            backend: conn.backend(),
            conn,
            queue,
            handler: Handler {
                timeouts,
                seat: None,
                notifier: None,
                listed: false,
                notifications: Vec::new(),
                events: Vec::new(),
            },
        };
        d.flush()?;
        Ok(d)
    }

    /// Readable when the compositor sent something (or went away).
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.backend.poll_fd()
    }

    /// Handles what came in, adding what the compositor said to `out`. An error
    /// means the connection is no use: the compositor went away, broke the
    /// protocol, or lacks ext-idle-notify-v1.
    pub fn read(&mut self, out: &mut Vec<DesktopEvent>) -> Result<()> {
        self.queue.dispatch_pending(&mut self.handler).context("wayland dispatch")?;
        if let Some(guard) = self.queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => bail!("wayland: {e}"),
            }
        }
        self.queue.dispatch_pending(&mut self.handler).context("wayland dispatch")?;
        if self.handler.listed && self.handler.notifications.is_empty() {
            self.subscribe()?;
        }
        out.append(&mut self.handler.events);
        self.flush()
    }

    /// The globals are known: one notification per stage.
    fn subscribe(&mut self) -> Result<()> {
        let h = &mut self.handler;
        let Some(notifier) = &h.notifier else {
            bail!("the compositor has no {NOTIFIER} (ext-idle-notify-v1)");
        };
        let Some(seat) = &h.seat else {
            bail!("the compositor has no seat");
        };
        let qh = self.queue.handle();
        for &(stage, after) in &h.timeouts {
            let ms = u32::try_from(after.as_millis()).unwrap_or(u32::MAX);
            h.notifications.push(notifier.get_idle_notification(ms, seat, &qh, stage));
        }
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        match self.conn.flush() {
            // The few small requests sent fit; the rest goes with the next flush.
            Err(WaylandError::Io(e)) if e.kind() == ErrorKind::WouldBlock => Ok(()),
            r => r.map_err(|e| anyhow!("wayland: {e}")),
        }
    }
}

impl Drop for DesktopIdle {
    fn drop(&mut self) {
        // Polite, not needed: closing the socket destroys everything anyway.
        for n in self.handler.notifications.drain(..) {
            n.destroy();
        }
        let _ = self.conn.flush();
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Handler {
    fn event(
        h: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global { name, interface, .. } = event else {
            return;
        };
        // Version 1 of each: the notifier's get_idle_notification obeys inhibitors.
        if interface == NOTIFIER && h.notifier.is_none() {
            h.notifier = Some(registry.bind(name, 1, qh, ()));
        } else if interface == "wl_seat" && h.seat.is_none() {
            h.seat = Some(registry.bind(name, 1, qh, ()));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Handler {
    fn event(h: &mut Self, _: &wl_callback::WlCallback, event: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = event {
            h.listed = true;
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Handler {
    /// Its name and capabilities: not needed.
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtIdleNotifierV1, ()> for Handler {
    /// It has no events.
    fn event(_: &mut Self, _: &ExtIdleNotifierV1, _: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtIdleNotificationV1, Stage> for Handler {
    fn event(
        h: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        stage: &Stage,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => h.events.push(DesktopEvent::Idled(*stage)),
            ext_idle_notification_v1::Event::Resumed => h.events.push(DesktopEvent::Resumed),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

    /// Against the Wayland session this runs in (`WAYLAND_DISPLAY`), as the user
    /// it runs as: subscribes with a 1 s time and waits up to 30 s for "idled"
    /// (leave the keyboard and mouse alone), printing what comes. Ignored: it
    /// needs a compositor and an idle desktop.
    /// `cargo test desktop_idle_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn desktop_idle_live() {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap();
        let socket = Path::new(&dir).join(std::env::var("WAYLAND_DISPLAY").unwrap());
        let mut d = DesktopIdle::connect(&socket, vec![(Stage::Dim, Duration::from_secs(1))]).unwrap();
        let start = std::time::Instant::now();
        let mut got = Vec::new();
        while start.elapsed() < Duration::from_secs(30) && !got.contains(&DesktopEvent::Idled(Stage::Dim)) {
            let mut fds = [PollFd::new(d.fd(), PollFlags::POLLIN)];
            poll(&mut fds, PollTimeout::from(1000u16)).unwrap();
            d.read(&mut got).unwrap();
            eprintln!("{:?}: {got:?}", start.elapsed());
        }
        assert!(got.contains(&DesktopEvent::Idled(Stage::Dim)), "{got:?}");
    }

    #[test]
    fn a_compositor_that_isnt_there() {
        let err = DesktopIdle::connect(Path::new("/nonexistent/wayland-9"), vec![(Stage::Dim, Duration::from_secs(1))]);
        assert!(format!("{:#}", err.err().unwrap()).contains("connecting to /nonexistent/wayland-9"));
        let err = DesktopIdle::connect(Path::new("/nonexistent"), vec![]);
        assert!(format!("{:#}", err.err().unwrap()).contains("no stages"));
    }
}
