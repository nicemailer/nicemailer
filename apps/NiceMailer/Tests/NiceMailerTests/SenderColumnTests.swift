import Foundation
import SwiftUI
import Testing
@testable import NiceMailer

/// The sender column of the list (`ui.md`, "Liste"): the person the
/// conversation is with, marked "→" when the user wrote the latest mail.
/// Before, the column showed the author of the latest mail: the user
/// themselves as soon as they had answered.
@MainActor
struct SenderColumnTests {
    /// A row as the core sends it, decoded as `CoreClient` does.
    func row(_ fields: [String: Any]) throws -> ThreadSummary {
        var json = Fixture.thread(1)
        json.merge(fields) { _, new in new }
        let data = try JSONSerialization.data(withJSONObject: json)
        return try CoreClient.decoder.decode(ThreadSummary.self, from: data)
    }

    @Test func anAnsweredConversationNamesThePersonAnsweredWithTheArrow() throws {
        let t = try row(["participants": ["Pascal Potel", "Achille"], "correspondent": "Pascal Potel", "last_is_mine": true])
        #expect(t.correspondentName == "Pascal Potel")
        #expect(SenderColumn.plain(t) == "→ Pascal Potel")
        // The arrow is drawn muted, the name in the row's colour.
        let text = SenderColumn.text(t)
        #expect(String(text.characters) == "→ Pascal Potel")
        let runs = text.runs.map { (String(text[$0.range].characters), $0.foregroundColor) }
        #expect(runs.count == 2)
        #expect(runs[0].0 == "→ " && runs[0].1 == Palette.muted)
        #expect(runs[1].0 == "Pascal Potel" && runs[1].1 == nil)
        // In the language of the machine running the tests.
        #expect(SenderColumn.spoken(t) == L("%@, you wrote last", "Pascal Potel"))
        #expect(SenderColumn.spoken(t) != "Pascal Potel")
    }

    @Test func aConversationWaitingOnTheUserShowsTheNameAlone() throws {
        let t = try row(["participants": ["Me", "Alice"], "correspondent": "Alice", "last_is_mine": false])
        #expect(SenderColumn.plain(t) == "Alice")
        #expect(String(SenderColumn.text(t).characters) == "Alice")
        #expect(SenderColumn.spoken(t) == "Alice")
    }

    @Test func withoutCorrespondentTheLatestParticipantIsShown() throws {
        // A conversation whose mails name nobody else (`correspondent`
        // null): the column is not left empty.
        let t = try row(["participants": ["Alice", "Bob"], "correspondent": NSNull(), "last_is_mine": false])
        #expect(SenderColumn.plain(t) == "Bob")
        let empty = try row(["participants": [String](), "correspondent": NSNull(), "last_is_mine": false])
        #expect(SenderColumn.plain(empty) == "")
    }

    @Test func theKindOfTheCorrespondentIsDecodedAndOptional() throws {
        // As the core sends it (`ThreadSummary::correspondent_kind`).
        let person = try row(["correspondent_kind": "person"])
        #expect(person.correspondentKind == "person")
        #expect(SenderColumn.kindSymbol(person) == "person.fill")
        let robot = try row(["correspondent_kind": "robot"])
        #expect(SenderColumn.kindSymbol(robot) == "gearshape.fill")
        // No verdict, or an older core that does not send the key: no
        // icon, and the row still decodes.
        let unknown = try row(["correspondent_kind": NSNull()])
        #expect(unknown.correspondentKind == nil && SenderColumn.kindSymbol(unknown) == nil)
        #expect(try row([:]).correspondentKind == nil)
        // The kind survives the copy of a row kept in a filtered list.
        #expect(person.with(unreadCount: 0, isFlagged: true).correspondentKind == "person")
        // Outside Quarantine VoiceOver reads the name alone.
        #expect(SenderColumn.spoken(person) == "Sender 1")
    }

    @Test func theListMarksTheAnsweredThread() async {
        // Through the model, as the list draws it: thread 3 of the fixture
        // is the one the user answered.
        let core = FakeCore(handler: Fixture.answer)
        let model = AppModel()
        await model.attach(core)
        _ = await eventually { model.threads.count == 8 }
        let rows = model.threads.map(SenderColumn.plain)
        #expect(rows.contains("→ Sender 3"))
        #expect(rows.contains("Sender 1"))
    }
}
