# gettext-catalog

gettext `.po` catalogs for an application's own strings, in pure Rust (no
libintl):

- `Plural-Forms` rules parsed once and evaluated per lookup, in the C subset
  GNU gettext accepts;
- named `{placeholders}` (`python-brace-format`), with `validate` for catalog
  checks;
- the user's languages from `LC_ALL`, `LC_MESSAGES`, `LANG` and the OS,
  negotiated against the locales you ship;
- per-message fallback: a message a translation lacks, leaves empty or marks
  fuzzy resolves from the next locale, then the default locale, then the
  source text; an unknown id renders as itself;
- constants every message may use, such as a product name.

```rust
use gettext_catalog::{Args, Locale, Localizer};

static LOCALES: &[Locale] = &[
    Locale { code: "en", source: include_str!("../po/en.po"), other_form: 1 },
    Locale { code: "ru", source: include_str!("../po/ru.po"), other_form: 1 },
];

let l = Localizer::from_env(LOCALES, "en")?.with_constant("brand", "MyApp");
let mut args = Args::new();
args.set("count", 3);
println!("{}", l.format("message-count", Some(&args)));
```

Fractions and text counts select CLDR `other`: the `other_form` index, or a
`# cldr-other:` translator comment on the entry.

Extracted from cox's `cox-i18n`.

Licensed under either of MIT or Apache-2.0 at your option.
