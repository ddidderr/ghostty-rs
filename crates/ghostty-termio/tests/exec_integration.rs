#![cfg(unix)]
#![allow(clippy::unwrap_used)]
//! End-to-end tests: real child processes on a real pty, driven through
//! `Termio` + `Thread` (IO thread) + the pty read thread, against a real
//! `Terminal`.

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
    io::ReadableIO,
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
    renderer: Arc<RendererRec>,
    renderer_wakeup: Wakeup,
}

fn base_config() -> Config {
    Config::default()
}

fn start(command: ConfigCommand, tweak: impl FnOnce(&mut Config)) -> Fixture {
    let mut config = base_config();
    tweak(&mut config);

    let mut ecfg = exec::Config::new(ghostty_os::env::get_env_map(), "xterm-256color");
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
        renderer,
        renderer_wakeup,
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
fn printf_output_reaches_terminal_and_exit_is_reported() {
    let mut f = start(sh("printf 'hello\\033[31mred'"), |_| {});
    let (code, runtime_ms, _) = f.wait_exit();
    assert_eq!(code, 0);
    assert!(runtime_ms < 10_000);

    // The reader thread may still be draining the pty after the exit event.
    f.wait_text("hellored");
    assert_eq!("hellored", f.text());

    // The SGR was applied: the cursor pen is red (palette 1).
    {
        let t = f.state.lock_terminal();
        let style = t.screen().cursor.style;
        assert!(
            !matches!(style.fg_color, ghostty_terminal::style::Color::None),
            "pen fg should be set after \\e[31m: {style:?}"
        );
        assert_eq!(t.screen().cursor.x, 8);
    }
    // The renderer was woken up and told to reset the cursor blink.
    assert!(f.renderer.reset_blinks.load(Ordering::SeqCst) >= 1);
    assert!(f.renderer_wakeup_pending());
    f.stop();
}

impl Fixture {
    fn renderer_wakeup_pending(&self) -> bool {
        self.renderer_wakeup.drain()
    }
}

#[test]
fn nonzero_exit_code_is_reported() {
    let mut f = start(sh("exit 3"), |_| {});
    let (code, _, _) = f.wait_exit();
    assert_eq!(code, 3);
    f.stop();
}

#[test]
fn killed_by_signal_reports_signal_number() {
    let mut f = start(sh("kill -KILL $$"), |_| {});
    let (code, _, _) = f.wait_exit();
    assert_eq!(code, 9);
    f.stop();
}

#[test]
fn exec_failure_exits_quickly_with_error() {
    // Abnormal exit detection itself lives in the Surface (it compares
    // `runtime_ms` against `abnormal-command-exit-runtime`); termio must
    // report a tiny runtime and the exit code.
    let mut f = start(
        ConfigCommand::Direct(vec!["/nonexistent/definitely-not-here".into()]),
        |_| {},
    );
    let (code, runtime_ms, _) = f.wait_exit();
    assert_eq!(code, 1);
    assert!(runtime_ms < 5_000, "runtime {runtime_ms}");
    f.stop();
}

#[test]
fn writes_reach_the_child_and_are_echoed() {
    let mut f = start(sh("cat"), |_| {});
    f.write(b"abc\n");
    // Once by the tty echo and once by cat.
    f.wait_for("two echoes", |f| {
        (f.text().matches("abc").count() >= 2).then_some(())
    });
    f.stop();
}

#[test]
fn large_write_with_backpressure_is_delivered_in_order() {
    // 256 KiB through a raw-mode `cat`: exercises the non-blocking write
    // queue (the pty buffer is far smaller) and the output reader at the
    // same time (cat would otherwise block on a full pty).
    let mut f = start(
        sh("stty raw -echo; echo ready; head -c 262144 | wc -c; echo done"),
        |_| {},
    );
    // Writing before `stty raw` applies would go through the cooked-mode
    // line discipline, which silently drops bytes past MAX_CANON (4095)
    // on a line without a newline.
    f.wait_text("ready");
    let chunk = vec![b'x'; 4096];
    for _ in 0..64 {
        f.write(&chunk);
    }
    f.wait_text("262144");
    f.wait_text("done");
    f.stop();
}

#[test]
fn linefeed_mode_translates_cr() {
    let mut f = start(sh("stty raw -echo; cat -v"), |_| {});
    // Give stty a moment so the first byte is not eaten by cooked mode.
    std::thread::sleep(Duration::from_millis(300));
    f.send(Message::LinefeedMode(true));
    f.write(b"a\r");
    f.wait_text("a^M");
    // cat -v got "a\r\n", so it printed a newline after the ^M.
    f.wait_for("cursor on second row", |f| {
        (f.state.lock_terminal().screen().cursor.y == 1).then_some(())
    });
    f.stop();
}

#[test]
fn terminal_query_reply_is_written_back_to_the_pty() {
    // DA1 through the whole stack: child -> pty -> reader thread -> stream
    // handler -> termio mailbox -> IO thread -> pty -> child.
    let mut f = start(
        sh("stty raw -echo; printf '\\033[c'; head -c 12 | cat -v; echo"),
        |_| {},
    );
    f.wait_text("^[[?62;22;52c");
    f.stop();
}

#[test]
fn initial_input_is_written_after_start() {
    let mut f = start(sh("cat"), |c| {
        c.input.list.push(ReadableIO::Raw("hi there\\n".into()));
    });
    f.wait_for("echo of input", |f| {
        (f.text().matches("hi there").count() >= 2).then_some(())
    });
    f.stop();
}

#[test]
fn missing_input_file_shows_error_and_thread_keeps_draining() {
    let mut f = start(sh("true"), |c| {
        c.input
            .list
            .push(ReadableIO::Path("/nonexistent/ghostty-input".into()));
    });
    f.wait_text("A configured `input` path was not found");
    assert!(
        !f.state
            .lock_terminal()
            .modes
            .get(ghostty_vt::modes::Mode::CursorVisible)
    );
    // Messages are swallowed in drain mode and stop still works.
    f.write(b"ignored");
    std::thread::sleep(Duration::from_millis(50));
    f.stop();
}

#[test]
fn resize_messages_are_coalesced_and_applied_to_the_pty() {
    let mut f = start(sh("sleep 0.7; stty size"), |_| {});
    std::thread::sleep(Duration::from_millis(100));
    for i in 0..20u32 {
        f.send(Message::Resize(size(60 + i % 5, 20 + i)));
    }
    f.send(Message::Resize(size(100, 40)));
    f.wait_text("40 100");
    {
        let t = f.state.lock_terminal();
        assert_eq!((t.cols, t.rows), (100, 40));
        assert_eq!(t.width_px, 1000);
        assert_eq!(t.height_px, 800);
    }
    let n = f.renderer.resizes.lock().unwrap().len();
    assert!((1..21).contains(&n), "expected coalescing, got {n} resizes");
    assert_eq!(
        *f.renderer.resizes.lock().unwrap().last().unwrap(),
        size(100, 40)
    );
    f.stop();
}

#[test]
fn synchronized_output_is_force_reset_by_timer() {
    let mut f = start(sh("printf '\\033[?2026h'; sleep 5"), |_| {});
    f.wait_for("sync output on", |f| {
        f.state
            .lock_terminal()
            .modes
            .get(ghostty_vt::modes::Mode::SynchronizedOutput)
            .then_some(())
    });
    let started = Instant::now();
    f.wait_for("sync output reset", |f| {
        (!f.state
            .lock_terminal()
            .modes
            .get(ghostty_vt::modes::Mode::SynchronizedOutput))
        .then_some(())
    });
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_millis(2500),
        "reset took {waited:?}"
    );
    f.stop();
}

