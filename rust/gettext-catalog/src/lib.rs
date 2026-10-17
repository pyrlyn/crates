//! gettext catalogs (`.po`) for an application's own strings: parse them,
//! negotiate the user's languages against the locales the application ships,
//! and turn a message id into text with named `{placeholders}` and plural
//! forms. A message a translation lacks, leaves empty or marks fuzzy resolves
//! from the next locale in the chain, ending at the default locale and then
//! the source text, so a partial translation is safe to ship.
//!
//! Pure Rust: catalogs are parsed with `polib` and `Plural-Forms` rules
//! evaluated by [`plural`], so there is no libintl to install. Extracted from
//! cox's `cox-i18n`; the application keeps its catalogs, its process-wide
//! localizer and any native-catalog export.
//!
//! ```
//! use gettext_catalog::{Locale, Localizer};
//!
//! static LOCALES: &[Locale] = &[Locale {
//!     code: "en",
//!     source: "msgid \"\"\nmsgstr \"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n\n\
//!              msgctxt \"hello\"\nmsgid \"Hello from {brand}\"\nmsgstr \"\"\n",
//!     other_form: 1,
//! }];
//!
//! let l = Localizer::for_tags(LOCALES, "en", &["de_DE.UTF-8"])?.with_constant("brand", "Mailune");
//! assert_eq!(l.format("hello", None), "Hello from Mailune");
//! assert_eq!(l.format("missing", None), "missing");
//! # Ok::<(), gettext_catalog::Error>(())
//! ```

pub mod catalog;
pub mod format;
pub mod plural;

use std::borrow::Cow;
use std::fmt;

pub use unic_langid::LanguageIdentifier;

use crate::catalog::{Catalog, Entry};

/// One locale an application ships.
#[derive(Debug, Clone, Copy)]
pub struct Locale {
    /// BCP 47 language subtag: the negotiation target and the catalog name.
    pub code: &'static str,
    /// The text of the locale's `.po` file.
    pub source: &'static str,
    /// The form CLDR `other` uses when the gettext forms have none of their
    /// own (ru and uk: `other` covers only fractions) and the translator
    /// wrote no `# cldr-other:` override. Also used for a non-numeric `count`.
    pub other_form: usize,
}

/// Why a catalog, a plural rule or a placeholder was rejected.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// polib could not read the catalog.
    #[error("locale `{code}`: {detail}")]
    Parse {
        /// The locale being parsed.
        code: String,
        /// polib's message.
        detail: String,
    },
    /// The catalog's `Plural-Forms` header is outside the gettext grammar.
    #[error("locale `{code}`: bad Plural-Forms: {detail}")]
    PluralForms {
        /// The locale being parsed.
        code: String,
        /// What is wrong with the rule.
        detail: String,
    },
    /// A `Plural-Forms` expression is outside the gettext grammar.
    #[error("{0}")]
    PluralExpr(String),
    /// Two entries share a key.
    #[error("locale `{code}`: message `{key}` is defined twice")]
    Duplicate {
        /// The locale being parsed.
        code: String,
        /// The repeated `msgctxt` or `msgid`.
        key: String,
    },
    /// A brace that is neither doubled nor part of a `{name}` placeholder.
    #[error("{0}")]
    Placeholder(String),
}

/// A message argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Text.
    Str(String),
    /// A whole number; as `count` it selects the plural form.
    Int(i64),
    /// A number with a fraction; as `count` it selects CLDR `other`.
    Float(f64),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => f.write_str(s),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_owned())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Self {
        Value::Str(s.clone())
    }
}
macro_rules! int_value {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(v: $t) -> Self {
                Value::Int(i64::try_from(v).unwrap_or(i64::MAX))
            }
        }
    )*};
}
int_value!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(f64::from(v))
    }
}

/// Named message arguments, filling `{name}` placeholders. The argument
/// named `count` also selects the plural form.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args {
    values: Vec<(Cow<'static, str>, Value)>,
}

impl Args {
    /// No arguments.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `name`, replacing an earlier value.
    pub fn set(&mut self, name: impl Into<Cow<'static, str>>, value: impl Into<Value>) {
        let name = name.into();
        let value = value.into();
        match self.values.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => self.values.push((name, value)),
        }
    }

    /// The value of `name`, if set.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

/// How the `count` argument selects a plural form.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Count {
    /// An integer (the sign dropped: gettext counts are unsigned).
    Whole(u64),
    /// A fraction, a string or no `count` at all: CLDR `other`.
    Other,
}

