//! `nicemailer core stop` and `nicemailer core start`: the core of the app,
//! run by launchd as the agent `eu.nicemailer.core` (`docs/app-bundle.md`,
//! "Stopping it").
//!
//! These commands talk to launchd, not to the core: starting a core that does
//! not run cannot go through its socket, and a stop by SIGTERM is the one the
//! core already handles (it finishes its work, closes its files cleanly and
//! leaves with code 0, which `KeepAlive` does not restart). No method of the
//! contract is involved.
//!
//! Both commands wait for the result: when `core stop` returns, the database
//! is free, and when `core start` returns, the core answers. A script can
//! therefore chain them with the next step. Both also succeed when there is
//! nothing to do, so that they can be run twice.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Subcommand;
use nm_api::paths;

/// The label of the agent, as declared by `scripts/make-bundle.sh`.
const LABEL: &str = "eu.nicemailer.core";
/// An absolute path: the command must not depend on the caller's `PATH`.
const LAUNCHCTL: &str = "/bin/launchctl";

/// How long a stop may take. Longer than launchd's own limit before it kills
/// the process (`exit timeout` of the agent), so that a slow stop is reported
/// from what really happened.
const STOP_PATIENCE: Patience = Patience { every: Duration::from_millis(50), limit: Duration::from_secs(30) };
/// How long a start may take before the core answers. A database upgrade does
/// not lengthen it: the core answers on its socket during one.
const START_PATIENCE: Patience = Patience { every: Duration::from_millis(50), limit: Duration::from_secs(15) };

/// `nicemailer core …`.
#[derive(Subcommand)]
pub enum CoreAction {
    /// Stop the core of the app: it finishes its work and leaves. It starts
    /// again with `nicemailer core start`, when the app is opened or comes
    /// back to the front, and at the next session.
    Stop,
    /// Start the core of the app, and wait until it answers.
    Start,
}

/// How often to check, and for how long.
///
/// Rust note: `#[derive(Clone, Copy)]` lets this small value be passed around
/// by copy, without moving it (ownership) or borrowing it.
#[derive(Clone, Copy)]
struct Patience {
    every: Duration,
    limit: Duration,
}

/// What launchd says of the agent.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum AgentState {
    /// launchd does not know the agent: the app was never opened, or it was
    /// turned off in the Login Items.
    NotLoaded,
    /// Known, with no process.
    NotRunning,
    /// Known, with a process.
    Running,
    /// Between the two, such as a core that failed and is about to be started
    /// again by `KeepAlive`.
    Changing,
}

/// The state written by `launchctl print`, a pure function so that it can be
/// tested on real output.
///
/// Only the line at the first level counts: the sections nested in the output
/// have `state = active` lines of their own, indented by two tabs.
fn parse_state(printed: &str) -> AgentState {
    let state = printed.lines().find_map(|line| line.strip_prefix("\tstate = "));
    match state.map(str::trim) {
        Some("running") => AgentState::Running,
        Some("not running") => AgentState::NotRunning,
        _ => AgentState::Changing,
    }
}

/// Whether launchd refuses to start the program, which `launchctl kickstart`
/// does not report: its signature no longer meets the constraint recorded
/// when the app registered the agent (`docs/app-bundle.md`, "Signature").
fn spawn_refused(printed: &str) -> bool {
    printed.lines().any(|line| line.trim() == "job state = spawn failed")
}

/// Runs `launchctl`. A trait, so that the tests play launchd: they must never
/// stop the real core of the machine they run on.
///
/// Rust note: a trait declares what a type can do; the functions below accept
/// any type that implements it (`&impl Launchd`).
trait Launchd {
    /// What the command printed if it succeeded, `None` if launchd refused
    /// it, an error if `launchctl` could not be run at all.
    fn run(&self, args: &[&str]) -> anyhow::Result<Option<String>>;
}

/// The real launchd, for the agent of the logged-in user.
struct System;

impl Launchd for System {
    fn run(&self, args: &[&str]) -> anyhow::Result<Option<String>> {
        let output = Command::new(LAUNCHCTL)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("cannot run {LAUNCHCTL}"))?;
        Ok(output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned()))
    }
}

/// The agent in the domain of the current user: `gui/501/eu.nicemailer.core`.
fn target() -> String {
    // Rust note: `unsafe` is required to call a C function; `getuid` has no
    // precondition and cannot fail.
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}/{LABEL}")
}

fn agent_state(launchd: &impl Launchd, target: &str) -> anyhow::Result<AgentState> {
    Ok(launchd.run(&["print", target])?.as_deref().map_or(AgentState::NotLoaded, parse_state))
}

