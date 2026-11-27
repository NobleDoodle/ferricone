# Translating Ferricone

Each language is one file, `po/<lang>/LC_MESSAGES/ferricone.po`, in the
gettext format that Poedit, Lokalize, GNOME Translation Editor or any text
editor can edit. Ferricone compiles every catalog in, so a translation works
in all the packages, the AppImage included.

## Add a language

```sh
scripts/i18n.py new de        # po/de/LC_MESSAGES/ferricone.po from the template
```

Then:

1. Add the language's own name to the file's header, for the Language list in
   Settings: `"X-Language-Name: Deutsch\n"`.
2. Fill in each `msgstr`. Leave one empty to keep the English text.
3. Run `scripts/i18n.py check`, then build and look: Settings › General ›
   Language switches at once, or start Ferricone with `LANG=de_DE.UTF-8`.
4. Open a pull request with the file.

Use a regional code (`pt_BR`, `zh_CN`) when the language differs by region;
System picks the closest one there is.

## Rules of thumb

- `{}` and `{0}`, `{1}` stand for values, `{n}` for a count: keep them, move
  them where the sentence needs them.
- Counts have one `msgstr[…]` per plural form of your language, as its
  `Plural-Forms` header says (`scripts/i18n.py new` fills it in).
- Key names ("Ctrl + O", "Space") may use your keyboard's own words.
- Leave units in editable fields as they are (`s`, `ms`, `deg`, `x`): what's
  typed there is read back.
- A missing or empty translation shows the English text, never nothing.

## For developers

- In `.slint`: `@tr("Text")`, `@tr("Volume {} %", v)`,
  `@tr("{n} Bookmark" | "{n} Bookmarks" % count)`.
- In Rust (`src/i18n.rs`): `tr!("Text")`, `tr!("Volume {} %", v)`,
  `trn!("{n} item", "{n} items", n)`, and `n_!("Text")` for a static table,
  shown with `i18n::t()`. Text Rust builds once registers `i18n::on_change`.
- After changing text, `scripts/i18n.py update` remakes `po/ferricone.pot`
  and merges it into every catalog; CI fails while the template is stale.