fn count_of(args: Option<&Args>) -> Count {
    match args.and_then(|a| a.get("count")) {
        Some(Value::Int(i)) => Count::Whole(i.unsigned_abs()),
        Some(Value::Float(x)) if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e19 => {
            // Truncation is exact here: the value is integral and in range.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Count::Whole(x.abs() as u64)
        }
        _ => Count::Other,
    }
}

/// A negotiated chain of catalogs, most preferred first and the default
/// locale last, plus constants every message may use (a product name).
#[derive(Debug, Default)]
pub struct Localizer {
    catalogs: Vec<(&'static Locale, Catalog)>,
    constants: Vec<(String, String)>,
}

impl Localizer {
    /// Catalogs for `requested` (most preferred first) negotiated against
    /// `locales`, ending in `default` when `locales` has it.
    ///
    /// # Errors
    ///
    /// The first catalog in the chain that does not parse.
    pub fn new(
        locales: &'static [Locale],
        default: &str,
        requested: &[LanguageIdentifier],
    ) -> Result<Self, Error> {
        let catalogs = negotiate(locales, default, requested)
            .into_iter()
            .map(|locale| Ok((locale, Catalog::parse(locale.code, locale.source)?)))
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            catalogs,
            constants: Vec::new(),
        })
    }

    /// [`Localizer::new`] over raw tags as the OS or the environment spell
    /// them (`uk_UA.UTF-8`, `ru-RU`); tags that do not parse are skipped.
    ///
    /// # Errors
    ///
    /// As [`Localizer::new`].
    pub fn for_tags<S: AsRef<str>>(
        locales: &'static [Locale],
        default: &str,
        tags: &[S],
    ) -> Result<Self, Error> {
        let requested: Vec<_> = tags.iter().filter_map(|t| parse_tag(t.as_ref())).collect();
        Self::new(locales, default, &requested)
    }

    /// The user's languages from the environment, then the OS
    /// ([`requested_languages`]).
    ///
    /// # Errors
    ///
    /// As [`Localizer::new`].
    pub fn from_env(locales: &'static [Locale], default: &str) -> Result<Self, Error> {
        Self::new(locales, default, &requested_languages())
    }

    /// Fills `{name}` with `value` in every message whose arguments do not
    /// set `name` themselves.
    #[must_use]
    pub fn with_constant(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let (name, value) = (name.into(), value.into());
        self.constants.retain(|(n, _)| *n != name);
        self.constants.push((name, value));
        self
    }

    /// The negotiated locale codes, most preferred first.
    pub fn chain(&self) -> Vec<&'static str> {
        self.catalogs.iter().map(|(l, _)| l.code).collect()
    }

    /// The first locale in the chain with a usable translation of `id`
    /// (`msgctxt`, or `msgid` for an entry without one), formatted; when none
    /// has one, the source text (`msgid`/`msgid_plural`) of the entry. `None`
    /// when no catalog defines `id`.
    pub fn try_format(&self, id: &str, args: Option<&Args>) -> Option<String> {
        let count = count_of(args);
        let template = self
            .catalogs
            .iter()
            .find_map(|(locale, catalog)| pick(locale, catalog, catalog.get(id)?, count))
            .or_else(|| {
                let entry = self.catalogs.iter().rev().find_map(|(_, c)| c.get(id))?;
                Some(source_text(entry, count))
            })?;
        Some(format::render(template, |name| {
            args.and_then(|a| a.get(name))
                .map(ToString::to_string)
                .or_else(|| {
                    self.constants
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.clone())
                })
        }))
    }

    /// [`Localizer::try_format`], or the id itself when nothing defines it, so
    /// a missing string shows up in the UI as its id instead of blank space.
    pub fn format(&self, id: &str, args: Option<&Args>) -> String {
        self.try_format(id, args).unwrap_or_else(|| id.to_owned())
    }
}

/// The translated form of `entry` for `count` in `locale`, if it has one.
fn pick<'a>(locale: &Locale, catalog: &Catalog, entry: &'a Entry, count: Count) -> Option<&'a str> {
    let forms = entry.translated(catalog.rule.nplurals())?;
    if entry.msgid_plural.is_none() {
        return forms.first().map(String::as_str);
    }
    match count {
        Count::Whole(n) => forms.get(catalog.rule.index(n)).map(String::as_str),
        Count::Other => entry
            .other
            .as_deref()
            .or_else(|| forms.get(locale.other_form).map(String::as_str)),
    }
}

