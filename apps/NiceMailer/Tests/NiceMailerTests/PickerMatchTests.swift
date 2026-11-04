import Testing
@testable import NiceMailer

/// Matching and ranking of picker rows (`PickerMatch`). Literal French and
/// English labels: the function is pure, it does not depend on the
/// machine's language.
struct PickerMatchTests {
    /// The French rows of the attribute selector, in their list order.
    let french = ["non lus", "lus", "drapeau", "sans drapeau", "pièce jointe", "sans pièce jointe",
                  "répondus", "sans ma réponse", "conversations", "mails seuls",
                  "De…", "À…", "Texte…", "Sujet…", "Date…"]
    let keywords: [String: [String]] = ["De…": ["expéditeur", "envoyé"], "À…": ["destinataire", "cc", "copie"]]

    func first(_ query: String, in labels: [String]) -> String? {
        PickerMatch.filter(labels, query: query, label: { $0 }, keywords: { keywords[$0] ?? [] }).first
    }

    @Test func theRowStartingWithTheTypedTextComesFirst() {
        // "non lus" is listed first and contains "lus": the former filter
        // chose it, the opposite of what was typed.
        #expect(first("lu", in: french) == "lus")
        #expect(first("read", in: ["unread", "read"]) == "read")
        #expect(first("de", in: french) == "De…")
        #expect(first("dr", in: french) == "drapeau")
        #expect(first("da", in: french) == "Date…")
        #expect(first("r", in: french) == "répondus")
    }

    @Test func accentsAndCaseAreIgnored() {
        #expect(first("a", in: french) == "À…")
        #expect(first("REP", in: french) == "répondus")
        #expect(first("piece", in: french) == "pièce jointe")
    }

    @Test func initialsReachNegativeRows() {
        #expect(first("nl", in: french) == "non lus")
        #expect(first("sd", in: french) == "sans drapeau")
        #expect(first("spj", in: french) == "sans pièce jointe")
        #expect(first("pj", in: french) == "pièce jointe")
        #expect(first("sm", in: french) == "sans ma réponse")
        #expect(first("nf", in: ["flagged", "not flagged"]) == "not flagged")
        // One letter is not taken as initials, nor are several words.
        #expect(PickerMatch.rank("s", label: "sans drapeau") == .labelPrefix)
        #expect(PickerMatch.rank("s d", label: "sans drapeau") == .wordPrefix)
        #expect(PickerMatch.rank("sx", label: "sans drapeau") == nil)
    }

    @Test func hiddenWordsFindARow() {
        #expect(first("cc", in: french) == "À…")
        #expect(first("dest", in: french) == "À…")
        #expect(first("exp", in: french) == "De…")
    }

    @Test func wordsInAnyOrderAndInsideWordsStillMatch() {
        #expect(PickerMatch.rank("jointe sans", label: "sans pièce jointe") == .wordPrefix)
        #expect(PickerMatch.rank("lagg", label: "flagged") == .substring)
        #expect(PickerMatch.rank("zz", label: "flagged") == nil)
        // A row matching no word is hidden.
        #expect(!PickerMatch.filter(french, query: "xyz", label: { $0 }).contains("lus"))
    }

    /// The "Typed to reach it" column of `docs/geek-ui.md` ("Attributes").
    @Test func theKeysOfTheDocumentationReachTheirRows() {
        let english = ["unread", "read", "flagged", "not flagged", "with attachments", "without attachments",
                       "answered", "not answered", "conversation", "single mail",
                       "From…", "To…", "Text…", "Subject…", "Date…"]
        let englishWords: [String: [String]] = ["From…": ["sender"], "To…": ["recipient", "cc"],
                                                "Text…": ["text", "words", "body"], "Subject…": ["title"],
                                                "Date…": ["when", "day", "period"]]
        let en: [String: String] = ["un": "unread", "r": "read", "f": "flagged", "nf": "not flagged",
                                    "wi": "with attachments", "witho": "without attachments",
                                    "an": "answered", "na": "not answered", "co": "conversation", "si": "single mail",
                                    "fr": "From…", "sender": "From…", "to": "To…", "cc": "To…",
                                    "te": "Text…", "su": "Subject…", "title": "Subject…", "da": "Date…"]
        for (typed, row) in en {
            let got = PickerMatch.filter(english, query: typed, label: { $0 }, keywords: { englishWords[$0] ?? [] }).first
            #expect(got == row, "\(typed)")
        }
        let frenchWords = keywords.merging(["Texte…": ["texte", "mots", "corps"], "Sujet…": ["titre", "objet"],
                                            "Date…": ["quand", "jour", "période"]]) { a, _ in a }
        let fr: [String: String] = ["no": "non lus", "nl": "non lus", "l": "lus", "dr": "drapeau", "sd": "sans drapeau",
                                    "pj": "pièce jointe", "pi": "pièce jointe", "spj": "sans pièce jointe",
                                    "r": "répondus", "sm": "sans ma réponse", "co": "conversations", "m": "mails seuls",
                                    "de": "De…", "exp": "De…", "a": "À…", "cc": "À…", "dest": "À…",
                                    "te": "Texte…", "su": "Sujet…", "objet": "Sujet…", "da": "Date…", "quand": "Date…"]
        for (typed, row) in fr {
            let got = PickerMatch.filter(french, query: typed, label: { $0 }, keywords: { frenchWords[$0] ?? [] }).first
            #expect(got == row, "\(typed)")
        }
    }

    @Test func emptyQueryKeepsTheListOrder() {
        #expect(PickerMatch.filter(french, query: "  ", label: { $0 }) == french)
    }

    @Test func rowsOfTheSameRankKeepTheirOrderAndPinnedRowsStayFirst() {
        let rows = ["b alpha", "a alpha", "+ new"]
        #expect(PickerMatch.filter(rows, query: "alpha", label: { $0 }, pinned: { $0.hasPrefix("+") })
            == ["+ new", "b alpha", "a alpha"])
        // A pinned row stays even when it does not match.
        #expect(PickerMatch.filter(rows, query: "zzz", label: { $0 }, pinned: { $0.hasPrefix("+") }) == ["+ new"])
    }
}
