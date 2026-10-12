//! The IO thread's event demultiplexer (the part of libxev's `Loop` the IO
//! thread uses: fd readiness plus a timeout).
//!
//! libxev selects its backend at runtime (`ghostty_os::xev`, set from the
//! `async-backend` config in the GTK application): `io_uring` or `epoll` on
//! Linux. This mirrors that choice:
//!
//! * [`Backend::IoUring`]: one `IORING_OP_POLL_ADD` per interest (re-armed
//!   after it fires, removed with `IORING_OP_POLL_REMOVE` when the interest
//!   goes away) and, for the wait timeout, `EXT_ARG` when the ring has it or
//!   else an `IORING_OP_TIMEOUT` entry (libxev's own timers use the latter).
//! * [`Backend::Epoll`]: a persistent epoll instance whose registrations are
//!   diffed against the interests every wait (level-triggered).
//! * `Poll`: plain `poll(2)`, used where libxev has a single, different
//!   backend (kqueue on macOS) and cannot be selected.

#[cfg(target_os = "linux")]
use std::{collections::HashMap, os::fd::RawFd};
use std::{io, os::fd::BorrowedFd, time::Duration};

pub use ghostty_os::xev::Backend;
use rustix::event::{PollFd, PollFlags};

/// One thing to wait for: readability (or writability) of `fd`. `token` is
/// reported back by [`Reactor::wait`] and must be unique per call. The fd
/// must stay open while its token is passed to `wait`; when it is closed the
/// token must be left out of one `wait` before it is reused with another fd.
#[derive(Clone, Copy)]
pub struct Interest<'a> {
    pub token: usize,
    pub fd: BorrowedFd<'a>,
    pub write: bool,
}

/// See the module docs.
pub enum Reactor {
    Poll,
    #[cfg(target_os = "linux")]
    Epoll(EpollReactor),
    #[cfg(target_os = "linux")]
    IoUring(Box<UringReactor>),
}

impl Reactor {
    /// Create the reactor for `backend`. Backends that do not exist for this
    /// platform use `poll(2)`.
    pub fn new(backend: Backend) -> io::Result<Reactor> {
        match backend {
            #[cfg(target_os = "linux")]
            Backend::Epoll => Ok(Reactor::Epoll(EpollReactor::new()?)),
            #[cfg(target_os = "linux")]
            Backend::IoUring => Ok(Reactor::IoUring(Box::new(UringReactor::new()?))),
            _ => Ok(Reactor::Poll),
        }
    }

    /// An io_uring reactor behaving as on a ring without `EXT_ARG`.
    #[cfg(all(test, target_os = "linux"))]
    fn uring_without_ext_arg() -> io::Result<Reactor> {
        Ok(Reactor::IoUring(Box::new(
            UringReactor::new_without_ext_arg()?,
        )))
    }

    /// A plain `poll(2)` reactor regardless of platform.
    pub fn poll() -> Reactor {
        Reactor::Poll
    }

    /// The name used in logs.
    pub fn name(&self) -> &'static str {
        match self {
            Reactor::Poll => "poll",
            #[cfg(target_os = "linux")]
            Reactor::Epoll(_) => "epoll",
            #[cfg(target_os = "linux")]
            Reactor::IoUring(_) => "io_uring",
        }
    }

    /// Wait until at least one interest is ready or `timeout` elapses
    /// (`None` blocks forever). The tokens of the ready interests are
    /// appended to `ready` (which is cleared first). An interrupted wait
    /// returns with nothing ready.
    pub fn wait(
        &mut self,
        interests: &[Interest<'_>],
        timeout: Option<Duration>,
        ready: &mut Vec<usize>,
    ) -> io::Result<()> {
        ready.clear();
        match self {
            Reactor::Poll => poll_wait(interests, timeout, ready),
            #[cfg(target_os = "linux")]
            Reactor::Epoll(r) => r.wait(interests, timeout, ready),
            #[cfg(target_os = "linux")]
            Reactor::IoUring(r) => r.wait(interests, timeout, ready),
        }
    }
}

fn timespec(d: Duration) -> rustix::time::Timespec {
    rustix::time::Timespec {
        tv_sec: d.as_secs() as _,
        tv_nsec: i64::from(d.subsec_nanos()) as _,
    }
}

