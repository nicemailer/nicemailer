# Localization

The Swift app follows the user's languages (System Settings, or the per-app language of NiceMailer). English and French are bundled; English is the fallback. The CLI and the core speak English only (`CLAUDE.md`, "Languages"). The work still to do for more languages is planned in `specifications/localisation.md`.

## Two locales

`Theme.swift` defines them once, at launch:

| Name | What it is | Used for |
| --- | --- | --- |
| `appLanguage`, `appLocale` | The language the app shows (`fr`), the best match between the user's languages and the bundled translations | The bundle of translations, and the language asked of the AI for its answers |
| `uiLocale` | The app's language for the words, the Mac's settings for the rest: region (order of day and month), 12- or 24-hour clock | Every date, time and number the app writes: in the **interface**, and **in a mail** (the line introducing a quotation, the header of a forwarded mail); the choice of plural forms |

The words NiceMailer writes into a mail follow the app's language, not the language of the mail answered (`specifications/localisation.md`, section 3.2): simple and predictable, at the cost of a French line above an English reply written from a French interface.

So an English interface on a Mac set to France writes "28/09/21", "23:13" and "28 Sep 2021"; a French interface on a Mac set to the United States writes "28 sept. 2021" and "11:13 PM". The order of day and month comes from the language's data for that region (`en` in France: day first; `fr` anywhere: day first), as in Apple's own apps.

## Dates