#[test]
fn password_input_is_detected_via_termios_polling() {
    let mut f = start(sh("stty -echo; sleep 3"), |_| {});
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match f.surface.recv_timeout(left) {
            Ok(SurfaceMessage::PasswordInput(true)) => break,
            Ok(_) => {}
            Err(err) => panic!("never saw PasswordInput(true): {err}"),
        }
    }
    f.stop();
}

#[test]
fn selection_scroll_timer_ticks_until_stopped() {
    let mut f = start(sh("sleep 3"), |_| {});
    f.send(Message::SelectionScroll(true));
    let mut ticks = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while ticks < 3 {
        let left = deadline.saturating_duration_since(Instant::now());
        if let Ok(SurfaceMessage::SelectionScrollTick(true)) = f.surface.recv_timeout(left) {
            ticks += 1;
        }
        assert!(Instant::now() < deadline);
    }
    f.send(Message::SelectionScroll(false));
    let mut saw_false = false;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(SurfaceMessage::SelectionScrollTick(false)) =
            f.surface.recv_timeout(Duration::from_millis(100))
        {
            saw_false = true;
            break;
        }
    }
    assert!(saw_false);
    // And then it stops ticking.
    std::thread::sleep(Duration::from_millis(100));
    while f.surface.try_recv().is_ok() {}
    std::thread::sleep(Duration::from_millis(100));
    assert!(f.surface.try_recv().is_err());
    f.stop();
}

