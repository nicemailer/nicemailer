import Foundation
import Testing
@testable import NiceMailer

/// The rules of `docs/localization.md`: plurals, placeholders, no text
/// outside the catalog, dates in the interface's locale.
///
/// Not on the main actor, unlike most suites: reading every source and
/// the catalog takes a while, and the suites that wait for the model on
/// the main actor (`ListPatchTests`) must not wait for it.
struct LocalizationTests {
    // MARK: Catalog

    /// The catalog, read from the repository (`Localizable.xcstrings`): the
    /// app only ships the tables compiled from it.
    static func catalog() throws -> [String: [String: Any]] {
        let url = sourcesRoot.appendingPathComponent("Resources/Localizable.xcstrings")
        let json = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any]
        return try #require(json?["strings"] as? [String: [String: Any]])
    }

    static let sourcesRoot = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/NiceMailer")

    /// Every Swift file of the app, as text.
    static func sources() throws -> [(name: String, text: String)] {
        let files = FileManager.default.enumerator(at: sourcesRoot, includingPropertiesForKeys: nil)?
            .compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" } ?? []
        return try files.map { ($0.lastPathComponent, try String(contentsOf: $0, encoding: .utf8)) }
    }

    /// The placeholders of a format, by argument position: `[1: "@", 2:
    /// "lld"]` for "%@ (%lld)" as for "%1$@ (%2$lld)". A plural
    /// substitution (`%1$#@mails@`) counts as the integer it stands for.
    static func placeholders(_ format: String) -> [Int: String] {
        let pattern = try! NSRegularExpression(pattern: #"%(?:(\d+)\$)?(#@\w+@|lld|ld|d|@|%)"#)
        var out: [Int: String] = [:]
        var next = 1
        for m in pattern.matches(in: format, range: NSRange(format.startIndex..., in: format)) {
            let type = String(format[Range(m.range(at: 2), in: format)!])
            if type == "%" { continue }
            let position = Range(m.range(at: 1), in: format).flatMap { Int(format[$0]) } ?? next
            out[position] = type.hasPrefix("#@") ? "lld" : type
            next = position + 1
        }
        return out
    }

    /// Every text a localization of the catalog can show, with its
    /// placeholders: its plain value, or each form of its plural
    /// variation; for a value with substitutions, its value and each form
    /// of each substitution (whose `%arg` is the number).
    static func forms(_ localization: [String: Any]) -> [(text: String, placeholders: [Int: String])] {
        var out: [(String, [Int: String])] = []
        func value(_ unit: Any?) -> String? {
            ((unit as? [String: Any])?["stringUnit"] as? [String: Any])?["value"] as? String
        }
        if let v = value(localization) {
            var p = placeholders(v)
            for (_, sub) in localization["substitutions"] as? [String: [String: Any]] ?? [:] {
                let n = sub["argNum"] as? Int ?? 0
                p[n] = sub["formatSpecifier"] as? String
                let forms = ((sub["variations"] as? [String: Any])?["plural"] as? [String: Any]) ?? [:]
                for (_, form) in forms {
                    let text = value(form) ?? ""
                    // A form of a substitution must show its number.
                    out.append((text, text.contains("%arg") ? [n: "lld"] : [:]))
                }
            }
            out.insert((v, p), at: 0)
        }
        let plural = ((localization["variations"] as? [String: Any])?["plural"] as? [String: Any]) ?? [:]
        for (_, form) in plural {
            if let v = value(form) { out.append((v, placeholders(v))) }
        }
        return out
    }

    /// Every form of every translation has the placeholders of its key,
    /// each argument with its type. A form that drops a number ("its
    /// mail") is refused: in French "one" is also 0. A `%@` given for a
    /// `%lld` would crash `String(format:)`.
    @Test func everyFormKeepsThePlaceholdersOfItsKey() throws {
        for (key, entry) in try Self.catalog() {
            let expected = Self.placeholders(key)
            let localizations = entry["localizations"] as? [String: [String: Any]] ?? [:]
            for (language, localization) in localizations {
                let forms = Self.forms(localization)
                #expect(!forms.isEmpty, "\(language) has no text for \(key)")
                for form in forms where localization["substitutions"] == nil {
                    #expect(form.placeholders == expected, "\(language) “\(form.text)” for “\(key)”")
                }
                if localization["substitutions"] != nil {
                    let main = try #require(forms.first)
                    #expect(main.placeholders == expected, "\(language) “\(main.text)” for “\(key)”")
                    for form in forms.dropFirst() {
                        #expect(!form.placeholders.isEmpty, "\(language) “\(form.text)” for “\(key)” has no %arg")
                    }
                    // Several arguments: positions written out, else the
                    // compiled table mixes them up ("(null)").
                    #expect(!main.text.contains(try Regex(#"%[@l]"#)), "\(language) “\(main.text)”: number each argument")
                }
            }
        }
    }

    /// A key with a plural variation has one in every language of the
    /// catalog, English included: the English text of the key is only the
    /// form "other".
    @Test func aPluralKeyVariesInEveryLanguage() throws {
        let catalog = try Self.catalog()
        let plural = catalog.filter { _, entry in
            (entry["localizations"] as? [String: [String: Any]] ?? [:]).values.contains {
                $0["variations"] != nil || $0["substitutions"] != nil
            }
        }
        #expect(plural.count >= 20)
        for (key, entry) in plural {
            let localizations = entry["localizations"] as? [String: [String: Any]] ?? [:]
            for language in ["en", "fr"] {
                #expect(localizations[language] != nil, "\(key) has no \(language)")
            }
        }
    }

    /// Every key of the catalog is used by a `L("…")` of the sources: a
    /// key left behind by a change of wording is removed with it.
    @Test func everyKeyOfTheCatalogIsUsed() throws {
        let pattern = try NSRegularExpression(pattern: #"\bL\((?:in: [^,]+, )?"((?:[^"\\]|\\.)*)""#)
        var used = Set<String>()
        for (_, text) in try Self.sources() {
            for m in pattern.matches(in: text, range: NSRange(text.startIndex..., in: text)) {
                if let r = Range(m.range(at: 1), in: text) {
                    used.insert(String(text[r]).replacingOccurrences(of: #"\""#, with: "\""))
                }
            }
        }
        for key in try Self.catalog().keys.sorted() {
            #expect(used.contains(key), "unused: \(key)")
        }
    }

    /// No text of the interface is written in a SwiftUI view without
    /// `L()`, the labels VoiceOver reads included: `Text("Aucun résultat")` looks its key up in the main
    /// bundle, which has no translation, and shows it as written. Only
    /// key names, symbols and words that are not translated (`text`,
    /// `html`) are allowed.
    @Test func noViewShowsATextOutsideTheCatalog() throws {
        let pattern = try NSRegularExpression(
            pattern: #"\b(?:Text|Label|Button|Toggle|TextField|Section|Menu)\("((?:[^"\\]|\\.)*)"|\.(?:alert|navigationTitle|help|confirmationDialog|accessibilityLabel|accessibilityHint|accessibilityValue)\("((?:[^"\\]|\\.)*)"|prompt: "((?:[^"\\]|\\.)*)""#)
        let allowed: Set<String> = [" text ", " html ", "T", "U", "F", "/", "–"]
        for (name, text) in try Self.sources() {
            for m in pattern.matches(in: text, range: NSRange(text.startIndex..., in: text)) {
                let literal = (1...3).compactMap { Range(m.range(at: $0), in: text).map { String(text[$0]) } }.first ?? ""
                // What the code inserts (`\(n)`) is not text.
                let words = literal.replacing(try Regex(#"\\\([^)]*\)+"#), with: "")
                guard words.contains(where: \.isLetter), !allowed.contains(literal) else { continue }
                Issue.record("\(name): “\(literal)” is not in the catalog")
            }
        }
    }

    // MARK: Errors of the core

    /// The reasons of the contract and the keys of their values, read from
    /// the table of `docs/contract.md`, section "Errors", which the Rust
    /// tests keep equal to `nm_api::reason` (`contract_doc.rs`).
    static func contractReasons() throws -> [(name: String, values: [String])] {
        let page = sourcesRoot.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("docs/contract.md")
        let text = try String(contentsOf: page, encoding: .utf8)
        let errors = try #require(text.components(separatedBy: "\n## Errors\n").last)
        return errors.split(separator: "\n").filter { $0.hasPrefix("| `") }.map { line in
            let cells = line.split(separator: "|").map { $0.trimmingCharacters(in: .whitespaces) }
            let name = cells[0].trimmingCharacters(in: CharacterSet(charactersIn: "`"))
            let values = cells[1] == "—" ? [] : cells[1].components(separatedBy: ", ").map { $0.trimmingCharacters(in: CharacterSet(charactersIn: "`")) }
            return (name, values)
        }
    }

    /// Every reason of the contract is written by the app, or listed as
    /// generic; never both, never neither.
    @Test func everyReasonOfTheCoreIsWrittenOrSaidGeneric() throws {
        let reasons = try Self.contractReasons()
        #expect(reasons.count > 90)
        for r in reasons {
            let values = Dictionary(uniqueKeysWithValues: r.values.map { ($0, "x") })
            let written = CoreReason.sentence(r.name, values) != nil
            let generic = CoreReason.generic.contains(r.name)
            #expect(written != generic, "\(r.name): \(written ? "written and generic" : "neither written nor generic")")
        }
        // And nothing for a reason the contract does not have.
        let names = Set(reasons.map(\.name))
        #expect(CoreReason.generic.subtracting(names).isEmpty, "generic reasons not in the contract: \(CoreReason.generic.subtracting(names))")
    }

    /// The values a sentence of `CoreReason` reads are values of its reason,
    /// and it names only reasons of the contract: a key mistyped would give
    /// an empty word.
    @Test func theSentencesOfTheCoreReadTheValuesOfTheirReason() throws {
        let reasons = Dictionary(uniqueKeysWithValues: try Self.contractReasons().map { ($0.name, Set($0.values)) })
        let source = try String(contentsOf: Self.sourcesRoot.appendingPathComponent("CoreReason.swift"), encoding: .utf8)
        let caseLine = try NSRegularExpression(pattern: #"case "([a-z0-9_]+)": L\((.*)\)$"#, options: .anchorsMatchLines)
        let value = try NSRegularExpression(pattern: #"v\("([a-z_]+)"\)"#)
        var cases = 0
        for m in caseLine.matches(in: source, range: NSRange(source.startIndex..., in: source)) {
            cases += 1
            let name = String(source[Range(m.range(at: 1), in: source)!])
            let call = String(source[Range(m.range(at: 2), in: source)!])
            let allowed = try #require(reasons[name], "\(name) is not a reason of the contract")
            for v in value.matches(in: call, range: NSRange(call.startIndex..., in: call)) {
                let key = String(call[Range(v.range(at: 1), in: call)!])
                #expect(allowed.contains(key), "\(name) reads \(key), which it does not have")
            }
        }
        #expect(cases > 80)
    }

    /// The app's sentence, never the core's English message; a value of a
    /// third party cited; an unknown reason, the generic sentence with the
    /// message.
    @Test func anErrorOfTheCoreIsSaidFromItsReason() {
        let tag = CoreClient.RPCError(code: -32602, message: "the tag Urgent exists already", data: ["reason": "tag_exists", "tag": "Urgent"])
        #expect(tag.localizedDescription == L("The tag %@ exists already", "Urgent"))
        #expect(L(in: "fr", "The tag %@ exists already", "Urgent") == "Le tag Urgent existe déjà")
        let refused = CoreClient.RPCError(code: 1009, message: "x", data: ["reason": "imap_auth_refused", "server_text": "[AUTHENTICATIONFAILED] Mot de passe incorrect"])
        #expect(refused.localizedDescription == L("The server refused the login: %@", "[AUTHENTICATIONFAILED] Mot de passe incorrect"))
        let failure = CoreClient.RPCError(code: -32603, message: "internal error: disk full", data: ["reason": "internal", "detail": "disk full"])
        #expect(failure.localizedDescription == L("NiceMailer could not do it: %@", "internal error: disk full"))
        let bare = CoreClient.RPCError(code: 1, message: "something")
        #expect(bare.localizedDescription == L("NiceMailer could not do it: %@", "something"))
    }

    /// The values of `data` reach the sentence whatever their JSON type.
    @Test func theValuesOfAnErrorAreReadAsText() {
        let e = CoreClient.rpcError(["code": -32602, "message": "m",
                                     "data": ["reason": "file_too_large", "file": "a.pdf", "size_mb": 72, "list": ["a", "b"]]])
        #expect(e.reason == "file_too_large")
        #expect(e.data["size_mb"] == "72" && e.data["list"] == "a, b")
        #expect(e.localizedDescription == L("%@ is too large to read (%@ MB, 50 MB at most)", "a.pdf", "72"))
    }

    /// The `why` of an event or a result: its reason's sentence, else the
    /// core's English message. A result decoded with the app's decoder
    /// gives back the keys of the contract (`server_text`, not
    /// `serverText`).
    @Test func theWhyOfAnEventOrAResultIsSaidFromItsReason() throws {
        #expect(CoreReason.text(why: ["reason": "smtp_refused", "server_text": "550 no"], message: "x")
            == L("The mail server refused the mail: %@", "550 no"))
        #expect(CoreReason.text(why: nil, message: "english") == "english")
        #expect(CoreReason.text(why: ["reason": "internal"], message: "english") == "english")
        let json = #"{"id": 1, "key": "perso", "kind": "imap", "enabled": true, "position": 0, "error": "x", "#
            + #""why": {"reason": "password_server_changed", "account": "perso", "saved_user": "a", "saved_host": "h1", "user": "b", "host": "h2"}}"#
        let info = try CoreClient.decoder.decode(AccountInfo.self, from: Data(json.utf8))
        #expect(info.why?.values["saved_host"] == "h1")
        #expect(info.shownError == L("The password of %@ was saved for another server (%@ on %@): give it again to confirm %@ on %@",
                                     "perso", "a", "h1", "b", "h2"))
        #expect(CoreWhy.snake("serverText") == "server_text" && CoreWhy.snake("reason") == "reason")
    }

    // MARK: Plurals

    @Test func aNumberChoosesItsFormByTheRulesOfTheLanguage() {
        // English: 1 is singular, 0 plural.
        #expect(L(in: "en", "%lld mails", 0) == "0 mails")
        #expect(L(in: "en", "%lld mails", 1) == "1 mail")
        #expect(L(in: "en", "%lld mails", 2) == "2 mails")
        // French: 0 and 1 are singular.
        #expect(L(in: "fr", "%lld mails", 0) == "0 mail")
        #expect(L(in: "fr", "%lld mails", 1) == "1 mail")
        #expect(L(in: "fr", "%lld mails", 2) == "2 mails")
        #expect(L(in: "fr", "%lld mails filed", 1) == "1 mail rangé")
        #expect(L(in: "fr", "%lld mails filed", 3) == "3 mails rangés")
        // The number need not come first; the other values keep their place.
        #expect(L(in: "en", "Delete the tag %@ (taken off its %lld mails)?", "devis", 1) == "Delete the tag devis (taken off 1 mail)?")
        #expect(L(in: "fr", "Delete the tag %@ (taken off its %lld mails)?", "devis", 0) == "Supprimer le tag devis (retiré de 0 mail) ?")
        #expect(L(in: "fr", "Delete the tag %@ (taken off its %lld mails)?", "devis", 4) == "Supprimer le tag devis (retiré de ses 4 mails) ?")
        #expect(L(in: "en", "%@ (and %lld others)", "Lire", 1) == "Lire (and 1 other)")
        #expect(L(in: "fr", "%@ (and %lld others)", "Lire", 2) == "Lire (et 2 autres)")
        #expect(L(in: "en", "%lld days", 1) == "1 day" && L(in: "fr", "%lld years", 1) == "1 an")
    }

    @Test func twoNumbersVaryEachOnItsOwn() {
        let key = "%lld attachments saved to %@, %lld not available"
        #expect(L(in: "en", key, 1, "Downloads", 2) == "1 attachment saved to Downloads, 2 not available")
        #expect(L(in: "en", key, 3, "Downloads", 1) == "3 attachments saved to Downloads, 1 not available")
        #expect(L(in: "fr", key, 1, "Téléchargements", 2) == "1 pièce jointe enregistrée dans Téléchargements, 2 non disponibles")
        #expect(L(in: "fr", key, 2, "Téléchargements", 1) == "2 pièces jointes enregistrées dans Téléchargements, 1 non disponible")
        let sort = "%@: %lld of %lld sorted · esc stop"
        #expect(L(in: "fr", sort, "Tri", 0, 1) == "Tri : 0 sur 1 trié · esc arrêter")
        #expect(L(in: "fr", sort, "Tri", 1, 5) == "Tri : 1 sur 5 triés · esc arrêter")
        #expect(L(in: "en", sort, "Sort", 1, 5) == "Sort: 1 of 5 sorted · esc stop")
    }

    /// `L` itself, in the app's language, picks the plural form too.
    @Test func theAppsOwnLookupAppliesThePlurals() {
        #expect(L("%lld mails", 1) == L(in: appLanguage, "%lld mails", 1))
        #expect(L("%lld mails", 1) != L("%lld mails", 2))
    }

    // MARK: Dates

    static let paris = TimeZone(identifier: "Europe/Paris")!
    static var calendar: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = paris
        return c
    }
    /// A moment in Paris, as the core gives it (seconds since 1970).
    static func at(_ year: Int, _ month: Int, _ day: Int, _ hour: Int = 23, _ minute: Int = 13) -> Int {
        Int(calendar.date(from: DateComponents(year: year, month: month, day: day, hour: hour, minute: minute))!.timeIntervalSince1970)
    }
    /// Tuesday 14 November 2023, 23:13 in Paris.
    static let now = Date(timeIntervalSince1970: TimeInterval(at(2023, 11, 14)))

    static func dates(_ language: String, _ settings: String) -> Format.Dates {
        Format.Dates(locale: interfaceLocale(language: language, settings: Locale(identifier: settings)), timeZone: paris)
    }

    /// The words follow the app's language, the order and the clock the
    /// Mac's settings.
    @Test func theInterfaceLocaleTakesTheWordsFromTheAppAndTheRestFromTheMac() {
        let english = interfaceLocale(language: "en", settings: Locale(identifier: "fr_FR"))
        #expect(english.language.languageCode?.identifier == "en")
        #expect(english.region?.identifier == "FR")
        #expect(english.hourCycle == .zeroToTwentyThree)
        // A 24-hour clock chosen on an American Mac is kept.
        let forced = interfaceLocale(language: "fr", settings: Locale(identifier: "en_US@hours=h23"))
        #expect(forced.hourCycle == .zeroToTwentyThree)
        #expect(interfaceLocale(language: "fr", settings: Locale(identifier: "en_US")).hourCycle == .oneToTwelve)
    }

    @Test func listDatesFollowTheInterfaceLocale() {
        func list(_ d: Format.Dates, _ t: Int) -> String { Format.listDate(t, in: d, now: Self.now, calendar: Self.calendar) }
        let today = Self.at(2023, 11, 14), october = Self.at(2023, 10, 5), older = Self.at(2020, 11, 15)
        // English words, French order and clock.
        let enFR = Self.dates("en", "fr_FR")
        var s = list(enFR, today); #expect(s == "23:13")
        s = list(enFR, october); #expect(s == "5 Oct")
        s = list(enFR, older); #expect(s == "15/11/2020")
        // An American Mac.
        let enUS = Self.dates("en", "en_US")
        s = list(enUS, today); #expect(s == "11:13\u{202F}PM")
        s = list(enUS, october); #expect(s == "Oct 5")
        s = list(enUS, older); #expect(s == "11/15/20")
        // French words: what the classic theme wrote whatever the language.
        let frFR = Self.dates("fr", "fr_FR")
        s = list(frFR, october); #expect(s == "5 oct.")
        s = Format.longDate(today, in: frFR); #expect(s == "mardi 14 novembre 2023 à 23:13")
        s = Format.longDate(today, in: enFR); #expect(s == "Tuesday 14 November 2023 at 23:13")
    }

    @Test func geekListDatesFitTheirEightCharacterColumnInEveryLocale() {
        func list(_ d: Format.Dates, _ t: Int) -> String { Format.geekListDate(t, in: d, now: Self.now, calendar: Self.calendar) }
        for (language, settings) in [("fr", "fr_FR"), ("en", "en_US"), ("en", "en_GB"), ("de", "de_DE"), ("pt", "pt_BR"),
                                     ("ru", "ru_RU"), ("ja", "ja_JP"), ("ko", "ko_KR"), ("es", "es_ES"), ("pl", "pl_PL"),
                                     // The Islamic calendar and Arabic digits.
                                     ("ar", "ar_SA")] {
            let d = Self.dates(language, settings)
            // The 28th of every month, this year and an earlier one.
            for month in 1...11 {
                for t in [Self.at(2023, month, 28), Self.at(2021, month, 28), Self.at(2023, 11, 14, 9, 5)] {
                    let s = list(d, t)
                    #expect(s.count <= 8, "\(language)_\(settings): \(s)")
                }
            }
        }
        let fr = Self.dates("fr", "fr_FR"), us = Self.dates("en", "en_US")
        var s = list(fr, Self.at(2023, 11, 14)); #expect(s == "23:13")
        s = list(fr, Self.at(2020, 11, 5)); #expect(s == "05/11/20")
        s = list(us, Self.at(2020, 11, 5)); #expect(s == "11/05/20")
        // German writes "28. Sept.", one character too many: the column
        // gets the numbers instead.
        s = list(Self.dates("de", "de_DE"), Self.at(2023, 9, 28)); #expect(s == "28.09.")
        // The order of the region, whatever the language: a French
        // interface on an American Mac, as a typed date is read.
        s = list(Self.dates("fr", "en_US"), Self.at(2020, 11, 5)); #expect(s == "11/05/20")
        #expect(Format.twoDigits("M/d/yy") == "MM/dd/yy" && Format.twoDigits("dd.MM.y") == "dd.MM.yy" && Format.twoDigits("y/MM/dd") == "yy/MM/dd")
    }

    @Test func geekMessageDatesLetTheLocaleJoinTheDayAndTheTime() {
        let fr = Self.dates("fr", "fr_FR"), us = Self.dates("en", "en_US")
        func header(_ d: Format.Dates, _ t: Int) -> String { Format.geekMessageDate(t, in: d, now: Self.now, calendar: Self.calendar) }
        var s = header(fr, Self.at(2023, 11, 14, 8, 2)); #expect(s == "08:02")
        s = header(fr, Self.at(2023, 11, 13)); #expect(s == L("yesterday %@", "23:13"))
        s = header(fr, Self.at(2023, 10, 5)); #expect(s == "5 oct. à 23:13")
        s = header(us, Self.at(2023, 10, 5)); #expect(s == "Oct 5 at 11:13\u{202F}PM")
        s = header(fr, Self.at(2020, 11, 5)); #expect(s == "5 nov. 2020")
    }
}