/// Whether a core answers on this socket. Connecting changes nothing, unlike
/// taking the lock of the mailbox for a moment, which could make a core that
/// is starting believe that another one holds it. The socket file left by a
/// crash refuses the connection.
fn core_answers(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

/// Checks until `done` is true; false if the time ran out.
fn wait_until(patience: Patience, mut done: impl FnMut() -> anyhow::Result<bool>) -> anyhow::Result<bool> {
    let started = Instant::now();
    loop {
        if done()? {
            return Ok(true);
        }
        if started.elapsed() >= patience.limit {
            return Ok(false);
        }
        std::thread::sleep(patience.every);
    }
}

/// What a command did, for the message it prints.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Stopped,
    AlreadyStopped,
    Started,
    AlreadyRunning,
}

impl Outcome {
    fn message(&self) -> &'static str {
        match self {
            Outcome::Stopped => "Core stopped.",
            Outcome::AlreadyStopped => "The core is not running.",
            Outcome::Started => "Core started.",
            Outcome::AlreadyRunning => "The core is already running.",
        }
    }
}

/// `nicemailer core stop`.
fn stop(launchd: &impl Launchd, target: &str, socket: &Path, patience: Patience) -> anyhow::Result<Outcome> {
    match agent_state(launchd, target)? {
        AgentState::Running => {}
        AgentState::Changing => {
            bail!("launchd is restarting the core, which keeps failing: see core.log of the mailbox, then try again")
        }
        // No core of the agent. One that answers was started by hand, and
        // launchd cannot reach it.
        AgentState::NotLoaded | AgentState::NotRunning if core_answers(socket) => {
            bail!("this core was started by hand: stop it where it runs (Ctrl-C)")
        }
        AgentState::NotLoaded | AgentState::NotRunning => return Ok(Outcome::AlreadyStopped),
    }
    // Refused when the core left between the two calls: the wait below then
    // sees that it is gone, which is what was asked.
    launchd.run(&["kill", "SIGTERM", target])?;
    let gone = wait_until(patience, || Ok(agent_state(launchd, target)? != AgentState::Running))?;
    if !gone {
        bail!("the core is still running after {} s: see core.log of the mailbox", patience.limit.as_secs());
    }
    Ok(Outcome::Stopped)
}

/// `nicemailer core start`.
fn start(launchd: &impl Launchd, target: &str, socket: &Path, patience: Patience) -> anyhow::Result<Outcome> {
    if core_answers(socket) {
        return Ok(Outcome::AlreadyRunning);
    }
    // Only the app can register the agent (`SMAppService`).
    if agent_state(launchd, target)? == AgentState::NotLoaded || launchd.run(&["kickstart", target])?.is_none() {
        bail!("the core is not installed as a background item: open NiceMailer once, and allow it in System Settings > General > Login Items if asked");
    }
    if wait_until(patience, || Ok(core_answers(socket)))? {
        return Ok(Outcome::Started);
    }
    let refused = launchd.run(&["print", target])?.is_some_and(|printed| spawn_refused(&printed));
    if refused {
        bail!("launchd refuses to start this build of the core: open NiceMailer, which registers it again");
    }
    bail!("the core did not answer within {} s: see core.log of the mailbox", patience.limit.as_secs())
}

/// The mailbox the agent serves: launchd gives its core no variable, so it is
/// the one chosen in the app, whatever `NICEMAILER_DATA_DIR` says here.
fn agent_data_dir() -> PathBuf {
    let default = paths::default_data_dir();
    let chosen = std::fs::read_to_string(default.join(paths::CHOICE_FILE)).ok();
    paths::resolve(None, chosen.as_deref(), default)
}

/// Whether two paths name one folder, through symbolic links if they exist.
fn same_dir(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    real(a) == real(b)
}

