# Contributing to NiceMailer

NiceMailer is made by one person, in the open. Help is welcome, in this order of usefulness.

## Report a bug

Open an [issue](https://github.com/nicemailer/nicemailer/issues/new/choose) with the bug form: it asks for the version, the provider and the state of the synchronization, which is what is needed to find the cause. Remove email addresses and anything private from what you paste.

## Ask a question, share feedback

Use the [Discussions](https://github.com/nicemailer/nicemailer/discussions). Telling what you like, what bothers you and why you stopped using NiceMailer helps as much as a bug report.

## Review a translation

The app is in 19 languages. Only English and French have been checked by a native speaker. If you see a wrong or clumsy wording in your language, open an issue with the sentence, where it appears and your suggestion. The strings are in `apps/NiceMailer/Sources/NiceMailer/Resources/Localizable.xcstrings` ([docs/localization.md](docs/localization.md)).

## Propose a change to the code

Please open an issue before writing code for anything beyond a small fix, so that we agree on the change before you spend time on it.

A pull request follows the rules of the project, written in [CLAUDE.md](CLAUDE.md):

- the code is tested: Rust tests, and for the app a unit test plus a UI scenario (`apps/NiceMailer/UITests`); `make test` and `make ui-test` pass;
- the documentation in `docs/` is updated in the same change, and a change users can notice adds a line under `## Unreleased` in [CHANGELOG.md](CHANGELOG.md);
- comments explain why, not only what;
- commit messages are in English, in the form `type: area: What it does`, as in `git log`.

[Build from source](README.md#build-from-source) explains how to build and run it.

By contributing, you agree that your contribution is published under the [AGPLv3](LICENSE), the licence of the project.