fn poll_wait(
    interests: &[Interest<'_>],
    timeout: Option<Duration>,
    ready: &mut Vec<usize>,
) -> io::Result<()> {
    let ts = timeout.map(timespec);
    let mut fds: Vec<PollFd<'_>> = interests
        .iter()
        .map(|i| {
            PollFd::from_borrowed_fd(
                i.fd,
                if i.write {
                    PollFlags::OUT
                } else {
                    PollFlags::IN
                },
            )
        })
        .collect();
    match rustix::event::poll(&mut fds, ts.as_ref()) {
        Ok(_) => {}
        Err(rustix::io::Errno::INTR) => return Ok(()),
        Err(err) => return Err(err.into()),
    }
    for (i, fd) in interests.iter().zip(&fds) {
        if !fd.revents().is_empty() {
            ready.push(i.token);
        }
    }
    Ok(())
}

/// Persistent epoll instance (level-triggered).
#[cfg(target_os = "linux")]
pub struct EpollReactor {
    epoll: std::os::fd::OwnedFd,
    /// token -> (raw fd, write)
    registered: HashMap<usize, (RawFd, bool)>,
}

#[cfg(target_os = "linux")]
impl EpollReactor {
    fn new() -> io::Result<EpollReactor> {
        use rustix::event::epoll;
        Ok(EpollReactor {
            epoll: epoll::create(epoll::CreateFlags::CLOEXEC)?,
            registered: HashMap::new(),
        })
    }

    #[allow(unsafe_code)]
    fn sync(&mut self, interests: &[Interest<'_>]) -> io::Result<()> {
        use rustix::event::epoll;

        // Drop registrations that are gone or changed.
        let stale: Vec<usize> = self
            .registered
            .iter()
            .filter(|(token, (raw, write))| {
                !interests.iter().any(|i| {
                    i.token == **token
                        && std::os::fd::AsRawFd::as_raw_fd(&i.fd) == *raw
                        && i.write == *write
                })
            })
            .map(|(t, _)| *t)
            .collect();
        for token in stale {
            if let Some((raw, _)) = self.registered.remove(&token) {
                // SAFETY: the fd may already have been closed by its owner;
                // `epoll_ctl(DEL)` then fails with EBADF/ENOENT, which is
                // ignored. The borrow does not outlive this call and nothing
                // is read from or written to the fd.
                let fd = unsafe { BorrowedFd::borrow_raw(raw) };
                let _ = epoll::delete(&self.epoll, fd);
            }
        }

        for i in interests {
            if self.registered.contains_key(&i.token) {
                continue;
            }
            let flags = if i.write {
                epoll::EventFlags::OUT
            } else {
                epoll::EventFlags::IN
            };
            epoll::add(
                &self.epoll,
                i.fd,
                epoll::EventData::new_u64(i.token as u64),
                flags,
            )?;
            self.registered
                .insert(i.token, (std::os::fd::AsRawFd::as_raw_fd(&i.fd), i.write));
        }
        Ok(())
    }

    fn wait(
        &mut self,
        interests: &[Interest<'_>],
        timeout: Option<Duration>,
        ready: &mut Vec<usize>,
    ) -> io::Result<()> {
        use rustix::event::epoll;

        self.sync(interests)?;
        // (`Event` is not `Send`, so it cannot live in the reactor.)
        let mut events: Vec<epoll::Event> = Vec::with_capacity(16);
        match epoll::wait(
            &self.epoll,
            rustix::buffer::spare_capacity(&mut events),
            timeout.map(timespec).as_ref(),
        ) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => return Ok(()),
            Err(err) => return Err(err.into()),
        }
        for ev in &events {
            let token = ev.data.u64() as usize;
            if self.registered.contains_key(&token) {
                ready.push(token);
            }
        }
        Ok(())
    }
}

/// The user_data of `IORING_OP_POLL_REMOVE` submissions (ids start at 1).
#[cfg(target_os = "linux")]
const REMOVE_ID: u64 = 0;
/// The first user_data of the `IORING_OP_TIMEOUT` entries used when `EXT_ARG`
/// is missing (poll ids count up from 1 and never reach this range).
#[cfg(target_os = "linux")]
const TIMEOUT_BASE: u64 = 1 << 63;

/// io_uring based poller (`IORING_OP_POLL_ADD` one-shot polls).
#[cfg(target_os = "linux")]
pub struct UringReactor {
    ring: io_uring::IoUring,
    /// token -> (raw fd, write, id of the armed poll)
    armed: HashMap<usize, (RawFd, bool, u64)>,
    next_id: u64,
    /// Whether waits may carry their timeout as `EXT_ARG` (kernel 5.11+).
    ext_arg: bool,
    /// user_data of the `IORING_OP_TIMEOUT` of the current wait and whether
    /// it has completed (the `EXT_ARG`-less path).
    timeout_id: u64,
    timeout_fired: bool,
}

#[cfg(target_os = "linux")]
impl UringReactor {
    fn new() -> io::Result<UringReactor> {
        // Same ring size and (absent) flags as libxev's `IoUring.init(256, 0)`.
        let ring = io_uring::IoUring::new(256)?;
        let ext_arg = ring.params().is_feature_ext_arg();
        Ok(UringReactor {
            ring,
            armed: HashMap::new(),
            next_id: 1,
            ext_arg,
            timeout_id: TIMEOUT_BASE,
            timeout_fired: false,
        })
    }

    /// A reactor that behaves as on a ring without `EXT_ARG`.
    #[cfg(test)]
    fn new_without_ext_arg() -> io::Result<UringReactor> {
        let mut r = UringReactor::new()?;
        r.ext_arg = false;
        Ok(r)
    }

    /// Queue an entry, flushing the submission queue first if it is full.
    #[allow(unsafe_code)]
    fn push(&mut self, entry: &io_uring::squeue::Entry) -> io::Result<()> {
        // SAFETY: poll add/remove entries reference no user memory: the fd
        // is a plain descriptor number that is valid for the duration of the
        // `wait` call that submits it (the caller borrows it), and the
        // user_data is an opaque id.
        unsafe {
            if self.ring.submission().push(entry).is_err() {
                self.ring.submit()?;
                self.ring
                    .submission()
                    .push(entry)
                    .map_err(|_| io::Error::other("io_uring submission queue full"))?;
            }
        }
        Ok(())
    }

    fn sync(&mut self, interests: &[Interest<'_>]) -> io::Result<()> {
        use io_uring::{opcode, types};

        // Cancel polls whose interest is gone or changed.
        let stale: Vec<usize> = self
            .armed
            .iter()
            .filter(|(token, (raw, write, _))| {
                !interests.iter().any(|i| {
                    i.token == **token
                        && std::os::fd::AsRawFd::as_raw_fd(&i.fd) == *raw
                        && i.write == *write
                })
            })
            .map(|(t, _)| *t)
            .collect();
        for token in stale {
            if let Some((_, _, id)) = self.armed.remove(&token) {
                let entry = opcode::PollRemove::new(id).build().user_data(REMOVE_ID);
                self.push(&entry)?;
            }
        }

        // Arm the new ones.
        for i in interests {
            if self.armed.contains_key(&i.token) {
                continue;
            }
            let raw = std::os::fd::AsRawFd::as_raw_fd(&i.fd);
            let mask = if i.write { libc::POLLOUT } else { libc::POLLIN } as u32;
            let id = self.next_id;
            self.next_id += 1;
            let entry = opcode::PollAdd::new(types::Fd(raw), mask)
                .build()
                .user_data(id);
            self.push(&entry)?;
            self.armed.insert(i.token, (raw, i.write, id));
        }
        Ok(())
    }

    fn reap(&mut self, interests: &[Interest<'_>], ready: &mut Vec<usize>) {
        let mut cq = self.ring.completion();
        cq.sync();
        for cqe in &mut cq {
            let id = cqe.user_data();
            if id == REMOVE_ID {
                continue;
            }
            if id >= TIMEOUT_BASE {
                // A stale (cancelled) timeout of an earlier wait is ignored.
                if id == self.timeout_id {
                    self.timeout_fired = true;
                }
                continue;
            }
            let Some(token) = self
                .armed
                .iter()
                .find(|(_, (_, _, armed_id))| *armed_id == id)
                .map(|(t, _)| *t)
            else {
                // Completion of a poll we already cancelled.
                continue;
            };
            // Polls are one-shot: it must be armed again next time.
            self.armed.remove(&token);
            if interests.iter().any(|i| i.token == token) {
                ready.push(token);
            }
        }
    }

    fn wait(
        &mut self,
        interests: &[Interest<'_>],
        timeout: Option<Duration>,
        ready: &mut Vec<usize>,
    ) -> io::Result<()> {
        use io_uring::types;

        self.sync(interests)?;

        // Results already waiting (e.g. from a previous partial reap).
        let res = match timeout {
            None => self.ring.submit_and_wait(1),
            Some(d) if !self.ext_arg => return self.wait_with_timeout_op(d, interests, ready),
            Some(d) => {
                let ts = types::Timespec::new()
                    .sec(d.as_secs())
                    .nsec(d.subsec_nanos());
                let args = types::SubmitArgs::new().timespec(&ts);
                self.ring.submitter().submit_with_args(1, &args)
            }
        };
        match res {
            Ok(_) => {}
            Err(e) => match e.raw_os_error() {
                Some(libc::ETIME | libc::EINTR | libc::EBUSY) => {}
                _ => return Err(e),
            },
        }
        self.reap(interests, ready);
        Ok(())
    }

    /// Wait with a timeout on a ring without `EXT_ARG`: queue an
    /// `IORING_OP_TIMEOUT` (5.4+, what libxev's timers use) whose completion
    /// ends the wait. Completions that are not for the interests (a cancelled
    /// timeout of an earlier wait) do not end it.
    fn wait_with_timeout_op(
        &mut self,
        d: Duration,
        interests: &[Interest<'_>],
        ready: &mut Vec<usize>,
    ) -> io::Result<()> {
        use io_uring::{opcode, types};

        let ts = types::Timespec::new()
            .sec(d.as_secs())
            .nsec(d.subsec_nanos());
        self.timeout_id = self.timeout_id.wrapping_add(1).max(TIMEOUT_BASE);
        self.timeout_fired = false;
        // The kernel copies the timespec while consuming the entry in
        // `submit_and_wait` below; `ts` outlives that call.
        let entry = opcode::Timeout::new(std::ptr::addr_of!(ts))
            .build()
            .user_data(self.timeout_id);
        self.push(&entry)?;
        loop {
            let res = self.ring.submit_and_wait(1);
            self.reap(interests, ready);
            match res {
                Ok(_) => {}
                Err(e) => match e.raw_os_error() {
                    Some(libc::ETIME | libc::EINTR | libc::EBUSY) => break,
                    _ => return Err(e),
                },
            }
            if self.timeout_fired || !ready.is_empty() {
                break;
            }
        }
        if !self.timeout_fired {
            // Cancel it so it cannot wake a later wait.
            let entry = opcode::TimeoutRemove::new(self.timeout_id)
                .build()
                .user_data(REMOVE_ID);
            self.push(&entry)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::{AsFd, OwnedFd};

    use super::*;

    fn backends() -> Vec<Backend> {
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut v = vec![Backend::Kqueue]; // -> poll
        #[cfg(target_os = "linux")]
        {
            v.push(Backend::Epoll);
            if ghostty_os::xev::available(Backend::IoUring) {
                v.push(Backend::IoUring);
            } else {
                eprintln!("SKIP io_uring reactor: unavailable");
            }
        }
        v
    }

    fn pipe() -> (OwnedFd, OwnedFd) {
        rustix::pipe::pipe().unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn io_uring_without_ext_arg_uses_timeout_op() {
        let Ok(mut r) = Reactor::uring_without_ext_arg() else {
            eprintln!("SKIP io_uring reactor: unavailable");
            return;
        };
        let (rd, wr) = pipe();
        let interest = [Interest {
            token: 7,
            fd: rd.as_fd(),
            write: false,
        }];
        let mut ready = vec![];
        // Times out when nothing is ready, repeatedly (a cancelled timeout of
        // the previous wait must not end the next one early).
        for _ in 0..3 {
            let start = std::time::Instant::now();
            r.wait(&interest, Some(Duration::from_millis(30)), &mut ready)
                .unwrap();
            assert!(ready.is_empty());
            assert!(start.elapsed() >= Duration::from_millis(25));
        }
        // Readiness ends the wait long before the timeout.
        rustix::io::write(&wr, b"x").unwrap();
        let start = std::time::Instant::now();
        r.wait(&interest, Some(Duration::from_secs(10)), &mut ready)
            .unwrap();
        assert_eq!(ready, vec![7]);
        assert!(start.elapsed() < Duration::from_secs(5));
        // The abandoned 10 s timeout does not wake the following wait.
        let mut buf = [0u8; 1];
        rustix::io::read(&rd, &mut buf).unwrap();
        let start = std::time::Instant::now();
        r.wait(&interest, Some(Duration::from_millis(30)), &mut ready)
            .unwrap();
        assert!(ready.is_empty());
        assert!(start.elapsed() >= Duration::from_millis(25));
    }

    #[test]
    fn times_out_with_nothing_ready() {
        for be in backends() {
            let mut r = Reactor::new(be).unwrap();
            let (rd, _wr) = pipe();
            let mut ready = vec![];
            let start = std::time::Instant::now();
            r.wait(
                &[Interest {
                    token: 7,
                    fd: rd.as_fd(),
                    write: false,
                }],
                Some(Duration::from_millis(30)),
                &mut ready,
            )
            .unwrap();
            assert!(ready.is_empty(), "{}", r.name());
            assert!(start.elapsed() >= Duration::from_millis(25), "{}", r.name());
        }
    }

    #[test]
    fn readable_write_and_rearm() {
        for be in backends() {
            let mut r = Reactor::new(be).unwrap();
            let (rd, wr) = pipe();
            let ints = [
                Interest {
                    token: 1,
                    fd: rd.as_fd(),
                    write: false,
                },
                Interest {
                    token: 2,
                    fd: wr.as_fd(),
                    write: true,
                },
            ];
            let mut ready = vec![];
            // The write end is writable immediately; the read end is not.
            r.wait(&ints, Some(Duration::from_millis(500)), &mut ready)
                .unwrap();
            assert_eq!(ready, vec![2], "{}", r.name());

            rustix::io::write(&wr, b"x").unwrap();
            // Level-triggered: still reported on every wait until drained.
            for _ in 0..3 {
                r.wait(&ints[..1], Some(Duration::from_millis(500)), &mut ready)
                    .unwrap();
                assert_eq!(ready, vec![1], "{}", r.name());
            }
            let mut buf = [0u8; 8];
            rustix::io::read(&rd, &mut buf).unwrap();
            r.wait(&ints[..1], Some(Duration::from_millis(20)), &mut ready)
                .unwrap();
            assert!(ready.is_empty(), "{}", r.name());
        }
    }

    #[test]
    fn dropped_interest_is_not_reported_and_can_return() {
        for be in backends() {
            let mut r = Reactor::new(be).unwrap();
            let (rd, wr) = pipe();
            let (rd2, wr2) = pipe();
            let both = [
                Interest {
                    token: 1,
                    fd: rd.as_fd(),
                    write: false,
                },
                Interest {
                    token: 2,
                    fd: rd2.as_fd(),
                    write: false,
                },
            ];
            let mut ready = vec![];
            r.wait(&both, Some(Duration::from_millis(10)), &mut ready)
                .unwrap();
            rustix::io::write(&wr, b"x").unwrap();
            rustix::io::write(&wr2, b"y").unwrap();
            // Only token 2 is wanted now.
            r.wait(&both[1..], Some(Duration::from_millis(500)), &mut ready)
                .unwrap();
            assert_eq!(ready, vec![2], "{}", r.name());
            // Token 1 comes back and is still readable.
            r.wait(&both[..1], Some(Duration::from_millis(500)), &mut ready)
                .unwrap();
            assert_eq!(ready, vec![1], "{}", r.name());
        }
    }

    #[test]
    fn wakes_from_another_thread() {
        for be in backends() {
            let mut r = Reactor::new(be).unwrap();
            let (rd, wr) = pipe();
            let h = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                rustix::io::write(&wr, b"x").unwrap();
                wr
            });
            let mut ready = vec![];
            r.wait(
                &[Interest {
                    token: 3,
                    fd: rd.as_fd(),
                    write: false,
                }],
                None,
                &mut ready,
            )
            .unwrap();
            assert_eq!(ready, vec![3], "{}", r.name());
            drop(h.join().unwrap());
        }
    }

    #[test]
    fn closed_fd_registration_is_replaced() {
        for be in backends() {
            let mut r = Reactor::new(be).unwrap();
            let (rd, wr) = pipe();
            let mut ready = vec![];
            r.wait(
                &[Interest {
                    token: 1,
                    fd: rd.as_fd(),
                    write: false,
                }],
                Some(Duration::from_millis(5)),
                &mut ready,
            )
            .unwrap();
            drop(rd);
            drop(wr);
            // Contract: a token whose fd was closed is omitted from one wait
            // before it may be used with a new fd.
            r.wait(&[], Some(Duration::from_millis(1)), &mut ready)
                .unwrap();
            let (rd, wr) = pipe();
            rustix::io::write(&wr, b"x").unwrap();
            r.wait(
                &[Interest {
                    token: 1,
                    fd: rd.as_fd(),
                    write: false,
                }],
                Some(Duration::from_millis(500)),
                &mut ready,
            )
            .unwrap();
            assert_eq!(ready, vec![1], "{}", r.name());
        }
    }
}
