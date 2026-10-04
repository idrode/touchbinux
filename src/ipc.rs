//! Our own Unix socket: JSON lines in both directions.
//!
//! In  (client -> daemon): `{"type":"set","key":"volume","value":42}`
//! Out (daemon -> every client): `{"type":"tap","id":"..."}`,
//! `{"type":"slider","id":"vol","value":57}`, and `{"type":"error",...}` replies.
//!
//! Every client socket is non-blocking and owned by the main epoll loop; nothing
//! here ever blocks.

use anyhow::{Context, Result};
use nix::sys::{
    epoll::{Epoll, EpollEvent, EpollFlags},
    socket::{getsockopt, sockopt::PeerCredentials},
    stat::{Mode, umask},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::{
        fs::{FileTypeExt, PermissionsExt, chown},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};

/// Longest accepted input line; longer ones are discarded up to the next newline.
pub const MAX_LINE: usize = 64 * 1024;
/// A client that doesn't read its events gets dropped past this much backlog.
const MAX_OUTBUF: usize = 1024 * 1024;
const MAX_CLIENTS: usize = 32;
/// Bytes read from one client per wakeup, so one chatty client can't starve the loop.
const MAX_READ_PER_WAKEUP: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Incoming {
    Set { key: String, value: Value },
}

struct Client {
    stream: UnixStream,
    uid: u32,
    inbuf: Vec<u8>,
    discarding: bool,
    outbuf: Vec<u8>,
    /// Registered for EPOLLOUT (only while `outbuf` is non-empty).
    want_write: bool,
}

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    /// Uid allowed besides root (the socket's owner).
    owner_uid: Option<u32>,
    clients: Vec<Option<Client>>,
    token_base: u64,
}

