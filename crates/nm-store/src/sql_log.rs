//! SQL query log, for debugging only. With `NICEMAILER_SQL_LOG` in the
//! environment, every statement a connection of the store runs is written
//! with its parameters bound (`sqlite3_trace_v2`, `SQLITE_TRACE_STMT`):
//! `1` (or `stderr`) writes to standard error, any other value is a file
//! path, appended to. The variable unset, nothing is installed and a query
//! costs exactly what it costs without this module.
//!
//! The log carries subjects, addresses and search words: never on by
//! default. All connections are covered, the writing one (`Store::init`)
//! and the readers (`Store::open_reader`, `pool.rs`).

use std::io::Write;
use std::sync::{Mutex, OnceLock};

use rusqlite::trace::{TraceEvent, TraceEventCodes};
use rusqlite::Connection;

/// The sink shared by every connection, decided once from the environment.
/// `None`: `NICEMAILER_SQL_LOG` unset, no log.
static SINK: OnceLock<Option<Mutex<Box<dyn Write + Send>>>> = OnceLock::new();

fn sink() -> &'static Option<Mutex<Box<dyn Write + Send>>> {
    SINK.get_or_init(|| {
        let w: Box<dyn Write + Send> = match std::env::var("NICEMAILER_SQL_LOG") {
            Err(_) => return None,
            Ok(v) if v.is_empty() || v == "1" || v == "stderr" => Box::new(std::io::stderr()),
            Ok(path) => match std::fs::File::options().create(true).append(true).open(&path) {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!("NICEMAILER_SQL_LOG : impossible d'ouvrir {path} ({e}), journal sur stderr");
                    Box::new(std::io::stderr())
                }
            },
        };
        Some(Mutex::new(w))
    })
}

/// The trace callback. `trace_v2` asks for a plain `fn`, not a closure:
/// the sink is the static above.
fn on_event(event: TraceEvent<'_>) {
    if let TraceEvent::Stmt(stmt, _) = event {
        // `expanded_sql` has the bound parameters, `sql` the raw text.
        let sql = stmt.expanded_sql().unwrap_or_else(|| stmt.sql().into_owned());
        if let Some(sink) = sink()
            && let Ok(mut w) = sink.lock()
        {
            let _ = writeln!(w, "{sql}");
        }
    }
}

/// Installs the log on `conn`, when `NICEMAILER_SQL_LOG` is set. Called
/// wherever a connection is opened (`Store::init`, `Store::open_reader`).
pub(crate) fn install(conn: &Connection) {
    if sink().is_some() {
        conn.trace_v2(TraceEventCodes::SQLITE_TRACE_STMT, Some(on_event));
    }
}
