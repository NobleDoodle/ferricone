//! Translations. One catalog per language, `po/<lang>/LC_MESSAGES/ferricone.po`,
//! serves both sides: Slint compiles it into the `.slint` code's `@tr()`, and
//! build.rs into the tables here for the strings Rust shows.
//!
//! - `tr!("text")` or `tr!("Volume {} %", v)`: translated now, as a String.
//! - `trn!("{n} file", "{n} files", n)`: with the language's plural forms.
//! - `n_!("text")`: only marks a literal for `scripts/i18n.py extract` (a
//!   static table, say); show it with `i18n::t()`.
//!
//! Placeholders follow Slint's: `{}` in order, `{0}` by position, `{n}` the count.
//! Rust-built lists register `on_change` to be built again in a new language.

use std::{borrow::Cow, cell::RefCell, sync::atomic::{AtomicUsize, Ordering}};

pub struct Catalog {
    pub code: &'static str,
    /// The language's own name for itself (`X-Language-Name` in the header).
    pub name: &'static str,
    plural: fn(u64) -> usize,
    /// Sorted by msgid; a plural entry's key is `singular \u{4} plural`.
    entries: &'static [(&'static str, &'static [&'static str])],
}

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// Index into CATALOGS, or NONE for the source text.
static CURRENT: AtomicUsize = AtomicUsize::new(NONE);
const NONE: usize = usize::MAX;

thread_local! {
    static LISTENERS: RefCell<Vec<Box<dyn Fn()>>> = RefCell::new(Vec::new());
}

fn catalog() -> Option<&'static Catalog> {
    CATALOGS.get(CURRENT.load(Ordering::Relaxed))
}

fn find(catalog: &Catalog, key: &str) -> Option<&'static [&'static str]> {
    let i = catalog.entries.binary_search_by(|(k, _)| (*k).cmp(key)).ok()?;
    Some(catalog.entries[i].1)
}

/// `text` in the current language, or as written.
pub fn t(text: &str) -> Cow<'_, str> {
    catalog()
        .and_then(|c| find(c, text))
        .and_then(|strs| strs.first().copied())
        .filter(|s| !s.is_empty())
        .map_or(Cow::Borrowed(text), Cow::Borrowed)
}

/// `t()` for static text, staying static (catalogs are compiled in).
pub fn ts(text: &'static str) -> &'static str {
    catalog()
        .and_then(|c| find(c, text))
        .and_then(|strs| strs.first().copied())
        .filter(|s| !s.is_empty())
        .unwrap_or(text)
}

/// The form of `one`/`many` for `n`, with `{n}` replaced.
pub fn tn(one: &str, many: &str, n: u64) -> String {
    let fallback = if n == 1 { one } else { many };
    let text = catalog()
        .and_then(|c| {
            let strs = find(c, &format!("{one}\u{4}{many}"))?;
            strs.get((c.plural)(n)).copied().filter(|s| !s.is_empty())
        })
        .unwrap_or(fallback);
    text.replace("{n}", &n.to_string())
}

/// Fills `{}` in order and `{0}`, `{1}`… by position; `{{` and `}}` are braces.
pub fn format(template: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut next = 0;
    let mut chars = template.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' if chars.peek().map(|&(_, c)| c) == Some('{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek().map(|&(_, c)| c) == Some('}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let Some(end) = template[i..].find('}') else {
                    out.push(c);
                    continue;
                };
                let inner = &template[i + 1..i + end];
                let arg = if inner.is_empty() {
                    next += 1;
                    args.get(next - 1)
                } else {
                    inner.parse::<usize>().ok().and_then(|k| args.get(k))
                };
                match arg {
                    Some(a) => out.push_str(a),
                    None => out.push_str(&template[i..=i + end]),
                }
                while chars.peek().is_some_and(|&(j, _)| j <= i + end) {
                    chars.next();
                }
            }
            c => out.push(c),
        }
    }
    out
}

macro_rules! tr {
    ($text:literal) => {
        $crate::i18n::t($text).into_owned()
    };
    ($text:literal, $($arg:expr),+ $(,)?) => {{
        #[allow(clippy::unnecessary_to_owned)]
        let args = [$(($arg).to_string()),+];
        $crate::i18n::format(&$crate::i18n::t($text), &args)
    }};
}

macro_rules! trn {
    ($one:literal, $many:literal, $n:expr) => {
        $crate::i18n::tn($one, $many, ($n) as u64)
    };
}

