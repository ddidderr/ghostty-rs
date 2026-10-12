#![cfg(windows)]
#![allow(clippy::unwrap_used, unsafe_code)]
//! End-to-end tests of the Windows backend: real `cmd.exe` children on a real
//! ConPTY, driven through `Termio` + `Thread` (IO thread: `WaitForMultipleObjects`
//! loop) + the pipe read thread, against a real `Terminal`. Counterpart of
//! `exec_integration` (Unix). Runs under Wine via `scripts/windows-wine-test.sh`.

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
    types_b::ShellIntegration,
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
    wakeups: AtomicUsize,
}

impl RendererMailbox for RendererRec {
    fn push(&self, msg: RendererMessage, _t: Timeout) -> usize {
        match msg {
            RendererMessage::Resize(s) => self.resizes.lock().unwrap().push(s),
            RendererMessage::ResetCursorBlink => {
                self.wakeups.fetch_add(1, Ordering::SeqCst);
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
}

fn start(command: ConfigCommand) -> Fixture {
    start_with(command, |_| {})
}

fn start_with(command: ConfigCommand, tweak: impl FnOnce(&mut exec::Config)) -> Fixture {
    let config = Config::default();
    let mut ecfg = exec::Config::new(
        ghostty_os::env::get_env_map(),
        ghostty_os::osbytes::EncodedOs::from_plain_text("xterm-256color"),
    );
    ecfg.command = Some(command);
    ecfg.shell_integration = ShellIntegration::None;
    // A short cwd keeps the cmd.exe prompt from wrapping at 80 columns.
    ecfg.working_directory = Some(ghostty_os::osbytes::EncodedOs::from_plain_text(r"C:\"));
    tweak(&mut ecfg);

    let mailbox = Mailbox::new().unwrap();
    let (tx, surface) = unbounded();
    let renderer = Arc::new(RendererRec::default());
    let termio = Termio::init(Options {
        size: size(80, 30),
        full_config: &config,
        config: DerivedConfig::new(&config),
        backend: Backend::new(backend::Config::Exec(ecfg)).unwrap(),
        mailbox: mailbox.clone(),
        renderer_wakeup: Wakeup::new().unwrap(),
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
    }
}

fn cmd(args: &[&str]) -> ConfigCommand {
    let mut v = vec!["cmd.exe".into()];
    v.extend(args.iter().map(|s| (*s).into()));
    ConfigCommand::Direct(v)
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

    fn wait_for<T>(&self, what: &str, mut f: impl FnMut(&Fixture) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(v) = f(self) {
                return v;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; terminal text = {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_text(&self, needle: &str) {
        self.wait_for(&format!("text {needle:?}"), |f| {
            f.text().contains(needle).then_some(())
        });
    }

    /// Wait for the ChildExited surface message: (exit code, runtime ms).
    fn wait_exit(&self) -> (u32, u64) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = self.surface.recv_timeout(left).unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for child exit; seen={seen:?}; text={:?}",
                    self.text()
                )
            });
            if let SurfaceMessage::ChildExited(c) = msg {
                return (c.exit_code, c.runtime_ms);
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
fn echo_output_reaches_terminal_and_exit_is_reported() {
    let mut f = start(cmd(&["/c", "echo hi"]));
    let (code, runtime_ms) = f.wait_exit();
    assert_eq!(code, 0);
    assert!(runtime_ms < 30_000);
    // The reader thread may still be draining the pipe after the exit event.
    f.wait_text("hi");
    assert!(f.renderer.wakeups.load(Ordering::SeqCst) >= 1);
    f.stop();
}

#[test]
fn nonzero_exit_code_is_reported() {
    let mut f = start(cmd(&["/c", "exit 7"]));
    let (code, _) = f.wait_exit();
    assert_eq!(code, 7);
    f.stop();
}

#[test]
fn exec_failure_is_reported_in_the_terminal() {
    // Unlike the Unix fork+exec path (child exits with 1), `CreateProcessW`
    // failing is a start error: the IO thread prints the error into the
    // terminal and goes into drain mode (same as Zig's `threadEnter`).
    let mut f = start(ConfigCommand::Direct(vec![
        r"C:\definitely\not\here.exe".into(),
    ]));
    f.wait_text("error starting IO thread");
    f.wait_text("This terminal is non-functional");
    // Messages are swallowed in drain mode and stop still works.
    f.write(b"ignored");
    f.stop();
}

#[test]
fn shell_command_runs_through_windir_cmd() {
    // `ConfigCommand::Shell` => `%WINDIR%\System32\cmd.exe /C <command>`.
    let mut f = start(ConfigCommand::Shell("echo shell-cmd-ok & exit 4".into()));
    let (code, _) = f.wait_exit();
    assert_eq!(code, 4);
    f.wait_text("shell-cmd-ok");
    f.stop();
}

#[test]
fn writes_reach_the_child_and_exit_code_follows() {
    // Interactive cmd.exe: type a command, then exit with a code.
    let mut f = start(cmd(&[]));
    f.write(b"echo mk1\r\n");
    f.wait_for("command output", |f| {
        // Once echoed by conhost with the prompt, once printed by `echo`.
        (f.text().matches("mk1").count() >= 2).then_some(())
    });
    f.write(b"exit 5\r\n");
    let (code, _) = f.wait_exit();
    assert_eq!(code, 5);
    f.stop();
}

#[test]
fn large_write_is_delivered_in_order() {
    // ~120 KiB typed into an interactive cmd.exe: exercises the writer
    // thread's overlapped WriteFile queue together with the reader.
    let mut f = start(cmd(&[]));
    f.wait_text(">");
    let mut data = Vec::new();
    for i in 0..2000 {
        data.extend_from_slice(format!("rem {i:05} {}\r\n", "x".repeat(50)).as_bytes());
    }
    data.extend_from_slice(b"echo done-%OS:~0,1%-end\r\n");
    for chunk in data.chunks(4096) {
        f.write(chunk);
    }
    // `%OS%` is Windows_NT: the expansion proves cmd.exe executed the line
    // after consuming every preceding byte.
    f.wait_text("done-W-end");
    f.write(b"exit 0\r\n");
    f.wait_exit();
    f.stop();
}

/// Not a test by itself: the child process of `resize_is_applied_to_the_pseudo_console`
/// (this test binary re-run on the pseudo console). Prints the console window
/// size on startup and after every line read from stdin; `q` quits.
#[test]
fn child_console_size() {
    use std::io::BufRead;

    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO,
        GetConsoleScreenBufferInfo,
        GetStdHandle,
        STD_OUTPUT_HANDLE,
    };
    if std::env::var_os("GHOSTTY_WINE_CHILD").is_none() {
        return;
    }
    let report = || {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        let ok =
            unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &raw mut info) };
        let w = i32::from(info.srWindow.Right) - i32::from(info.srWindow.Left) + 1;
        let h = i32::from(info.srWindow.Bottom) - i32::from(info.srWindow.Top) + 1;
        println!("SIZE ok={ok} {w}x{h} END");
    };
    report();
    for line in std::io::stdin().lock().lines() {
        if line.unwrap_or_default().trim() == "q" {
            break;
        }
        report();
    }
}

#[test]
fn resize_message_resizes_terminal_and_pseudo_console_call_succeeds() {
    let mut f = start(cmd(&[]));
    f.wait_text(">");
    for i in 0..10u32 {
        f.send(Message::Resize(size(60 + i, 20 + i)));
    }
    f.send(Message::Resize(size(100, 40)));
    f.wait_for("terminal resize", |f| {
        let t = f.state.lock_terminal();
        ((t.cols, t.rows) == (100, 40)).then_some(())
    });
    assert_eq!(
        *f.renderer.resizes.lock().unwrap().last().unwrap(),
        size(100, 40)
    );
    // The child is still alive and answering after ResizePseudoConsole.
    f.write(b"echo after-resize\r\n");
    f.wait_for("echo after resize", |f| {
        (f.text().matches("after-resize").count() >= 2).then_some(())
    });
    f.write(b"exit 0\r\n");
    f.wait_exit();
    f.stop();
}

#[test]
#[cfg_attr(
    wine,
    ignore = "wine: ResizePseudoConsole is a stub (\"fixme:console:ResizePseudoConsole (100,40) stub, faking success\"), the child never sees the new size"
)]
fn resize_is_applied_to_the_pseudo_console() {
    let exe = std::env::current_exe().unwrap().display().to_string();
    let mut f = start_with(
        ConfigCommand::Direct(vec![
            exe.into(),
            "--exact".into(),
            "child_console_size".into(),
            "--nocapture".into(),
        ]),
        |c| {
            c.env.insert("GHOSTTY_WINE_CHILD", "1");
        },
    );
    // size(80, 30) is what the pty was created with.
    f.wait_text("SIZE ok=1 80x30 END");
    f.send(Message::Resize(size(100, 40)));
    f.wait_for("terminal resize", |f| {
        let t = f.state.lock_terminal();
        ((t.cols, t.rows) == (100, 40)).then_some(())
    });
    // ResizePseudoConsole is synchronous with the message; ask the child.
    f.write(b"\r\n");
    f.wait_text("SIZE ok=1 100x40 END");
    f.write(b"q\r\n");
    let (code, _) = f.wait_exit();
    assert_eq!(code, 0);
    f.stop();
}

#[test]
fn stop_terminates_a_running_child() {
    let mut f = start(cmd(&[]));
    f.wait_text(">");
    let started = Instant::now();
    f.stop();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "stop took {:?}",
        started.elapsed()
    );
}

#[test]
fn writes_after_exit_are_ignored_and_thread_stays_alive_until_stopped() {
    let mut f = start(cmd(&["/c", "exit 0"]));
    let (code, _) = f.wait_exit();
    assert_eq!(code, 0);
    f.write(b"data nobody reads");
    f.send(Message::Focused(true));
    f.send(Message::Resize(size(90, 25)));
    f.wait_for("resize after exit", |f| {
        (f.state.lock_terminal().cols == 90).then_some(())
    });
    f.stop();
}
