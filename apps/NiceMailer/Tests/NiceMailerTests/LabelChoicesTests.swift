import Testing
@testable import NiceMailer

/// The rows of the `t` and `g` pickers (`LabelChoices`, `ui.md`, "Ajouter
/// et retirer un tag ou un groupe").
struct LabelChoicesTests {
    typealias Row = LabelChoices.Row

    @Test func aSingleMailOffersToRemoveWhatItHasAndToAddTheRest() {
        let rows = LabelChoices.rows(names: ["VIP", "Famille", "Club"], held: [["Club", "VIP"]])
        // Removals first, in display order; then the additions.
        #expect(rows == [
            Row(name: "VIP", remove: true, held: 1), Row(name: "Club", remove: true, held: 1),
            Row(name: "Famille", remove: false, held: 0),
        ])
    }

    @Test func aMixedSelectionOffersBothActionsForANameSomeMailsHave() {
        // Three mails: VIP on two, Famille on all, Club on none.
        let held = [["VIP", "Famille"], ["Famille"], ["VIP", "Famille"]]
        let rows = LabelChoices.rows(names: ["VIP", "Famille", "Club"], held: held)
        #expect(rows == [
            Row(name: "VIP", remove: true, held: 2), Row(name: "Famille", remove: true, held: 3),
            // Famille is on every mail: nothing to add.
            Row(name: "VIP", remove: false, held: 2), Row(name: "Club", remove: false, held: 0),
        ])
    }

    @Test func namesCompareWithoutCaseAndAreListedOnce() {
        // "vip" on a mail is the group "VIP"; a name listed twice (two
        // accounts' tags of the same name) is one row.
        let rows = LabelChoices.rows(names: ["VIP", "vip", "Club"], held: [["vip", "VIP"]])
        #expect(rows == [Row(name: "VIP", remove: true, held: 1), Row(name: "Club", remove: false, held: 0)])
    }

    @Test func aNameOnlyTheMailsKnowCanBeRemovedNotAdded() {
        // A tag of another account, absent from the names offered.
        let rows = LabelChoices.rows(names: ["VIP"], held: [["Zeta"], ["alpha"]])
        #expect(rows == [
            Row(name: "alpha", remove: true, held: 1), Row(name: "Zeta", remove: true, held: 1),
            Row(name: "VIP", remove: false, held: 0),
        ])
    }

    @Test func withoutMailEveryNameIsOfferedToAdd() {
        #expect(LabelChoices.rows(names: ["VIP"], held: []) == [Row(name: "VIP", remove: false, held: 0)])
    }
}
