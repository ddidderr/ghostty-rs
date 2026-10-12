#![cfg(unix)]
#![allow(clippy::unwrap_used)]
//! Exec's Flatpak branch (`ghostty_os::flatpak::is_flatpak()` true) against a
//! mock `org.freedesktop.Flatpak.Development` service on a private session
//! bus (`dbus-run-session`): spawn through HostCommand, output via the passed
//! pty fds, exit status propagation and SIGHUP of the process group on stop.
//!
//! One test only: the flatpak override is process-global.

use std::{
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use ghostty_apprt::surface::Message as SurfaceMessage;
use ghostty_config::{
    command::Command as ConfigCommand,
    config::Config,
    types_b::{LinuxCgroup, ShellIntegration},
};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_renderer::{
    message::Message as RendererMessage,
    size::{CellSize, Padding, ScreenSize, Size},
};
use ghostty_termio::{
    Mailbox,
    Options,
    RendererMailbox,
    SharedState,
    SharedStateExt,
    SurfaceMailbox,
    Termio,
    Thread,
    Wakeup,
    backend::{self, Backend},
    exec,
    message::Message,
    termio::DerivedConfig,
};

#[path = "../../ghostty-os/tests/common/flatpak_mock.rs"]
mod flatpak_mock;

struct SurfaceRec(Sender<SurfaceMessage>);

impl SurfaceMailbox for SurfaceRec {
    fn push(&self, msg: SurfaceMessage, _t: Timeout) -> usize {
        let _ = self.0.send(msg);
        1
    }
}

#[derive(Default)]
struct RendererRec {
    resizes: Mutex<Vec<Size>>,
    reset_blinks: AtomicUsize,
}

impl RendererMailbox for RendererRec {
    fn push(&self, msg: RendererMessage, _t: Timeout) -> usize {
        match msg {
            RendererMessage::Resize(s) => self.resizes.lock().unwrap().push(s),
            RendererMessage::ResetCursorBlink => {
                self.reset_blinks.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
        1
    }
}

fn size(cols: u32, rows: u32) -> Size {
    Size {
        screen: ScreenSize {
            width: cols * 10,
            height: rows * 20,
        },
        cell: CellSize {
            width: 10,
            height: 20,
        },
        padding: Padding::default(),
    }
}

struct Fixture {
    state: SharedState,
    mailbox: Mailbox,
    stop: Wakeup,
    thread: Option<JoinHandle<()>>,
    surface: Receiver<SurfaceMessage>,
    _renderer: Arc<RendererRec>,
    _renderer_wakeup: Wakeup,
}

fn base_config() -> Config {
    Config::default()
}

fn start(command: ConfigCommand, tweak: impl FnOnce(&mut Config)) -> Fixture {
    let mut config = base_config();
    tweak(&mut config);

    let mut ecfg = exec::Config::new(
        ghostty_os::env::get_env_map(),
        ghostty_os::osbytes::EncodedOs::from_plain_text("xterm-256color"),
    );
    ecfg.command = Some(command);
    ecfg.shell_integration = ShellIntegration::None;
    // The default config has `linux-cgroup = single-instance` +
    // `gtk-single-instance = detect` which only makes sense after the apprt
    // resolved it.
    ecfg.rt_pre_exec_info.linux_cgroup = LinuxCgroup::Never;
    ecfg.rt_post_fork_info.linux_cgroup = LinuxCgroup::Never;

    let mailbox = Mailbox::new().unwrap();
    let (tx, surface) = unbounded();
    let renderer = Arc::new(RendererRec::default());
    let renderer_wakeup = Wakeup::new().unwrap();
    let termio = Termio::init(Options {
        size: size(80, 30),
        full_config: &config,
        config: DerivedConfig::new(&config),
        backend: Backend::new(backend::Config::Exec(ecfg)).unwrap(),
        mailbox: mailbox.clone(),
        renderer_wakeup: renderer_wakeup.clone(),
        renderer_mailbox: renderer.clone(),
        surface_mailbox: Arc::new(SurfaceRec(tx)),
    })
    .unwrap();
    let state = Arc::clone(termio.renderer_state());

    let thread = Thread::new().unwrap();
    let stop = thread.stop_handle();
    let handle = std::thread::Builder::new()
        .name("io".into())
        .spawn(move || thread.thread_main(termio))
        .unwrap();

    Fixture {
        state,
        mailbox,
        stop,
        thread: Some(handle),
        surface,
        _renderer: renderer,
        _renderer_wakeup: renderer_wakeup,
    }
}

fn sh(script: &str) -> ConfigCommand {
    ConfigCommand::Direct(vec!["/bin/sh".into(), "-c".into(), script.into()])
}

impl Fixture {
    fn text(&self) -> String {
        self.state.lock_terminal().plain_string().unwrap()
    }

    fn send(&self, msg: Message) {
        self.mailbox.send(msg);
        self.mailbox.notify();
    }

    fn write(&self, data: &[u8]) {
        self.send(Message::write_req(data));
    }

    /// Wait until `f` returns Some or time out (panics).
    fn wait_for<T>(&self, what: &str, mut f: impl FnMut(&Fixture) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(v) = f(self) {
                return v;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; terminal text = {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_text(&self, needle: &str) {
        self.wait_for(&format!("text {needle:?}"), |f| {
            f.text().contains(needle).then_some(())
        });
    }

    /// Wait for the ChildExited surface message, returning all messages
    /// received up to and including it.
    fn wait_exit(&self) -> (u32, u64, Vec<SurfaceMessage>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = self
                .surface
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("timed out waiting for child exit; seen={seen:?}"));
            if let SurfaceMessage::ChildExited(c) = msg {
                return (c.exit_code, c.runtime_ms, seen);
            }
            seen.push(msg);
        }
    }

    fn stop(&mut self) {
        self.stop.notify().unwrap();
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.stop.notify();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
#[test]
fn flatpak_exec_spawn_exit_status_and_sighup_on_stop() {
    if !flatpak_mock::under_dbus_session("flatpak_exec_spawn_exit_status_and_sighup_on_stop") {
        return;
    }
    let info = std::env::temp_dir().join(format!("flatpak-info-termio-{}", std::process::id()));
    std::fs::write(&info, "[Application]\nname=com.mitchellh.ghostty\n").unwrap();
    assert!(ghostty_os::flatpak::set_flatpak_info_path(&info));
    assert!(ghostty_os::flatpak::is_flatpak());

    let svc = flatpak_mock::Service::start();

    // Spawn + output + exit status.
    let mut f = start(sh("echo from-host; exit 5"), |_| {});
    let (code, _, _) = f.wait_exit();
    assert_eq!(code, 5);
    f.wait_text("from-host");
    f.stop();
    let spawned = svc.spawned();
    assert_eq!(spawned.len(), 1, "{spawned:?}");
    assert_eq!(spawned[0].argv[0], "/bin/sh");
    assert_eq!(spawned[0].fds, vec![0, 1, 2]);
    assert!(spawned[0].env.contains_key("TERM"), "{:?}", spawned[0].env);

    // Input reaches the host command through the pty.
    let mut f = start(sh("read x; echo got:$x"), |_| {});
    f.write(b"abc\n");
    f.wait_text("got:abc");
    let (code, _, _) = f.wait_exit();
    assert_eq!(code, 0);
    f.stop();

    // Stopping the termio SIGHUPs the host process group.
    let mut f = start(sh("echo up; exec sleep 30"), |_| {});
    f.wait_text("up");
    let pid = svc.spawned().last().unwrap().pid;
    assert_eq!(svc.signals(), vec![]);
    f.stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !svc.signals().contains(&(pid, 1, true)) {
        assert!(Instant::now() < deadline, "no SIGHUP: {:?}", svc.signals());
        std::thread::sleep(Duration::from_millis(20));
    }
    // ... and the host process is gone.
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::path::Path::new(&format!("/proc/{pid}")).exists()
        && !std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z"))
    {
        assert!(Instant::now() < deadline, "host command still alive");
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = std::fs::remove_file(&info);
}