/// Runs `nicemailer core …` on the mailbox the command was given.
pub fn run(data_dir: &Path, action: CoreAction) -> anyhow::Result<()> {
    let agent_dir = agent_data_dir();
    // Checked before launchd is asked anything: stopping the core of the app
    // because of a command aimed at another mailbox would be a surprise.
    if !same_dir(data_dir, &agent_dir) {
        bail!(
            "{} is not the mailbox of the app's core ({}): `nicemailer core` only acts on that core. Stop a core started by hand where it runs (Ctrl-C)",
            data_dir.display(),
            agent_dir.display()
        );
    }
    let socket = agent_dir.join(paths::SOCKET);
    let outcome = match action {
        CoreAction::Stop => stop(&System, &target(), &socket, STOP_PATIENCE)?,
        CoreAction::Start => start(&System, &target(), &socket, START_PATIENCE)?,
    };
    println!("{}", outcome.message());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::os::unix::net::UnixListener;

    /// The output of `launchctl print` for a running agent, shortened from a
    /// real one: the nested `state = active` must not be taken for its state.
    const RUNNING: &str = "gui/501/eu.nicemailer.core = {\n\tactive count = 1\n\tstate = running\n\n\tpid = 1160\n\tendpoints = {\n\t\t\"x\" = {\n\t\t\tport = 0x1\n\t\t}\n\t}\n\tevent channels = {\n\t\tstate = active\n\t}\n}\n";
    const STOPPED: &str = "gui/501/eu.nicemailer.core = {\n\tactive count = 0\n\tstate = not running\n\n\tlast exit code = 0\n\tevent channels = {\n\t\tstate = active\n\t}\n}\n";
    const REFUSED: &str = "gui/501/eu.nicemailer.core = {\n\tstate = not running\n\tjob state = spawn failed\n\tlast exit code = 78: EX_CONFIG\n}\n";
    const SCHEDULED: &str = "gui/501/eu.nicemailer.core = {\n\tstate = spawn scheduled\n}\n";

    const QUICK: Patience = Patience { every: Duration::from_millis(1), limit: Duration::from_millis(40) };

    /// launchd played by a test. `prints` are the answers to `print`, in
    /// order, the last one repeating; `None` is an unknown agent.
    ///
    /// Rust note: `RefCell` allows changes through a shared reference (`&self`
    /// in the trait), checked when the program runs rather than when it
    /// compiles.
    struct FakeLaunchd {
        prints: RefCell<VecDeque<Option<&'static str>>>,
        calls: RefCell<Vec<String>>,
        /// `kickstart` is refused.
        refuse_kickstart: bool,
        /// The socket `kickstart` makes a core answer on.
        starts_on: Option<PathBuf>,
        /// Keeps that core's socket open until the end of the test.
        listener: RefCell<Option<UnixListener>>,
    }

    impl FakeLaunchd {
        fn new(prints: &[Option<&'static str>]) -> Self {
            FakeLaunchd {
                prints: RefCell::new(prints.iter().copied().collect()),
                calls: RefCell::new(Vec::new()),
                refuse_kickstart: false,
                starts_on: None,
                listener: RefCell::new(None),
            }
        }

        /// The commands other than `print`: those that change something.
        fn actions(&self) -> Vec<String> {
            self.calls.borrow().iter().filter(|c| !c.starts_with("print")).cloned().collect()
        }
    }

    impl Launchd for FakeLaunchd {
        fn run(&self, args: &[&str]) -> anyhow::Result<Option<String>> {
            self.calls.borrow_mut().push(args.join(" "));
            Ok(match args[0] {
                "print" => {
                    let mut prints = self.prints.borrow_mut();
                    let current = if prints.len() > 1 { prints.pop_front().unwrap() } else { prints[0] };
                    current.map(str::to_string)
                }
                "kickstart" if self.refuse_kickstart => None,
                "kickstart" => {
                    if let Some(socket) = &self.starts_on {
                        *self.listener.borrow_mut() = Some(UnixListener::bind(socket).unwrap());
                    }
                    Some(String::new())
                }
                _ => Some(String::new()),
            })
        }
    }

    /// A folder for a socket, and the path of that socket.
    fn socket() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(paths::SOCKET);
        (dir, path)
    }

    #[test]
    fn the_state_is_read_at_the_first_level_only() {
        assert_eq!(parse_state(RUNNING), AgentState::Running);
        assert_eq!(parse_state(STOPPED), AgentState::NotRunning);
        assert_eq!(parse_state(SCHEDULED), AgentState::Changing);
        // Only a nested state: not the state of the agent.
        assert_eq!(parse_state("x = {\n\tsub = {\n\t\tstate = running\n\t}\n}\n"), AgentState::Changing);
    }

    #[test]
    fn a_refused_spawn_is_recognized() {
        assert!(spawn_refused(REFUSED));
        assert!(!spawn_refused(STOPPED));
        assert!(!spawn_refused(RUNNING));
    }

    #[test]
    fn stop_sends_sigterm_and_waits_for_the_core_to_leave() {
        let (_dir, sock) = socket();
        let launchd = FakeLaunchd::new(&[Some(RUNNING), Some(RUNNING), Some(RUNNING), Some(STOPPED)]);
        assert_eq!(stop(&launchd, "t", &sock, QUICK).unwrap(), Outcome::Stopped);
        assert_eq!(launchd.actions(), ["kill SIGTERM t"]);
        // The state was read until the core had left.
        assert_eq!(launchd.calls.borrow().iter().filter(|c| c.starts_with("print")).count(), 4);
    }

    #[test]
    fn stop_twice_succeeds_and_sends_nothing() {
        let (_dir, sock) = socket();
        for state in [Some(STOPPED), None] {
            let launchd = FakeLaunchd::new(&[state]);
            assert_eq!(stop(&launchd, "t", &sock, QUICK).unwrap(), Outcome::AlreadyStopped);
            assert!(launchd.actions().is_empty());
        }
    }

    #[test]
    fn stop_reports_a_core_that_does_not_leave() {
        let (_dir, sock) = socket();
        let launchd = FakeLaunchd::new(&[Some(RUNNING)]);
        let error = stop(&launchd, "t", &sock, QUICK).unwrap_err().to_string();
        assert!(error.contains("still running"), "{error}");
    }

    #[test]
    fn stop_refuses_a_core_started_by_hand() {
        let (_dir, sock) = socket();
        let _core = UnixListener::bind(&sock).unwrap();
        for state in [Some(STOPPED), None] {
            let launchd = FakeLaunchd::new(&[state]);
            let error = stop(&launchd, "t", &sock, QUICK).unwrap_err().to_string();
            assert!(error.contains("started by hand"), "{error}");
            assert!(launchd.actions().is_empty());
        }
    }

    #[test]
    fn stop_says_when_launchd_keeps_restarting_a_failing_core() {
        let (_dir, sock) = socket();
        let launchd = FakeLaunchd::new(&[Some(SCHEDULED)]);
        let error = stop(&launchd, "t", &sock, QUICK).unwrap_err().to_string();
        assert!(error.contains("keeps failing"), "{error}");
        assert!(launchd.actions().is_empty());
    }

    #[test]
    fn start_asks_launchd_and_waits_for_the_core_to_answer() {
        let (_dir, sock) = socket();
        let mut launchd = FakeLaunchd::new(&[Some(STOPPED)]);
        launchd.starts_on = Some(sock.clone());
        assert_eq!(start(&launchd, "t", &sock, QUICK).unwrap(), Outcome::Started);
        assert_eq!(launchd.actions(), ["kickstart t"]);
    }

    #[test]
    fn start_does_nothing_when_a_core_answers() {
        let (_dir, sock) = socket();
        let _core = UnixListener::bind(&sock).unwrap();
        let launchd = FakeLaunchd::new(&[Some(RUNNING)]);
        assert_eq!(start(&launchd, "t", &sock, QUICK).unwrap(), Outcome::AlreadyRunning);
        assert!(launchd.calls.borrow().is_empty());
    }

    #[test]
    fn a_socket_left_by_a_crash_is_not_a_running_core() {
        let (_dir, sock) = socket();
        // Rust note: the listener is dropped at once, its file stays behind.
        drop(UnixListener::bind(&sock).unwrap());
        assert!(sock.exists());
        assert!(!core_answers(&sock));
    }

    #[test]
    fn start_explains_an_agent_the_app_has_not_registered() {
        let (_dir, sock) = socket();
        let launchd = FakeLaunchd::new(&[None]);
        let error = start(&launchd, "t", &sock, QUICK).unwrap_err().to_string();
        assert!(error.contains("open NiceMailer once"), "{error}");
        assert!(launchd.actions().is_empty());

        let mut refused = FakeLaunchd::new(&[Some(STOPPED)]);
        refused.refuse_kickstart = true;
        let error = start(&refused, "t", &sock, QUICK).unwrap_err().to_string();
        assert!(error.contains("open NiceMailer once"), "{error}");
    }

    #[test]
    fn start_tells_a_refused_spawn_from_a_silent_core() {
        let (_dir, sock) = socket();
        let launchd = FakeLaunchd::new(&[Some(REFUSED)]);
        let error = start(&launchd, "t", &sock, QUICK).unwrap_err().to_string();
        assert!(error.contains("refuses to start"), "{error}");

        let launchd = FakeLaunchd::new(&[Some(STOPPED)]);
        let error = start(&launchd, "t", &sock, QUICK).unwrap_err().to_string();
        assert!(error.contains("did not answer"), "{error}");
    }

    #[test]
    fn two_spellings_of_a_folder_are_the_same_folder() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(same_dir(&real, &link));
        assert!(same_dir(&real, &real.join(".")));
        assert!(!same_dir(&real, dir.path()));
    }
}
