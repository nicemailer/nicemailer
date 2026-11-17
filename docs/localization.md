# Localization

The Swift app follows the user's languages (System Settings, or the per-app language of NiceMailer). English and French are bundled; English is the fallback. The CLI and the core speak English only (`CLAUDE.md`, "Languages"). The work still to do for more languages is planned in `specifications/localisation.md`.

## Two locales

`Theme.swift` defines them once, at launch:

| Name | What it is | Used for |
| --- | --- | --- |
| `appLanguage`, `appLocale` | The language the app shows (`fr`), the best match between the user's languages and the bundled translations | The texts NiceMailer writes **into a mail** (the line introducing a quotation, the header of a forwarded mail), the language asked of the AI |
| `uiLocale` | The app's language for the words, the Mac's settings for the rest: region (order of day and month), 12- or 24-hour clock | Every date, time and number the **interface** shows, and the choice of plural forms |

So an English interface on a Mac set to France writes "28/09/21", "23:13" and "28 Sep 2021"; a French interface on a Mac set to the United States writes "28 sept. 2021" and "11:13 PM". The order of day and month comes from the language's data for that region (`en` in France: day first; `fr` anywhere: day first), as in Apple's own apps.

## Dates

No date format is spelled out. Every formatter uses a template (`setLocalizedDateFormatFromTemplate`): `jmm` for hours and minutes (`j` is the locale's own hour, 12 or 24), `dMMM` for day and month, `dMMMjmm` for both, so that the locale chooses the order and what joins them ("5 oct. à 23:13", "Oct 5 at 11:13 PM"). Only the file names and the parsing of the core's ISO dates use fixed patterns, in `en_US_POSIX`.

The formatters of the interface are built once in `Format.dates` (`Format.Dates`, `Models.swift`). The tests build their own for any locale and time zone, and pass a fixed `now`.

The date column of the geek lists is 8 characters wide. `Format.fitting` keeps the first template whose dates all fit, tried on the 28th of every month: a locale with longer names gets the numbers ("28.09." in German, whose "28. Sept." is 9 characters), and `yy-MM-dd` is the last resort.

## Strings

Every user-facing string goes through `L("…")`, whose key is the English text, looked up in `Resources/Localizable.xcstrings` (the String Catalog, compiled by SwiftPM into `Localizable.strings` and `Localizable.stringsdict` per language). A SwiftUI literal such as `Text("No results")` is **not** translated: SwiftUI looks it up in the main bundle, which has no translations. Pass `L("No results")`.

### One sentence, one key

Build a sentence with one key and its arguments, never by joining pieces: `L("%@ (and %lld others)", label, n)`, not `label + " " + L("(and %lld others)", n)`. The translation must be free to put the arguments where its grammar wants them. With several arguments, a translation numbers them (`%1$@`, `%2$lld`).

Segments of the status bar joined by " · " are independent phrases, each with its own key: that is allowed.

### Plurals

A key whose words depend on a number has plural variations in the catalog, in **every** language, English included: the key is only the form "other" of the English. `L()` formats with `uiLocale`, so `String(format:)` applies the rules of the app's language: in English 1 is singular and 0 plural; in French 0 and 1 are singular. Never write `n == 1 ? L("1 mail") : L("%lld mails", n)`: that is the English rule, wrong for French 0 and for most languages.

Every form keeps every argument of the key, the number included: French "one" also covers 0, so a form "its mail" without the number would be shown for 0 mails.

A key with two numbers that both change the words uses one substitution per number (`%1$#@saved@ … %3$#@missing@`), whose forms show their number as `%arg`. Its value must number every argument (`%2$@`), or the compiled table shows "(null)".

### Adding a string

1. Write `L("English text")` (or `L("%lld things", n)`).
2. Add the key to `Localizable.xcstrings` with its French. For a plural, give `en` and `fr` a `plural` variation with `one` and `other`.
3. `make swift-test`: the tests below fail on what is missing.

## Tests

`LocalizationTests` (Swift Testing):

- every form of every translation has the placeholders of its key, each argument with its type (a `%@` given for a `%lld` would crash `String(format:)`), substitutions numbered;
- every plural key varies in English and French;
- every key of the catalog is used by the sources, so wordings that change do not leave dead keys;
- no SwiftUI view shows a literal text outside the catalog;
- the plural forms in English and French, 0, 1 and more, one number or two;
- `interfaceLocale` keeps the region and the hour cycle of the Mac's settings;
- the dates of the lists and of the reader in several locales, and the 8-character column in ten of them.

`FormatTests` checks that every `L("…")` is in the catalog and that every key has a French translation in the compiled tables.

The UI scenarios run in English, with the region settings of a Mac set to France (`-AppleLocale fr_FR`), whatever the machine's. A scenario may set its language with `fixtures/<scenario>.language` and its region with `fixtures/<scenario>.locale` (`apps/NiceMailer/UITests/README.md`): `localization.uiscript` plays an English interface on a Mac set to France, `localization-fr.uiscript` a French interface on a Mac set to the United States.
