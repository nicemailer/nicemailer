//! The text of an attachment, read for the AI (`specifications/ia.md`,
//! section 5, `attachment.text`; the reading itself is
//! `nm_store::attachment_text`).
//!
//! # A process of its own
//!
//! PDF and DOCX files are read by parsers of other projects, which a
//! crafted file may crash, loop or make allocate without end
//! (`attachment_text`, "Hostile files"). The core runs them in a child: the
//! same binary, started with `--extract-text <file> --extract-kind <kind>`,
//! which reads the file, writes one JSON line on its standard output and
//! exits. The core kills it after [`TIMEOUT`]. A bad file then costs a
//! failed summary; the core, its database and its other clients go on.
//!
//! Rust note: `tokio::process::Command` starts the child without blocking
//! the runtime, and `kill_on_drop(true)` kills it when the handle is
//! dropped: on the timeout, and when the run waiting on it is cancelled
//! (`ai_cancel` drops the task, hence this future).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use nm_store::attachment_text::{self, Kind, Unreadable};
use tokio::io::AsyncReadExt;

/// Longest reading of one file. A PDF of a hundred pages reads in well
/// under a second; a reader still going after this is stuck.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(20);

/// Most bytes read back from the child: the text is cut at
/// `MAX_TEXT_CHARS` characters, of 4 bytes at most, and the JSON around.
const MAX_OUTPUT: u64 = (attachment_text::MAX_TEXT_CHARS as u64) * 4 * 2 + 4096;

/// The text of the file at `path`, of this kind. Text, HTML and mail are
/// read here, by the code that already reads every mail; PDF and DOCX in
/// a child process (module documentation).
pub(crate) async fn text(path: &Path, kind: Kind) -> Result<String, Unreadable> {
    let size = std::fs::metadata(path).map_err(|e| Unreadable::Damaged(e.to_string()))?.len();
    if size > attachment_text::MAX_FILE_BYTES {
        return Err(Unreadable::TooLarge(size / (1024 * 1024)));
    }
    if !kind.isolated() {
        let path = path.to_path_buf();
        // Rust note: reading and parsing block the thread: `spawn_blocking`
        // runs them on tokio's pool for such work, not on the threads that
        // serve the clients.
        return tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&path).map_err(|e| Unreadable::Damaged(e.to_string()))?;
            attachment_text::extract(&bytes, kind)
        })
        .await
        .unwrap_or_else(|_| Err(Unreadable::Damaged("the reading failed".into())));
    }
    let exe = std::env::current_exe().map_err(|e| Unreadable::Damaged(format!("cannot start the reader: {e}")))?;
    in_child(&exe, path, kind, TIMEOUT).await
}

/// Reads the file in a child process started from `exe`, killed after
/// `timeout`.
pub(crate) async fn in_child(exe: &Path, path: &Path, kind: Kind, timeout: Duration) -> Result<String, Unreadable> {
    let mut child = tokio::process::Command::new(exe)
        .arg("--extract-text")
        .arg(path)
        .arg("--extract-kind")
        .arg(kind.as_str())
        // Nothing of the core's settings or keys is needed to read a file:
        // the child gets an empty environment.
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Unreadable::Damaged(format!("cannot start the reader: {e}")))?;
    let mut stdout = child.stdout.take().expect("piped above");
    let read = async {
        let mut out = Vec::new();
        // Rust note: `take` bounds what is read, so a child gone wrong
        // cannot fill the core's memory through the pipe.
        (&mut stdout).take(MAX_OUTPUT).read_to_end(&mut out).await?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((out, status))
    };
    let (out, status) = match tokio::time::timeout(timeout, read).await {
        // Dropping `child` on return kills it (`kill_on_drop`).
        Err(_) => return Err(Unreadable::Damaged("reading it took too long".into())),
        Ok(Err(e)) => return Err(Unreadable::Damaged(e.to_string())),
        Ok(Ok(r)) => r,
    };
    let answer: Option<Answer> = serde_json::from_slice(&out).ok();
    match answer {
        Some(Answer { text: Some(t), .. }) => Ok(t),
        Some(Answer { error: Some(e), .. }) => Err(e.into()),
        // No answer: the reader crashed (a panic left uncaught, the memory
        // exhausted, a signal).
        _ => Err(Unreadable::Damaged(match status.code() {
            Some(c) => format!("the reader stopped (code {c})"),
            None => "the reader was stopped".into(),
        })),
    }
}

/// The child's one line: the text, or why there is none.
#[derive(serde::Serialize, serde::Deserialize)]
struct Answer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<Failure>,
}