impl IpcServer {
    /// Creates the socket at `path`, owned by `owner` (uid, gid) with mode 0600, so
    /// only that user and root can connect. Without an owner it stays root-only.
    pub fn bind(path: &Path, owner: Option<(u32, u32)>, token_base: u64) -> Result<IpcServer> {
        if let Ok(meta) = fs::symlink_metadata(path) {
            if !meta.file_type().is_socket() {
                anyhow::bail!("{} exists and is not a socket", path.display());
            }
            if UnixStream::connect(path).is_ok() {
                anyhow::bail!("{} is in use (another touchbinux running?)", path.display());
            }
            fs::remove_file(path).with_context(|| format!("removing stale {}", path.display()))?;
        }
        // umask 0177 makes bind() create the socket as 0600 from the start: no window
        // where anybody else could connect.
        let old = umask(Mode::from_bits_truncate(0o177));
        let bound = UnixListener::bind(path);
        umask(old);
        let listener = bound.with_context(|| format!("binding {}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        if let Some((uid, gid)) = owner {
            chown(path, Some(uid), Some(gid))
                .with_context(|| format!("chown {} to {uid}:{gid}", path.display()))?;
        }
        listener.set_nonblocking(true)?;
        eprintln!(
            "ipc: listening on {} (mode 0600, owner uid {})",
            path.display(),
            owner.map_or(0, |o| o.0)
        );
        Ok(IpcServer {
            listener,
            path: path.to_path_buf(),
            owner_uid: owner.map(|o| o.0),
            clients: Vec::new(),
            token_base,
        })
    }

    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    /// The client slot for an epoll token, if it belongs to us.
    pub fn client_for_token(&self, token: u64) -> Option<usize> {
        let i = token.checked_sub(self.token_base)? as usize;
        (i < self.clients.len()).then_some(i)
    }

    /// Accepts every pending connection.
    pub fn accept(&mut self, epoll: &Epoll) {
        loop {
            let stream = match self.listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    eprintln!("ipc: accept failed: {e}");
                    return;
                }
            };
            // Belt and braces on top of the file mode: check who is connecting.
            let uid = match getsockopt(&stream, PeerCredentials) {
                Ok(c) => c.uid(),
                Err(e) => {
                    eprintln!("ipc: no peer credentials, rejecting: {e}");
                    continue;
                }
            };
            if uid != 0 && Some(uid) != self.owner_uid {
                eprintln!("ipc: rejecting uid {uid}");
                continue;
            }
            let active = self.clients.iter().filter(|c| c.is_some()).count();
            if active >= MAX_CLIENTS {
                eprintln!("ipc: too many clients, rejecting uid {uid}");
                continue;
            }
            if let Err(e) = stream.set_nonblocking(true) {
                eprintln!("ipc: set_nonblocking: {e}");
                continue;
            }
            let slot = match self.clients.iter().position(Option::is_none) {
                Some(i) => i,
                None => {
                    self.clients.push(None);
                    self.clients.len() - 1
                }
            };
            let ev = EpollEvent::new(
                EpollFlags::EPOLLIN | EpollFlags::EPOLLRDHUP,
                self.token_base + slot as u64,
            );
            if let Err(e) = epoll.add(&stream, ev) {
                eprintln!("ipc: epoll add: {e}");
                continue;
            }
            eprintln!("ipc: client {slot} connected (uid {uid})");
            self.clients[slot] = Some(Client {
                stream,
                uid,
                inbuf: Vec::new(),
                discarding: false,
                outbuf: Vec::new(),
                want_write: false,
            });
        }
    }

    /// Handles readiness on a client socket; returns the valid messages it sent.
    pub fn handle(&mut self, slot: usize, flags: EpollFlags, epoll: &Epoll) -> Vec<Incoming> {
        let mut msgs = Vec::new();
        let mut errors = Vec::new();
        let mut close = false;

        if let Some(c) = self.clients.get_mut(slot).and_then(Option::as_mut) {
            if flags.contains(EpollFlags::EPOLLOUT) {
                close |= flush(c).is_err();
            }
            if flags.intersects(EpollFlags::EPOLLIN | EpollFlags::EPOLLRDHUP | EpollFlags::EPOLLHUP)
            {
                let mut tmp = [0u8; 8192];
                let mut total = 0;
                while total < MAX_READ_PER_WAKEUP {
                    match c.stream.read(&mut tmp) {
                        Ok(0) => {
                            close = true;
                            break;
                        }
                        Ok(n) => {
                            total += n;
                            split_lines(c, &tmp[..n], &mut msgs, &mut errors);
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(_) => {
                            close = true;
                            break;
                        }
                    }
                }
            }
        }
        for err in errors {
            self.send_to(slot, &json!({"type": "error", "message": err}), epoll);
        }
        if close {
            self.close(slot, epoll);
        }
        msgs
    }

    /// Sends one message to every client.
    pub fn broadcast(&mut self, msg: &Value, epoll: &Epoll) {
        for slot in 0..self.clients.len() {
            self.send_to(slot, msg, epoll);
        }
    }

    pub fn send_to(&mut self, slot: usize, msg: &Value, epoll: &Epoll) {
        let Some(c) = self.clients.get_mut(slot).and_then(Option::as_mut) else {
            return;
        };
        c.outbuf.extend_from_slice(msg.to_string().as_bytes());
        c.outbuf.push(b'\n');
        if flush(c).is_err() || c.outbuf.len() > MAX_OUTBUF {
            eprintln!("ipc: client {slot} not keeping up or gone, closing");
            self.close(slot, epoll);
            return;
        }
        // Ask for EPOLLOUT only while something is pending.
        let want = !c.outbuf.is_empty();
        if want != c.want_write {
            let mut flags = EpollFlags::EPOLLIN | EpollFlags::EPOLLRDHUP;
            if want {
                flags |= EpollFlags::EPOLLOUT;
            }
            let mut ev = EpollEvent::new(flags, self.token_base + slot as u64);
            if epoll.modify(&c.stream, &mut ev).is_ok() {
                c.want_write = want;
            }
        }
    }

    fn close(&mut self, slot: usize, epoll: &Epoll) {
        if let Some(c) = self.clients.get_mut(slot).and_then(Option::take) {
            let _ = epoll.delete(&c.stream);
            eprintln!("ipc: client {slot} (uid {}) disconnected", c.uid);
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path) {
            eprintln!("warning: failed to remove {}: {e}", self.path.display());
        }
    }
}

