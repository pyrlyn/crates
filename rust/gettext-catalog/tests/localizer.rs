//! The localizer over real `.po` fixtures: plural selection per language,
//! the per-message fallback chain, negotiation from OS and POSIX tags, and
//! placeholders filled from arguments and constants.

use gettext_catalog::catalog::Catalog;
use gettext_catalog::{Args, Locale, Localizer, Value, format};

static LOCALES: &[Locale] = &[
    Locale {
        code: "en",
        source: include_str!("fixtures/en.po"),
        other_form: 1,
    },
    Locale {
        code: "ru",
        source: include_str!("fixtures/ru.po"),
        other_form: 1,
    },
    Locale {
        code: "uk",
        source: include_str!("fixtures/uk.po"),
        other_form: 1,
    },
];

fn localizer(tag: &str) -> Localizer {
    Localizer::for_tags(LOCALES, "en", &[tag])
        .expect("fixtures parse")
        .with_constant("brand", "Mailune")
}

fn messages(l: &Localizer, count: impl Into<Value>) -> String {
    let mut args = Args::new();
    args.set("count", count);
    l.format("message-count", Some(&args))
}

#[test]
fn every_fixture_parses_and_keeps_only_english_placeholders() {
    let en = Catalog::parse("en", LOCALES[0].source).expect("en");
    for locale in LOCALES {
        let catalog = Catalog::parse(locale.code, locale.source).expect("parses");
        for entry in catalog.entries() {
            let source = en.get(&entry.key).expect("every key is in en");
            let allowed = format::placeholders(&source.msgid);
            for text in entry.forms.iter().chain(&entry.other) {
                format::validate(text).expect("well-formed braces");
                for name in format::placeholders(text) {
                    assert!(allowed.contains(&name), "{}: {{{name}}}", locale.code);
                }
            }
        }
    }
}

#[test]
fn plural_table_for_every_locale() {
    let ns = [0, 1, 2, 5, 11, 21, 22, 25, 111];
    let ru = [
        "сообщений",
        "сообщение",
        "сообщения",
        "сообщений",
        "сообщений",
        "сообщение",
        "сообщения",
        "сообщений",
        "сообщений",
    ];
    let uk = [
        "листів",
        "лист",
        "листи",
        "листів",
        "листів",
        "лист",
        "листи",
        "листів",
        "листів",
    ];
    let en = [
        "messages", "message", "messages", "messages", "messages", "messages", "messages",
        "messages", "messages",
    ];
    let (ru_l, uk_l, en_l) = (localizer("ru"), localizer("uk"), localizer("en"));
    for (i, n) in ns.into_iter().enumerate() {
        assert_eq!(messages(&ru_l, n), format!("{n} {}", ru[i]));
        assert_eq!(messages(&uk_l, n), format!("{n} {}", uk[i]));
        assert_eq!(messages(&en_l, n), format!("{n} {}", en[i]));
    }
}

#[test]
fn negative_fractional_and_text_counts() {
    let (ru, uk, en) = (localizer("ru"), localizer("uk"), localizer("en"));
    // gettext counts are unsigned: the sign is dropped for selection only.
    assert_eq!(messages(&ru, -21), "-21 сообщение");
    // Fractions are CLDR `other`: `other_form`, or a `# cldr-other:` override.
    assert_eq!(messages(&ru, 1.5), "1.5 сообщения");
    assert_eq!(messages(&uk, 2.5), "2.5 листа");
    assert_eq!(messages(&en, 1.5), "1.5 messages");
    // An integral float selects like the integer.
    assert_eq!(messages(&uk, 5.0), "5 листів");
    // A string count is not a number: `other`.
    assert_eq!(messages(&ru, "5"), "5 сообщения");
}

#[test]
fn fallback_is_translation_then_default_msgstr_then_msgid() {
    let ru = localizer("ru");
    assert_eq!(ru.chain(), ["ru", "en"]);
    assert_eq!(ru.format("settings-title", None), "Настройки");
    // Empty in ru.
    assert_eq!(ru.format("send-feedback", None), "Send feedback");
    // Missing from ru, empty in en.
    assert_eq!(ru.format("only-in-english", None), "Archive");
    // Fuzzy in uk.
    assert_eq!(
        localizer("uk").format("send-feedback", None),
        "Send feedback"
    );
}

#[test]
fn missing_key_renders_as_its_id() {
    let ru = localizer("ru");
    assert_eq!(ru.try_format("no-such-message", None), None);
    assert_eq!(ru.format("no-such-message", None), "no-such-message");
    assert_eq!(
        Localizer::default().format("settings-title", None),
        "settings-title"
    );
}

#[test]
fn os_and_posix_tags_select_the_language() {
    assert_eq!(localizer("uk-UA").chain(), ["uk", "en"]);
    assert_eq!(localizer("uk_UA.UTF-8").chain(), ["uk", "en"]);
    assert_eq!(localizer("ru_RU@euro").chain(), ["ru", "en"]);
    assert_eq!(localizer("C").chain(), ["en"]);
    assert_eq!(localizer("de-DE").chain(), ["en"]);
    let many = Localizer::for_tags(LOCALES, "en", &["de", "uk-UA", "ru", "uk"]).expect("builds");
    assert_eq!(many.chain(), ["uk", "ru", "en"]);
}

#[test]
fn constants_and_arguments_fill_placeholders() {
    let mut args = Args::new();
    args.set("name", "Ada");
    assert_eq!(
        localizer("uk").format("welcome-user", Some(&args)),
        "Ласкаво просимо до Mailune, Ada!"
    );
    // An argument wins over a constant of the same name.
    args.set("brand", "Other");
    assert_eq!(
        localizer("en").format("welcome-user", Some(&args)),
        "Welcome to Other, Ada!"
    );
    // A missing argument stays visible as its placeholder.
    assert_eq!(
        localizer("en").format("welcome-user", None),
        "Welcome to Mailune, {name}!"
    );
}
