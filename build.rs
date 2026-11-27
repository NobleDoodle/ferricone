fn main() {
    // Translations: po/<lang>/LC_MESSAGES/ferricone.po, compiled in, so every
    // package translates without installing locale files. One msgid per text,
    // shared with the Rust side (src/i18n.rs), hence no default context.
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("po")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("compiling ui/app.slint");
    catalogs::write("po", &std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("catalogs.rs"));

    // Development runs load the vendored libVLC. DT_RPATH (not RUNPATH) also
    // covers the private libraries VLC's plugins load from lib/vlc/.
    // Packages set FERRICONE_RPATH ($ORIGIN-relative, packaging/bundle.sh).
    println!("cargo:rerun-if-env-changed=FERRICONE_RPATH");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        if let Ok(rpath) = std::env::var("FERRICONE_RPATH") {
            println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags,-rpath,{rpath}");
        } else if let Ok(lib) = std::env::var("DEP_VLC_LIB") {
            println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags,-rpath,{lib},-rpath,{lib}/vlc");
        }
    }
}

/// The same .po files for the Rust side's strings (src/i18n.rs): a sorted
/// table per language, and its Plural-Forms as a function.
mod catalogs {
    use std::{fmt::Write as _, fs, path::Path};

    #[derive(Default)]
    struct Entry {
        id: String,
        plural: Option<String>,
        strs: Vec<String>,
        fuzzy: bool,
    }

