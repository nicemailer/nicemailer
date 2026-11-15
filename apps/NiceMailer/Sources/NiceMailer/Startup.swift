import Foundation
import os

/// How long the launch of the app took, until the first list was shown
/// (`AppModel.startupPhase`): written once to the system log, category
/// `startup`, so that a slow launch can be measured on the user's Mac
/// (`log show --predicate 'subsystem == "eu.nicemailer.app"' --last 5m`)
/// rather than guessed. The core writes the same for its own part in its
/// log (`startup: socket ready after …`).
struct StartupTimings: Equatable {
    /// From the model's creation to the socket answering.
    let socket: Duration
    /// From the model's creation to the first list shown.
    let firstList: Duration
    /// Whether the core had to be started (no core answered at first).
    let startedCore: Bool
    /// Rows in that first list.
    let rows: Int

    /// The log line. A pure function, tested.
    var line: String {
        let socketMs = Self.milliseconds(socket)
        let listMs = Self.milliseconds(firstList)
        let core = startedCore ? "started by the app" : "already running"
        return "first list after \(listMs) ms (\(rows) rows; socket after \(socketMs) ms, core \(core))"
    }

    static func milliseconds(_ d: Duration) -> Int64 {
        // Swift note: a `Duration` is seconds plus attoseconds (10⁻¹⁸ s).
        d.components.seconds * 1000 + d.components.attoseconds / 1_000_000_000_000_000
    }

    static let log = Logger(subsystem: "eu.nicemailer.app", category: "startup")
}