/// Writes as much of the backlog as the socket takes.
fn flush(c: &mut Client) -> std::io::Result<()> {
    while !c.outbuf.is_empty() {
        match c.stream.write(&c.outbuf) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => {
                c.outbuf.drain(..n);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Accumulates bytes into lines and parses each complete one.
fn split_lines(c: &mut Client, data: &[u8], msgs: &mut Vec<Incoming>, errors: &mut Vec<String>) {
    for &b in data {
        if b == b'\n' {
            if !c.discarding {
                parse_line(&c.inbuf, msgs, errors);
            }
            c.inbuf.clear();
            c.discarding = false;
        } else if !c.discarding {
            c.inbuf.push(b);
            if c.inbuf.len() > MAX_LINE {
                c.inbuf.clear();
                c.discarding = true;
                errors.push(format!("line longer than {MAX_LINE} bytes, discarded"));
            }
        }
    }
}

fn parse_line(line: &[u8], msgs: &mut Vec<Incoming>, errors: &mut Vec<String>) {
    let line = line.trim_ascii();
    if line.is_empty() {
        return;
    }
    match serde_json::from_slice::<Incoming>(line) {
        Ok(m) => msgs.push(m),
        Err(e) => errors.push(format!("invalid message: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> (Vec<Incoming>, Vec<String>) {
        let (a, _) = UnixStream::pair().unwrap();
        let mut c = Client {
            stream: a,
            uid: 0,
            inbuf: Vec::new(),
            discarding: false,
            outbuf: Vec::new(),
            want_write: false,
        };
        let (mut msgs, mut errors) = (Vec::new(), Vec::new());
        // Feed in small chunks to exercise lines split across reads.
        for chunk in input.chunks(7) {
            split_lines(&mut c, chunk, &mut msgs, &mut errors);
        }
        (msgs, errors)
    }

    #[test]
    fn valid_invalid_and_split_lines() {
        let (msgs, errors) = parse(
            b"{\"type\":\"set\",\"key\":\"volume\",\"value\":42}\n\
              not json\n\
              {\"type\":\"nope\"}\n\
              \n\
              {\"type\":\"set\",\"key\":\"label\",\"value\":\"hi\"}\n",
        );
        assert_eq!(msgs.len(), 2);
        assert!(matches!(&msgs[0], Incoming::Set { key, value } if key == "volume" && value == 42));
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn end_to_end_set_and_broadcast() {
        use nix::sys::epoll::{EpollCreateFlags, EpollTimeout};
        use std::{io::BufRead, os::unix::fs::MetadataExt};

        let path =
            std::env::temp_dir().join(format!("touchbinux-test-{}.sock", std::process::id()));
        let me = fs::metadata("/proc/self").unwrap();
        let mut server = IpcServer::bind(&path, Some((me.uid(), me.gid())), 100).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let epoll = Epoll::new(EpollCreateFlags::empty()).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        server.accept(&epoll);
        client
            .write_all(b"{\"type\":\"set\",\"key\":\"volume\",\"value\":42}\nbad\n")
            .unwrap();
        let mut evs = [EpollEvent::empty(); 4];
        assert!(epoll.wait(&mut evs, EpollTimeout::from(1000u16)).unwrap() > 0);
        let slot = server.client_for_token(evs[0].data()).unwrap();
        let msgs = server.handle(slot, evs[0].events(), &epoll);
        assert_eq!(msgs.len(), 1);

        server.broadcast(&json!({"type": "tap", "id": "x"}), &epoll);
        let mut reader = std::io::BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("\"error\""), "{line}");
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"id\":\"x\",\"type\":\"tap\"}\n");

        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn giant_line_is_discarded_and_next_line_works() {
        let mut input = vec![b'x'; MAX_LINE * 3];
        input.extend_from_slice(b"\n{\"type\":\"set\",\"key\":\"a\",\"value\":1}\n");
        let (msgs, errors) = parse(&input);
        assert_eq!(msgs.len(), 1);
        assert_eq!(errors.len(), 1);
    }
}