/// Marks a literal for extraction only; translate it where it's shown.
macro_rules! n_ {
    ($text:literal) => {
        $text
    };
}

pub(crate) use {n_, tr, trn};

/// The languages Ferricone has, as (code, own name), English first.
pub fn languages() -> Vec<(&'static str, &'static str)> {
    let mut list: Vec<_> = CATALOGS.iter().map(|c| (c.code, c.name)).collect();
    list.sort_by_key(|&(code, name)| (code != "en", name.to_lowercase()));
    list
}

/// The desktop's language, as the catalog that matches it: the whole locale
/// (`pt_BR`), else its language (`pt`). `None` falls back to English.
fn system() -> Option<usize> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())?;
    let locale = locale.split(['.', '@']).next().unwrap_or_default();
    if locale == "C" || locale == "POSIX" {
        return None;
    }
    let base = |l: &str| l.split(['_', '-']).next().unwrap_or(l).to_string();
    CATALOGS
        .iter()
        .position(|c| c.code == locale || c.code.replace('-', "_") == locale)
        .or_else(|| CATALOGS.iter().position(|c| base(c.code) == base(locale)))
}

/// Shows the interface in `language`, a catalog code, or the desktop's when
/// empty; call after the window exists. Both sides switch, then whatever
/// registered with `on_change` builds its text again.
pub fn select(language: &str) {
    let index = match language {
        "" => system(),
        code => CATALOGS.iter().position(|c| c.code == code).or_else(system),
    };
    CURRENT.store(index.unwrap_or(NONE), Ordering::Relaxed);
    let code = index.map_or("", |i| CATALOGS[i].code);
    if let Err(e) = slint::select_bundled_translation(code) {
        eprintln!("ferricone: language {code}: {e:?}");
    }
    LISTENERS.with(|l| l.borrow().iter().for_each(|f| f()));
}

/// Runs `f` after every change of language (on the UI thread).
pub fn on_change(f: impl Fn() + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

/// The code of the language shown, "en" for the source text.
pub fn current() -> &'static str {
    catalog().map_or("en", |c| c.code)
}

/// A number as Rust writes it ("1.25"), with the language's decimal separator.
pub fn number(value: impl std::fmt::Display) -> String {
    let text = value.to_string();
    if comma_language(current()) { text.replace('.', ",") } else { text }
}

/// `value` with `digits` decimals and the language's decimal separator.
pub fn decimal(value: f64, digits: usize) -> String {
    let text = format!("{value:.digits$}");
    if comma_language(current()) { text.replace('.', ",") } else { text }
}

/// Languages that write 1,5 rather than 1.5 (CLDR's decimal symbol);
/// Mexican and US Spanish use the point.
fn comma_language(code: &str) -> bool {
    let base = code.split(['_', '-']).next().unwrap_or(code);
    let comma = matches!(
        base,
        "az" | "be" | "bg" | "bs" | "ca" | "cs" | "da" | "de" | "el" | "es" | "et" | "eu" | "fi"
            | "fr" | "gl" | "hr" | "hu" | "hy" | "id" | "is" | "it" | "ka" | "kk" | "ky" | "lt"
            | "lv" | "mk" | "nb" | "nl" | "nn" | "no" | "pl" | "pt" | "ro" | "ru" | "sk" | "sl"
            | "sq" | "sr" | "sv" | "tr" | "uk" | "uz" | "vi"
    );
    comma && !matches!(code.replace('-', "_").as_str(), "es_MX" | "es_US")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        let args = ["a".to_string(), "b".to_string()];
        assert_eq!(format("{} and {}", &args), "a and b");
        assert_eq!(format("{1} before {0}", &args), "b before a");
        assert_eq!(format("{{literal}} {}", &args), "{literal} a");
        assert_eq!(format("{} {} {}", &args), "a b {}");
    }

    #[test]
    fn source_text_without_a_catalog() {
        CURRENT.store(NONE, Ordering::Relaxed);
        assert_eq!(t("Settings"), "Settings");
        assert_eq!(tn("{n} file", "{n} files", 1), "1 file");
        assert_eq!(tn("{n} file", "{n} files", 3), "3 files");
        assert_eq!(decimal(1.25, 2), "1.25");
    }

    #[test]
    fn english_is_bundled() {
        assert!(languages().first().is_some_and(|&(code, _)| code == "en"));
    }
}