No date format is spelled out. Every formatter uses a template (`setLocalizedDateFormatFromTemplate`): `jmm` for hours and minutes (`j` is the locale's own hour, 12 or 24), `dMMM` for day and month, `dMMMjmm` for both, so that the locale chooses the order and what joins them ("5 oct. à 23:13", "Oct 5 at 11:13 PM"). Only the file names and the parsing of the core's ISO dates use fixed patterns, in `en_US_POSIX`.

The formatters of the interface are built once in `Format.dates` (`Format.Dates`, `Models.swift`). The tests build their own for any locale and time zone, and pass a fixed `now`.

A date the user types (the attribute selector, `DateChoice.Order`) is read in the order of the region's short date, whatever the language; the date column of the geek lists shows the same short date with two digits per field (`Format.twoDigits`), so that what is typed is read as what is shown. ICU's templates with padded fields (`ddMMyy`) would follow the language instead: a French interface on an American Mac would show day first.

The date column of the geek lists is 8 characters wide. `Format.fitting` keeps the first pattern whose dates all fit, tried on the 28th of every month: a locale with longer names gets the numbers ("28.09." in German, whose "28. Sept." is 9 characters), and `yy-MM-dd` is the last resort (Korean, whose short date "yy. MM. dd." is 11 characters).

## Numbers

`L()` formats with `uiLocale`, so a `%lld` is written with the region's grouping ("12 345", "12,345"): pass a count as `%lld`, never as a string formatted beforehand, which would lose its plural forms. A size goes through `Format.bytes` ("251 ko", "251 kB"): `ByteCountFormatter` takes no locale.

## Strings

Every user-facing string goes through `L("…")`, whose key is the English text, looked up in `Resources/Localizable.xcstrings` (the String Catalog, compiled by SwiftPM into `Localizable.strings` and `Localizable.stringsdict` per language). A SwiftUI literal such as `Text("No results")` is **not** translated: SwiftUI looks it up in the main bundle, which has no translations. Pass `L("No results")`.

### One sentence, one key

Build a sentence with one key and its arguments, never by joining pieces: `L("%@ (and %lld others)", label, n)`, not `label + " " + L("(and %lld others)", n)`. The translation must be free to put the arguments where its grammar wants them. With several arguments, a translation numbers them (`%1$@`, `%2$lld`).

Segments of the status bar joined by " · " are independent phrases, each with its own key: that is allowed.

### Plurals

A key whose words depend on a number has plural variations in the catalog, in **every** language, English included: the key is only the form "other" of the English. `L()` formats with `uiLocale`, so `String(format:)` applies the rules of the app's language: in English 1 is singular and 0 plural; in French 0 and 1 are singular. Never write `n == 1 ? L("1 mail") : L("%lld mails", n)`: that is the English rule, wrong for French 0 and for most languages.

Every form keeps every argument of the key, the number included: French "one" also covers 0, so a form "its mail" without the number would be shown for 0 mails.

A key with two numbers that both change the words uses one substitution per number (`%1$#@saved@ … %3$#@missing@`), whose forms show their number as `%arg`. Its value must number every argument (`%2$@`), or the compiled table shows "(null)".

### Errors of the core

The core does not know the user's language. A failed request gives a reason and its values in `data` (`nm_api::reason`, the table in `docs/contract.md`, "Errors"); its `message` is the English sentence, for the CLI, the logs and the AI agents. The app writes the sentence in `CoreReason.swift`, the one place that turns a reason into words: `RPCError.localizedDescription` is that sentence, so every `error.localizedDescription` of the app is in the user's language. Never show `RPCError.message`.

Events and results that say why something failed (`AccountError`, `ActionReverted`, `SendFailed`, `AiFailed`, the accounts of `list_accounts`) carry the same reason and values as `why`: `CoreReason.text(why:message:)` writes it, falling back on the English message. The app's decoder turns snake_case keys into camelCase, those of a dictionary included; `CoreWhy` turns them back.

A value whose key ends with `_text` (`server_text`, `system_text`, `provider_text`) was written by someone else: the sentence cites it as it is. The generic reasons (mistakes of a program, refusals made to AI agents) are listed in `CoreReason.generic` and said by one sentence with the English message.

A new reason: add it to `reasons!` in `nm-api` (name and English sentence), give it where it applies, add its row to `docs/contract.md`, and its sentence to `CoreReason` with its French in the catalog (or add it to `generic`). The tests below fail on each of these steps left out.

### Adding a string

1. Write `L("English text")` (or `L("%lld things", n)`).
2. Add the key to `Localizable.xcstrings` with its French. For a plural, give `en` and `fr` a `plural` variation with `one` and `other`.
3. `make swift-test`: the tests below fail on what is missing.

## The app bundle

`make bundle` lists in `CFBundleLocalizations` the languages of the String Catalog (`scripts/bundle-languages.py`): macOS offers the app in a language only when the bundle declares it. The texts of `Info.plist` that macOS shows itself (`NSContactsUsageDescription`, why the app asks for Contacts) are in English in `Info.plist` and, for each other language, in `apps/NiceMailer/Bundle/<language>.lproj/InfoPlist.strings`; `scripts/tests/test_bundle_languages.py` fails when a language lacks one.

The lists of addresses or of names are joined with ", ", right for the languages written in Latin or Cyrillic letters; Chinese and Japanese (`、`) and Arabic (`، `) would need their own, with right-to-left writing (`specifications/localisation.md`, section 3.8).

## Tests

`LocalizationTests` (Swift Testing):

- every form of every translation has the placeholders of its key, each argument with its type (a `%@` given for a `%lld` would crash `String(format:)`), substitutions numbered;
- every plural key varies in English and French;
- every key of the catalog is used by the sources, so wordings that change do not leave dead keys;
- no SwiftUI view shows a literal text outside the catalog;
- the plural forms in English and French, 0, 1 and more, one number or two;
- `interfaceLocale` keeps the region and the hour cycle of the Mac's settings;
- the dates of the lists and of the reader in several locales, and the 8-character column in ten of them;
- every reason of the contract written by `CoreReason` or listed as generic, never both; each sentence reads only values of its reason; an error said from its reason, a third party's text cited, an unknown reason said generically.

In Rust, `contract_doc.rs` keeps the table of reasons of `docs/contract.md` equal to `nm_api::reason` with their values, and checks that each reason is given by the code; `nm-api` checks that every English sentence is completed by its values.

`FormatTests` checks that every `L("…")` is in the catalog and that every key has a French translation in the compiled tables.

The UI scenarios run in English, with the region settings of a Mac set to France (`-AppleLocale fr_FR`), whatever the machine's. A scenario may set its language with `fixtures/<scenario>.language` and its region with `fixtures/<scenario>.locale` (`apps/NiceMailer/UITests/README.md`): `localization.uiscript` plays an English interface on a Mac set to France, `localization-fr.uiscript` a French interface on a Mac set to the United States.
