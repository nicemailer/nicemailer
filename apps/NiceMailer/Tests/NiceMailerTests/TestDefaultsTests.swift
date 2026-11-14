import Foundation
import Testing

/// The tests' own preferences (`TestDefaults`): empty, apart from each
/// other, and kept out of `~/Library/Preferences`.
struct TestDefaultsTests {
    @Test func eachSuiteStartsEmptyAndApart() {
        let a = TestDefaults.make()
        let b = TestDefaults.make()
        #expect(a.object(forKey: "geek.columns") == nil)
        a.set(2, forKey: "geek.columns")
        #expect(a.integer(forKey: "geek.columns") == 2)
        #expect(b.object(forKey: "geek.columns") == nil)
    }

    @Test func aSuiteIsAFileOfTheRunsFolderNotOfThePreferences() {
        let path = TestDefaults.newPath()
        let d = UserDefaults(suiteName: path)!
        d.set(true, forKey: "probe")
        d.synchronize()
        let fm = FileManager.default
        #expect(fm.fileExists(atPath: path + ".plist"))
        #expect(path.hasPrefix(fm.temporaryDirectory.path))
        let name = (path as NSString).lastPathComponent
        let preferences = NSHomeDirectory() + "/Library/Preferences"
        let leaked = (try? fm.contentsOfDirectory(atPath: preferences))?.filter { $0.contains(name) } ?? []
        #expect(leaked.isEmpty)
    }
}
