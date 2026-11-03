import Foundation
import Testing
@testable import NiceMailer

/// The text size of the geek theme (`TextSize`) and what derives from it.
@MainActor
struct TextSizeTests {
    func fresh() -> (TextSize, UserDefaults) {
        let d = UserDefaults(suiteName: "NiceMailerTests-\(UUID().uuidString)")!
        return (TextSize(defaults: d), d)
    }

    @Test func startsAtTheDefaultSize() {
        let (t, _) = fresh()
        #expect(t.size == 12 && t.scale == 1)
    }

    @Test func changesByOnePointWithinTheRange() {
        let (t, _) = fresh()
        for _ in 0..<30 { t.bigger() }
        #expect(t.size == 24)
        for _ in 0..<30 { t.smaller() }
        #expect(t.size == 9)
        t.reset()
        #expect(t.size == 12)
    }

    @Test func isRememberedAndClampedWhenRead() {
        let (t, d) = fresh()
        t.bigger(); t.bigger()
        #expect(TextSize(defaults: d).size == 14)
        d.set(99.0, forKey: TextSize.key)
        #expect(TextSize(defaults: d).size == 24)
    }

    @Test func columnWidthsFollowTheScale() {
        let normal = RowLayout(width: 1000, scale: 1)
        let big = RowLayout(width: 1000, scale: 1.5)
        #expect(big.char > normal.char)
        #expect(abs(big.date - 8 * big.char) < 0.001)
    }
}