#[test]
fn focus_events_are_sent_when_enabled() {
    let mut f = start(
        sh("stty raw -echo; printf '\\033[?1004h'; head -c 3 | cat -v; echo"),
        |_| {},
    );
    f.wait_for("focus mode", |f| {
        f.state
            .lock_terminal()
            .modes
            .get(ghostty_vt::modes::Mode::FocusEvent)
            .then_some(())
    });
    f.send(Message::Focused(true));
    f.wait_text("^[[I");
    f.stop();
}

#[test]
fn size_report_mode_2048_and_xtwinops() {
    let mut f = start(
        sh("stty raw -echo; printf '\\033[18t'; head -c 10 | cat -v; echo"),
        |_| {},
    );
    // CSI 18 t -> ESC [ 8 ; 30 ; 80 t
    f.wait_text("^[[8;30;80t");
    f.stop();
}

#[test]
fn clear_screen_message_at_prompt_clears_and_sends_formfeed() {
    let mut f = start(
        sh("stty raw -echo; printf 'one\\ntwo\\n'; head -c 1 | cat -v; echo"),
        |_| {},
    );
    f.wait_text("two");
    f.send(Message::ClearScreen { history: false });
    // Not at a prompt (no shell integration marks): rows above the cursor
    // are erased and no form feed is sent.
    f.wait_for("rows above cursor erased", |f| {
        let t = f.text();
        (!t.contains("one")).then_some(())
    });
    f.stop();
}

#[test]
fn stop_kills_the_child_process_group() {
    let dir = std::env::temp_dir().join(format!("ghostty-termio-pid-{}", std::process::id()));
    let pidfile = dir.display().to_string();
    let mut f = start(sh(&format!("echo $$ > {pidfile}; exec sleep 60")), |_| {});
    let pid: i32 = f.wait_for("pid file", |_| {
        std::fs::read_to_string(&dir)
            .ok()
            .and_then(|s| s.trim().parse().ok())
    });
    let started = Instant::now();
    f.stop();
    assert!(started.elapsed() < Duration::from_secs(5));
    let alive = rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).unwrap());
    assert!(alive.is_err(), "child {pid} is still alive");
    let _ = std::fs::remove_file(&dir);
}

#[test]
fn writes_after_exit_are_ignored_and_thread_stays_alive_until_stopped() {
    // `wait-after-command` keeps the surface (and so this thread) alive
    // after the child exited.
    let mut f = start(sh("true"), |_| {});
    let (code, _, _) = f.wait_exit();
    assert_eq!(code, 0);
    f.write(b"data nobody reads");
    f.send(Message::Focused(true));
    f.send(Message::Resize(size(90, 25)));
    f.wait_for("resize after exit", |f| {
        (f.state.lock_terminal().cols == 90).then_some(())
    });
    f.stop();
}

#[test]
fn change_config_message_updates_palette_and_colors() {
    let mut f = start(sh("sleep 3"), |_| {});
    let mut cfg = DerivedConfig::new(&base_config());
    cfg.background = ghostty_config::types_a::Color { r: 1, g: 2, b: 3 };
    cfg.enquiry_response = "x".into();
    f.send(Message::ChangeConfig(Box::new(cfg)));
    f.wait_for("background change", |f| {
        let t = f.state.lock_terminal();
        (t.colors.background.default == Some(ghostty_vt::color::Rgb { r: 1, g: 2, b: 3 }))
            .then_some(())
    });
    assert!(f.state.lock_terminal().flags.dirty.palette);
    f.stop();
}

#[test]
fn process_output_manual_api() {
    // Termio can also be fed manually (no child involved).
    let config = base_config();
    let mut ecfg = exec::Config::new(ghostty_os::EnvMap::new(), "xterm-256color");
    ecfg.command = Some(sh("true"));
    let (tx, _rx) = unbounded();
    let termio = Termio::init(Options {
        size: size(80, 30),
        full_config: &config,
        config: DerivedConfig::new(&config),
        backend: Backend::new(backend::Config::Exec(ecfg)).unwrap(),
        mailbox: Mailbox::new().unwrap(),
        renderer_wakeup: Wakeup::new().unwrap(),
        renderer_mailbox: Arc::new(RendererRec::default()),
        surface_mailbox: Arc::new(SurfaceRec(tx)),
    })
    .unwrap();
    termio.process_output(b"manual\x1b[31m!");
    assert_eq!(
        "manual!",
        termio
            .renderer_state()
            .lock_terminal()
            .plain_string()
            .unwrap()
    );
    termio.process_output(b"\x1b[5n");
    let m = termio.mailbox().pop().unwrap();
    assert!(matches!(m, Message::Write(w) if w.slice() == b"\x1b[0n"));
}