    pub fn write(root: &str, out: &Path) {
        println!("cargo:rerun-if-changed={root}");
        let mut langs: Vec<_> = fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|d| {
                let po = d.path().join("LC_MESSAGES/ferricone.po");
                po.is_file().then(|| (d.file_name().to_string_lossy().into_owned(), po))
            })
            .collect();
        langs.sort();
        let mut code = String::from("pub static CATALOGS: &[Catalog] = &[\n");
        let mut fns = String::new();
        for (i, (lang, po)) in langs.iter().enumerate() {
            println!("cargo:rerun-if-changed={}", po.display());
            let text = fs::read_to_string(po).unwrap_or_else(|e| panic!("{}: {e}", po.display()));
            let entries = parse(&text).unwrap_or_else(|e| panic!("{}: {e}", po.display()));
            let header = entries.iter().find(|e| e.id.is_empty()).map(|e| e.strs.join("")).unwrap_or_default();
            let field = |name: &str| {
                header.lines().find_map(|l| l.strip_prefix(name).map(|v| v.trim().to_string()))
            };
            let rule = field("Plural-Forms:")
                .and_then(|f| f.split(';').find_map(|p| p.trim().strip_prefix("plural=").map(str::to_string)))
                .unwrap_or_else(|| "n != 1".into());
            let expr = plural::rust(&rule).unwrap_or_else(|e| panic!("{}: Plural-Forms: {e}", po.display()));
            writeln!(fns, "#[allow(unused_parens, clippy::all)]\nfn plural_{i}(n: u64) -> usize {{ let n = n as i64; ({expr}) as usize }}").unwrap();
            let name = field("X-Language-Name:").unwrap_or_else(|| lang.clone());
            let mut rows: Vec<_> = entries
                .into_iter()
                .filter(|e| !e.id.is_empty() && !e.fuzzy && e.strs.iter().any(|s| !s.is_empty()))
                .map(|e| {
                    let key = match &e.plural { Some(p) => format!("{}\u{4}{}", e.id, p), None => e.id };
                    (key, e.strs)
                })
                .collect();
            rows.sort();
            writeln!(code, "    Catalog {{ code: {lang:?}, name: {name:?}, plural: plural_{i}, entries: &[").unwrap();
            for (key, strs) in rows {
                writeln!(code, "        ({key:?}, &{strs:?}),").unwrap();
            }
            code.push_str("    ] },\n");
        }
        code.push_str("];\n");
        fs::write(out, fns + &code).unwrap();
    }

    /// Entries end at a blank line, or where a new `msgid` or flags line
    /// follows a `msgstr`.
    fn parse(text: &str) -> Result<Vec<Entry>, String> {
        #[derive(Clone, Copy, PartialEq)]
        enum Field {
            None,
            Ctxt,
            Id,
            Plural,
            Str(usize),
        }
        let mut entries = vec![];
        let mut e = Entry::default();
        let (mut field, mut started, mut seen_str) = (Field::None, false, false);
        let mut flush = |e: &mut Entry, started: &mut bool, seen_str: &mut bool| {
            if *started {
                entries.push(std::mem::take(e));
            }
            *e = Entry::default();
            (*started, *seen_str) = (false, false);
        };
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            let at = |what: &str| format!("line {}: {what}", n + 1);
            if line.is_empty() {
                flush(&mut e, &mut started, &mut seen_str);
                field = Field::None;
                continue;
            }
            if let Some(flags) = line.strip_prefix("#,") {
                if seen_str {
                    flush(&mut e, &mut started, &mut seen_str);
                }
                e.fuzzy |= flags.split(',').any(|f| f.trim() == "fuzzy");
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            let rest = if line.starts_with('"') {
                line
            } else {
                let (key, rest) = line.split_once(char::is_whitespace).ok_or_else(|| at(line))?;
                if (key == "msgid" || key == "msgctxt") && seen_str {
                    flush(&mut e, &mut started, &mut seen_str);
                }
                started = true;
                field = match key {
                    "msgctxt" => Field::Ctxt,
                    "msgid" => Field::Id,
                    "msgid_plural" => Field::Plural,
                    "msgstr" => Field::Str(0),
                    k if k.starts_with("msgstr[") && k.ends_with(']') => {
                        Field::Str(k[7..k.len() - 1].parse().map_err(|_| at(k))?)
                    }
                    k => return Err(at(&format!("unknown keyword {k}"))),
                };
                if let Field::Str(i) = field {
                    seen_str = true;
                    if e.strs.len() <= i {
                        e.strs.resize(i + 1, String::new());
                    }
                }
                rest.trim()
            };
            let value = unquote(rest).ok_or_else(|| at("bad string"))?;
            match field {
                Field::Id => e.id.push_str(&value),
                Field::Plural => e.plural.get_or_insert_with(String::new).push_str(&value),
                Field::Str(i) => e.strs[i].push_str(&value),
                Field::Ctxt => {}
                Field::None => return Err(at("text outside an entry")),
            }
        }
        flush(&mut e, &mut started, &mut seen_str);
        Ok(entries)
    }

    fn unquote(s: &str) -> Option<String> {
        let inner = s.strip_prefix('"')?.strip_suffix('"')?;
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            out.push(match chars.next()? {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                c => c,
            });
        }
        Some(out)
    }

    /// Plural-Forms' C expression in `n`, as Rust over i64 (C's truth values).
    pub mod plural {
        pub fn rust(expr: &str) -> Result<String, String> {
            let tokens = tokens(expr)?;
            let mut p = Parser { tokens, pos: 0 };
            let out = p.ternary()?;
            if p.pos != p.tokens.len() {
                return Err(format!("unexpected {:?}", p.tokens[p.pos]));
            }
            Ok(out)
        }

        fn tokens(s: &str) -> Result<Vec<String>, String> {
            let mut out = vec![];
            let b = s.as_bytes();
            let mut i = 0;
            while i < b.len() {
                let c = b[i] as char;
                if c.is_whitespace() {
                    i += 1;
                } else if c.is_ascii_digit() {
                    let start = i;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    out.push(s[start..i].to_string());
                } else if c == 'n' {
                    out.push("n".into());
                    i += 1;
                } else if let Some(op) = ["==", "!=", "<=", ">=", "&&", "||"].iter().find(|op| s[i..].starts_with(**op)) {
                    out.push(op.to_string());
                    i += 2;
                } else if "?:()<>%+-*/!".contains(c) {
                    out.push(c.to_string());
                    i += 1;
                } else {
                    return Err(format!("unexpected {c:?}"));
                }
            }
            Ok(out)
        }

        struct Parser {
            tokens: Vec<String>,
            pos: usize,
        }

        impl Parser {
            fn peek(&self) -> Option<&str> {
                self.tokens.get(self.pos).map(String::as_str)
            }
            fn eat(&mut self, t: &str) -> bool {
                let hit = self.peek() == Some(t);
                self.pos += usize::from(hit);
                hit
            }
            fn ternary(&mut self) -> Result<String, String> {
                let cond = self.binary(0)?;
                if !self.eat("?") {
                    return Ok(cond);
                }
                let a = self.ternary()?;
                if !self.eat(":") {
                    return Err("missing :".into());
                }
                let b = self.ternary()?;
                Ok(format!("(if ({cond}) != 0 {{ {a} }} else {{ {b} }})"))
            }
            fn binary(&mut self, level: usize) -> Result<String, String> {
                const LEVELS: &[&[&str]] = &[&["||"], &["&&"], &["==", "!="], &["<", ">", "<=", ">="], &["+", "-"], &["*", "/", "%"]];
                if level == LEVELS.len() {
                    return self.unary();
                }
                let mut left = self.binary(level + 1)?;
                while let Some(op) = self.peek().filter(|t| LEVELS[level].contains(t)).map(str::to_string) {
                    self.pos += 1;
                    let right = self.binary(level + 1)?;
                    left = match op.as_str() {
                        "||" => format!("(i64::from(({left}) != 0 || ({right}) != 0))"),
                        "&&" => format!("(i64::from(({left}) != 0 && ({right}) != 0))"),
                        "+" | "-" | "*" | "/" | "%" => format!("(({left}) {op} ({right}))"),
                        cmp => format!("(i64::from(({left}) {cmp} ({right})))"),
                    };
                }
                Ok(left)
            }
            fn unary(&mut self) -> Result<String, String> {
                if self.eat("!") {
                    return Ok(format!("(i64::from(({}) == 0))", self.unary()?));
                }
                if self.eat("(") {
                    let inner = self.ternary()?;
                    return if self.eat(")") { Ok(format!("({inner})")) } else { Err("missing )".into()) };
                }
                match self.peek() {
                    Some(t) if t == "n" || t.chars().all(|c| c.is_ascii_digit()) => {
                        let t = t.to_string();
                        self.pos += 1;
                        Ok(if t == "n" { "n".into() } else { format!("{t}i64") })
                    }
                    t => Err(format!("unexpected {t:?}")),
                }
            }
        }
    }
}