/// [`Unreadable`] through the pipe.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
enum Failure {
    Unsupported,
    TooLarge(u64),
    Encrypted,
    Empty,
    Damaged(String),
}

impl From<Unreadable> for Failure {
    fn from(u: Unreadable) -> Failure {
        match u {
            Unreadable::Unsupported => Failure::Unsupported,
            Unreadable::TooLarge(n) => Failure::TooLarge(n),
            Unreadable::Encrypted => Failure::Encrypted,
            Unreadable::Empty => Failure::Empty,
            Unreadable::Damaged(m) => Failure::Damaged(m),
        }
    }
}

impl From<Failure> for Unreadable {
    fn from(f: Failure) -> Unreadable {
        match f {
            Failure::Unsupported => Unreadable::Unsupported,
            Failure::TooLarge(n) => Unreadable::TooLarge(n),
            Failure::Encrypted => Unreadable::Encrypted,
            Failure::Empty => Unreadable::Empty,
            Failure::Damaged(m) => Unreadable::Damaged(m),
        }
    }
}

/// The child's side: `nicemailer-core --extract-text <file> --extract-kind
/// <kind>`. Writes its answer and returns; `main` then exits, before
/// touching the data directory.
pub(crate) fn child_main(path: &Path, kind: &str) {
    // A panic caught by `attachment_text` must not print its message: the
    // standard error is closed anyway, and the answer says it.
    std::panic::set_hook(Box::new(|_| {}));
    let result = match Kind::parse(kind) {
        None => Err(Unreadable::Unsupported),
        Some(kind) => std::fs::read(path)
            .map_err(|e| Unreadable::Damaged(e.to_string()))
            .and_then(|bytes| attachment_text::extract(&bytes, kind)),
    };
    let answer = match result {
        Ok(t) => Answer { text: Some(t), error: None },
        Err(e) => Answer { text: None, error: Some(e.into()) },
    };
    println!("{}", serde_json::to_string(&answer).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake reader: a shell script with this body, standing for the
    /// core's binary (the test binary is not the core).
    fn script(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("reader.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn the_childs_answer_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.pdf");
        let ok = script(dir.path(), r#"echo '{"text":"Facture"}'"#);
        assert_eq!(in_child(&ok, &file, Kind::Pdf, TIMEOUT).await, Ok("Facture".into()));
        let locked = script(dir.path(), r#"echo '{"error":{"kind":"encrypted"}}'"#);
        assert_eq!(in_child(&locked, &file, Kind::Pdf, TIMEOUT).await, Err(Unreadable::Encrypted));
        let damaged = script(dir.path(), r#"echo '{"error":{"kind":"damaged","detail":"bad xref"}}'"#);
        assert_eq!(in_child(&damaged, &file, Kind::Pdf, TIMEOUT).await, Err(Unreadable::Damaged("bad xref".into())));
    }

    #[tokio::test]
    async fn a_reader_that_crashes_or_hangs_fails_the_reading_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.pdf");
        // A panic outside `catch_unwind`: no answer, Rust's exit code.
        let crashed = script(dir.path(), "exit 101");
        let e = in_child(&crashed, &file, Kind::Pdf, TIMEOUT).await.unwrap_err();
        assert_eq!(e, Unreadable::Damaged("the reader stopped (code 101)".into()));
        // A loop: killed at the delay, and gone afterwards.
        let pid = dir.path().join("pid");
        let hung = script(dir.path(), &format!("echo $$ > {}\nexec sleep 30", pid.display()));
        let started = std::time::Instant::now();
        let e = in_child(&hung, &file, Kind::Pdf, Duration::from_millis(300)).await.unwrap_err();
        assert_eq!(e, Unreadable::Damaged("reading it took too long".into()));
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid: i32 = std::fs::read_to_string(&pid).unwrap().trim().parse().unwrap();
        // Rust note: signal 0 checks that a process exists without sending
        // anything; the child was killed and reaped (`kill_on_drop`), give
        // the system a moment.
        let mut alive = true;
        for _ in 0..50 {
            alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!alive, "the reader {pid} is still running");
    }

    #[tokio::test]
    async fn text_and_html_are_read_in_the_core_and_big_files_not_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.html");
        std::fs::write(&file, "<p>Bonjour</p>").unwrap();
        assert_eq!(text(&file, Kind::Html).await, Ok("Bonjour".into()));
        let big = dir.path().join("big.pdf");
        std::fs::File::create(&big).unwrap().set_len(attachment_text::MAX_FILE_BYTES + 1).unwrap();
        assert_eq!(text(&big, Kind::Pdf).await, Err(Unreadable::TooLarge(50)));
    }
}