/// The source text of `entry`, with English plural selection: gettext source
/// strings are English by convention.
fn source_text(entry: &Entry, count: Count) -> &str {
    match (&entry.msgid_plural, count) {
        (Some(_), Count::Whole(1)) | (None, _) => &entry.msgid,
        (Some(plural), _) => plural,
    }
}

/// Negotiates `requested` against `locales` by language subtag, in request
/// order (so `uk-UA` and `uk_UA.UTF-8` select `uk`), appending `default`
/// when it is not already in the chain.
pub fn negotiate(
    locales: &'static [Locale],
    default: &str,
    requested: &[LanguageIdentifier],
) -> Vec<&'static Locale> {
    let mut chain: Vec<&'static Locale> = Vec::new();
    let codes = requested
        .iter()
        .map(|id| id.language.as_str())
        .chain([default]);
    for code in codes {
        if let Some(locale) = locales.iter().find(|l| l.code == code)
            && !chain.iter().any(|l| l.code == code)
        {
            chain.push(locale);
        }
    }
    chain
}

/// Parses a tag as POSIX environment variables and OS APIs spell it:
/// `uk_UA.UTF-8`, `ru_RU@euro`, `en-US`. `C` and `POSIX` carry no language
/// and give `None`.
pub fn parse_tag(raw: &str) -> Option<LanguageIdentifier> {
    let tag = raw.split(['.', '@']).next()?.trim().replace('_', "-");
    if tag.is_empty() || tag.eq_ignore_ascii_case("c") || tag.eq_ignore_ascii_case("posix") {
        return None;
    }
    tag.parse().ok()
}

/// The user's languages, most preferred first: the POSIX message locale
/// (`LC_ALL`, `LC_MESSAGES`, `LANG`, first one set wins), then the OS UI
/// languages (macOS and Windows preferences; a GUI app gets no `LANG`).
pub fn requested_languages() -> Vec<LanguageIdentifier> {
    requested_from(|name| std::env::var(name).ok(), sys_locale::get_locales())
}

fn requested_from(
    env: impl Fn(&str) -> Option<String>,
    os: impl IntoIterator<Item = String>,
) -> Vec<LanguageIdentifier> {
    let posix = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| env(name).filter(|v| !v.is_empty()));
    posix
        .into_iter()
        .chain(os)
        .filter_map(|tag| parse_tag(&tag))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn languages(ids: &[LanguageIdentifier]) -> Vec<String> {
        ids.iter().map(|id| id.language.to_string()).collect()
    }

    #[test]
    fn posix_env_wins_over_os_languages() {
        let env = |name: &str| (name == "LANG").then(|| "uk_UA.UTF-8".to_owned());
        let requested = requested_from(env, ["ru-RU".to_owned()]);
        assert_eq!(languages(&requested), ["uk", "ru"]);
        // LC_ALL beats LANG; an empty value counts as unset.
        let env = |name: &str| match name {
            "LC_ALL" => Some(String::new()),
            "LC_MESSAGES" => Some("ru_RU.UTF-8".to_owned()),
            "LANG" => Some("uk_UA.UTF-8".to_owned()),
            _ => None,
        };
        assert_eq!(languages(&requested_from(env, [])), ["ru"]);
    }

    #[test]
    fn counts_select_whole_or_other() {
        let with = |v: Value| {
            let mut a = Args::new();
            a.set("count", v);
            count_of(Some(&a))
        };
        assert_eq!(with(Value::Int(-21)), Count::Whole(21));
        assert_eq!(with(Value::Float(5.0)), Count::Whole(5));
        assert_eq!(with(Value::Float(1.5)), Count::Other);
        assert_eq!(with(Value::Float(f64::NAN)), Count::Other);
        assert_eq!(with("5".into()), Count::Other);
        assert_eq!(count_of(None), Count::Other);
    }

    #[test]
    fn tags_parse_like_posix_and_os_apis() {
        assert_eq!(
            parse_tag("uk_UA.UTF-8").map(|t| t.to_string()),
            Some("uk-UA".into())
        );
        assert_eq!(
            parse_tag("ru_RU@euro").map(|t| t.to_string()),
            Some("ru-RU".into())
        );
        assert!(parse_tag("C").is_none());
        assert!(parse_tag("POSIX").is_none());
        assert!(parse_tag("").is_none());
    }
}
