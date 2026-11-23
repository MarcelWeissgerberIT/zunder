//! Übersetzer für Filterlisten: aus der Schreibweise von Adblock Plus
//! (EasyList und Verwandte) in das Regelformat von WebKit.
//!
//! Die Listen sind fremde Daten aus dem Netz. Deshalb entsteht hier nichts,
//! was mehr kann als sperren, ausblenden und Ausnahmen davon machen – kein
//! Skript, keine Umleitung, kein veränderter Seiteninhalt. Alles, was sich in
//! WebKits Regelsprache nicht GENAU ausdrücken lässt, fällt weg, und zwar immer
//! zur vorsichtigen Seite: Eine Sperre wird höchstens enger, eine Ausnahme
//! höchstens weiter als in der Liste.
//!
//! Zwei Eigenheiten von WebKit bestimmen den Aufbau:
//! * Ein einziges ungültiges Suchmuster lässt die ganze Liste scheitern. Die
//!   Muster entstehen deshalb nur aus festen Bausteinen (`PREFIX`, `SEP` …),
//!   und `filter_is_safe` prüft jedes noch einmal nach.
//! * Eine Ausnahme („ignore-previous-rules“) wirkt nur innerhalb ihrer eigenen
//!   Liste und nur auf Regeln davor. Darum stehen die Ausnahmen am Ende – und
//!   in JEDEM Teil, wenn die Sperren auf mehrere Listen verteilt werden.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Hochzählen, wenn sich an der Übersetzung etwas ändert: Dann werden die
/// zwischengespeicherten Listen neu gebaut.
pub const CONVERTER: u32 = 1;

pub struct Source {
    /// Dateiname ohne Endung.
    pub name: &'static str,
    pub title: &'static str,
    pub url: &'static str,
}

/// Die Listen, die Zunder mitbringt und auffrischt.
pub const SOURCES: &[Source] = &[
    Source { name: "easylist", title: "EasyList", url: "https://easylist.to/easylist/easylist.txt" },
    Source {
        name: "easylistgermany",
        title: "EasyList Germany",
        url: "https://easylist.to/easylistgermany/easylistgermany.txt",
    },
    Source { name: "easyprivacy", title: "EasyPrivacy", url: "https://easylist.to/easylist/easyprivacy.txt" },
];

/// Listen gemeldeter Betrugs- und Schadseiten. Sie liegen der App nicht bei
/// (sie veralten binnen Stunden) und werden nach dem Start geholt.
pub const GUARD_SOURCES: &[Source] = &[
    Source {
        name: "phishing",
        title: "Phishing URL Blocklist",
        url: "https://malware-filter.gitlab.io/malware-filter/phishing-filter.txt",
    },
    Source {
        name: "badware",
        title: "uBlock filters – Badware risks",
        url: "https://ublockorigin.github.io/uAssets/filters/badware.txt",
    },
];

/// Höchstens so viele Sperren je übersetzter Liste (dazu kommen die Ausnahmen).
const CHUNK: usize = 40_000;

/// Vor dem Rechnernamen: Schema (http, https, ws, wss) und beliebig viele
/// vorangestellte Namensteile – `||beispiel.de` trifft auch `a.b.beispiel.de`.
const PREFIX: &str = r"^[htpsw]+://([^:/]+\.)?";
/// Ein Trennzeichen im Sinn der Listen: alles außer Buchstaben, Ziffern und `_ . % -`.
const SEP: &str = "[^a-zA-Z0-9_.%-]";
/// Grenzen gegen manipulierte Listen. Echte bleiben weit darunter: Die drei
/// großen zusammen haben einige Tausend Ausnahmen und rund 70.000
/// Ausblend-Regeln, und keine Regel nennt mehr als ein paar Hundert Websites.
const MAX_EXCEPTIONS: usize = 60_000;
const MAX_LIST_RULES: usize = 140_000;
const MAX_SPARED: usize = 2_000;

/// Port oder Pfad.
const PORT_OR_PATH: &str = "[:/]";
/// Ein Punkt, der hinter einem vollständigen Rechnernamen stehen darf:
/// `beispiel.de.` ist derselbe Rechner wie `beispiel.de`. Ohne ihn käme jede
/// Adresse mit dieser Schreibweise an den Listen vorbei.
const ROOT_DOT: &str = r"\.?";
/// Nach einem vollständigen Rechnernamen: (Punkt,) Port oder Pfad.
const HOST_END: &str = r"\.?[:/]";

const T_IMAGE: u16 = 1 << 0;
const T_STYLE: u16 = 1 << 1;
const T_SCRIPT: u16 = 1 << 2;
const T_FONT: u16 = 1 << 3;
const T_MEDIA: u16 = 1 << 4;
const T_FETCH: u16 = 1 << 5;
const T_SOCKET: u16 = 1 << 6;
const T_PING: u16 = 1 << 7;
const T_OTHER: u16 = 1 << 8;
/// Eingebettete Rahmen.
const T_FRAME: u16 = 1 << 9;
const T_POPUP: u16 = 1 << 10;
/// Die Seite selbst (oberster Rahmen).
const T_DOC: u16 = 1 << 11;

/// Ohne Angabe gilt eine Regel für alles, was eine Seite nachlädt – aber weder
/// für die Seite selbst noch für neue Fenster. Sonst führte kein Link mehr zu
/// einer Adresse, die auf einer Liste steht.
const T_DEFAULT: u16 =
    T_IMAGE | T_STYLE | T_SCRIPT | T_FONT | T_MEDIA | T_FETCH | T_SOCKET | T_PING | T_OTHER | T_FRAME;
/// Wirklich alles – auch die Seite selbst und neue Fenster (`$all`).
const T_ALL: u16 = T_DEFAULT | T_POPUP | T_DOC;

const TYPE_NAMES: &[(u16, &str)] = &[
    (T_IMAGE, "image"),
    (T_IMAGE, "svg-document"),
    (T_STYLE, "style-sheet"),
    (T_SCRIPT, "script"),
    (T_FONT, "font"),
    (T_MEDIA, "media"),
    (T_FETCH, "fetch"),
    (T_SOCKET, "websocket"),
    (T_PING, "ping"),
    (T_OTHER, "other"),
    (T_POPUP, "popup"),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Party {
    Any,
    Third,
    First,
}

/// Ausnahmen, die nicht einzelne Abrufe freigeben, sondern das Ausblenden auf
/// einer Seite abschalten.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Unhide {
    No,
    /// Nur die allgemeinen Ausblend-Regeln (`$generichide`).
    Generic,
    /// Alle Ausblend-Regeln (`$elemhide`).
    All,
}

#[derive(Debug)]
struct Net {
    exception: bool,
    /// Ein oder zwei Suchmuster (zwei, wenn die Regel auch am Adressende greift).
    filters: Vec<String>,
    /// Das Muster trifft jede Adresse.
    universal: bool,
    case: bool,
    types: u16,
    party: Party,
    include: Vec<String>,
    exclude: Vec<String>,
    unhide: Unhide,
    /// Ausnahme für ganze Seiten (`@@…$document`): dort gilt gar keine Regel.
    whole_page: bool,
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub blocking: usize,
    pub exceptions: usize,
    pub hiding: usize,
    /// Zeilen, die sich nicht genau ausdrücken ließen.
    pub skipped: usize,
}

impl Stats {
    pub fn rules(&self) -> usize {
        self.blocking + self.exceptions + self.hiding
    }
}

pub struct Converted {
    /// Fertige Regellisten als JSON – jede für sich vollständig.
    pub lists: Vec<String>,
    pub stats: Stats,
}

// ---------------------------------------------------------------------------
// Suchmuster

enum Start {
    Any,
    Line,
    Host,
}

struct Filters {
    list: Vec<String>,
    universal: bool,
}

fn escape_into(out: &mut String, c: char) {
    if matches!(c, '\\' | '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '$' | '^' | '*') {
        out.push('\\');
    }
    out.push(c);
}

/// Macht aus dem Adressteil einer Regel ein oder zwei Suchmuster für WebKit.
/// `None`: nicht ausdrückbar (eigene reguläre Ausdrücke, fremde Zeichen …).
fn url_filters(pattern: &str) -> Option<Filters> {
    if pattern.len() > 600 || !pattern.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return None;
    }
    // Reguläre Ausdrücke der Listen kennen Dinge, die WebKit nicht versteht.
    if pattern.len() > 1 && pattern.starts_with('/') && pattern.ends_with('/') {
        return None;
    }
    let (start, mut body) = if let Some(rest) = pattern.strip_prefix("||") {
        (Start::Host, rest)
    } else if let Some(rest) = pattern.strip_prefix('|') {
        (Start::Line, rest)
    } else {
        (Start::Any, pattern)
    };
    let end = body.ends_with('|');
    if end {
        body = &body[..body.len() - 1];
    }
    if matches!(start, Start::Any) {
        body = body.trim_start_matches('*');
    }
    if !end {
        body = body.trim_end_matches('*');
    }
    if body.is_empty() {
        return Some(Filters { list: vec![".*".to_string()], universal: true });
    }
    let literal = body.bytes().filter(|b| !matches!(b, b'*' | b'^')).count();
    if literal == 0 {
        return None;
    }
    // Ein, zwei Zeichen irgendwo in der Adresse träfen fast alles – so etwas
    // steht in keiner ernst gemeinten Liste.
    if matches!(start, Start::Any) && literal < 3 {
        return None;
    }

    // Häufigster Fall: nur ein Rechnername.
    if matches!(start, Start::Host) && !end {
        let (host, closed) = match body.strip_suffix('^') {
            Some(host) => (host, true),
            None => (body, false),
        };
        let plain = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if plain {
            let mut out = String::with_capacity(PREFIX.len() + host.len() + 12);
            out.push_str(PREFIX);
            for c in host.chars() {
                escape_into(&mut out, c);
            }
            if closed {
                out.push_str(HOST_END);
            }
            return Some(Filters { list: vec![out], universal: false });
        }
    }

    let mut out = String::with_capacity(PREFIX.len() + body.len() * 2);
    match start {
        Start::Host => out.push_str(PREFIX),
        Start::Line => out.push('^'),
        Start::Any => {}
    }
    // Ein Trennzeichen ganz am Ende gilt in den Listen auch dann als erfüllt,
    // wenn die Adresse dort aufhört. Ein „oder“ kennt WebKit nicht – also zwei Muster.
    let tail_sep = !end && body.ends_with('^');
    let core = if tail_sep { &body[..body.len() - 1] } else { body };
    // `||rechner/pfad`: Wo der Rechnername endet (vor Pfad, Port oder
    // Trennzeichen), darf der Punkt stehen. Das Muster ist reines ASCII –
    // Stellen in Bytes und in Zeichen sind dasselbe.
    let host_end = if matches!(start, Start::Host) {
        let run = core.bytes().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')).count();
        let complete = run > 0 && !core[..run].ends_with('.') && matches!(core.as_bytes().get(run), Some(b'/' | b'^' | b':'));
        complete.then_some(run)
    } else {
        None
    };
    let mut last_star = false;
    for (index, c) in core.char_indices() {
        if host_end == Some(index) {
            out.push_str(ROOT_DOT);
        }
        match c {
            '*' => {
                if !last_star {
                    out.push_str(".*");
                }
                last_star = true;
                continue;
            }
            '^' => out.push_str(SEP),
            c => escape_into(&mut out, c),
        }
        last_star = false;
    }
    let list = if tail_sep {
        vec![format!("{out}{SEP}"), format!("{out}$")]
    } else if end {
        out.push('$');
        vec![out]
    } else {
        vec![out]
    };
    Some(Filters { list, universal: false })
}

/// Prüft ein fertiges Suchmuster gegen das, was WebKit sicher versteht: kein
/// „oder“, keine Wiederholungszähler, keine Zeichenklassen-Kürzel, `^` nur
/// vorn, `$` nur hinten. Ein Muster, das hier durchfällt, wird weggelassen –
/// sonst nähme WebKit die ganze Liste nicht an.
pub fn filter_is_safe(filter: &str) -> bool {
    if filter == ".*" {
        return true;
    }
    let (rest, anchored) = match filter.strip_prefix(PREFIX) {
        Some(rest) => (rest, true),
        None => match filter.strip_prefix('^') {
            Some(rest) => (rest, true),
            None => (filter, false),
        },
    };
    let rest = rest.strip_suffix('$').filter(|r| !r.ends_with('\\') || r.ends_with("\\\\")).unwrap_or(rest);
    if rest.is_empty() {
        return anchored && filter.len() > 1;
    }
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !(0x21..=0x7e).contains(&b) {
            return false;
        }
        match b {
            b'\\' => {
                // Nur Satzzeichen dürfen so geschützt sein – `\d`, `\w`, `\b` gibt es nicht.
                match bytes.get(i + 1) {
                    // Der Punkt hinter einem Rechnernamen – das einzige „vielleicht“ in den Mustern.
                    Some(b'.') if bytes.get(i + 2) == Some(&b'?') => i += ROOT_DOT.len(),
                    Some(next) if next.is_ascii_punctuation() => i += 2,
                    _ => return false,
                }
            }
            b'[' => {
                if rest[i..].starts_with(SEP) {
                    i += SEP.len();
                } else if rest[i..].starts_with(PORT_OR_PATH) {
                    i += PORT_OR_PATH.len();
                } else {
                    return false;
                }
            }
            b'.' => {
                i += 1;
                if bytes.get(i) == Some(&b'*') {
                    i += 1;
                }
            }
            b'*' | b'+' | b'?' | b'(' | b')' | b']' | b'{' | b'}' | b'|' | b'^' | b'$' => return false,
            _ => i += 1,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Zeilen lesen

/// Trennt die Optionen (`$script,domain=…`) vom Adressteil. Wie in den Listen
/// üblich zählt das erste `$`, hinter dem bis zum Ende nur Optionen stehen.
fn split_options(body: &str) -> (&str, &str) {
    for (index, _) in body.match_indices('$') {
        let options = &body[index + 1..];
        if !options.is_empty() && options.split(',').all(option_shaped) {
            return (&body[..index], options);
        }
    }
    (body, "")
}

fn option_shaped(item: &str) -> bool {
    let item = item.strip_prefix('~').unwrap_or(item);
    let name = item.split('=').next().unwrap_or("");
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn domain_ok(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
}

struct Domains {
    include: Vec<String>,
    exclude: Vec<String>,
    /// Es standen Einträge da, die sich nicht ausdrücken lassen (`beispiel.*`).
    lost_include: bool,
    lost_exclude: bool,
}

fn split_domains(list: &str, separator: char) -> Domains {
    let mut out = Domains { include: Vec::new(), exclude: Vec::new(), lost_include: false, lost_exclude: false };
    for entry in list.split(separator) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (negated, name) = match entry.strip_prefix('~') {
            Some(name) => (true, name),
            None => (false, entry),
        };
        let name = name.to_ascii_lowercase();
        match (domain_ok(&name), negated) {
            (true, false) => out.include.push(name),
            (true, true) => out.exclude.push(name),
            (false, false) => out.lost_include = true,
            (false, true) => out.lost_exclude = true,
        }
    }
    out
}

fn type_bit(name: &str) -> Option<u16> {
    Some(match name {
        "script" => T_SCRIPT,
        "image" => T_IMAGE,
        "stylesheet" | "css" => T_STYLE,
        "font" => T_FONT,
        "media" => T_MEDIA,
        "xmlhttprequest" | "xhr" => T_FETCH,
        "websocket" => T_SOCKET,
        "ping" | "beacon" => T_PING,
        "other" => T_OTHER,
        "subdocument" | "frame" => T_FRAME,
        "popup" => T_POPUP,
        "document" | "doc" => T_DOC,
        _ => return None,
    })
}

/// Besteht der Adressteil nur aus einem Rechnernamen (`||beispiel.de^`)?
fn host_only(pattern: &str) -> bool {
    let Some(rest) = pattern.strip_prefix("||") else {
        return false;
    };
    let host = rest.strip_suffix('^').unwrap_or(rest);
    host.contains('.') && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Eine Zeile, die nur aus einem Rechnernamen oder einer IP-Adresse besteht –
/// so führen Listen gemeldeter Betrugsseiten ganze Rechner.
fn host_line(line: &str) -> bool {
    line.len() <= 253
        && line.contains('.')
        && !line.starts_with(['.', '-'])
        && !line.ends_with(['.', '-'])
        && line.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
}

/// Liest eine Sperr- oder Ausnahmeregel. `None`: nicht genau ausdrückbar.
///
/// `guard`: Die Zeile stammt aus einer Liste gemeldeter Betrugs- und
/// Schadseiten. Dort heißt ein bloßer Rechnername ohne weitere Angabe: der
/// ganze Rechner, auch als Seite – nicht nur, was andere Seiten von ihm nachladen.
fn parse_net(line: &str, guard: bool) -> Option<Net> {
    let (exception, body) = match line.strip_prefix("@@") {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    let (pattern, options) = split_options(body);
    let mut plus: u16 = 0;
    let mut minus: u16 = 0;
    let mut party = Party::Any;
    let mut case = false;
    let mut domains: Option<Domains> = None;
    let mut unhide = Unhide::No;
    let mut whole_page = false;
    // Typen, die es in WebKit nicht gibt (Plug-in-Inhalte): Eine Regel nur dafür entfällt.
    let mut unknown_type = false;
    // `$genericblock` lässt sich nicht ausdrücken; steht es allein, entfällt die Regel.
    let mut generic_block = false;
    for item in options.split(',').filter(|item| !item.is_empty()) {
        let (negated, item) = match item.strip_prefix('~') {
            Some(rest) => (true, rest),
            None => (false, item),
        };
        let (name, value) = match item.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (item, None),
        };
        let name = name.to_ascii_lowercase();
        match (name.as_str(), value) {
            ("third-party" | "3p", None) => party = if negated { Party::First } else { Party::Third },
            ("first-party" | "1p", None) => party = if negated { Party::Third } else { Party::First },
            ("match-case", None) => case = !negated,
            ("domain" | "from", Some(value)) if !negated => domains = Some(split_domains(value, '|')),
            // Vorrang-Angabe eines anderen Blockers: ändert nichts daran, was gesperrt wird.
            ("important", None) if !exception && !negated => {}
            ("generichide" | "ghide", None) if exception && !negated => unhide = Unhide::Generic,
            ("elemhide" | "ehide", None) if exception && !negated => unhide = Unhide::All,
            ("genericblock", None) if exception && !negated => generic_block = true,
            // Schaltet Regeln ab, die es hier ohnehin nicht gibt.
            ("csp", None) if exception && !negated => {}
            ("document" | "doc", None) if exception && !negated => whole_page = true,
            ("object" | "object-subrequest", None) => unknown_type |= !negated,
            ("all", None) if !negated => plus |= T_ALL,
            (name, None) => match type_bit(name) {
                Some(bit) if negated => minus |= bit,
                Some(bit) => plus |= bit,
                // Umleitungen, eingeschleuste Kopfzeilen, Skripte …: bewusst nicht.
                None => return None,
            },
            _ => return None,
        }
    }
    let types = if plus != 0 {
        plus & !minus
    } else if unknown_type && minus == 0 {
        0
    } else if guard && !exception && minus == 0 && host_only(pattern) {
        T_ALL
    } else {
        T_DEFAULT & !minus
    };
    let only_special = unhide != Unhide::No || whole_page || generic_block;
    if types == 0 && !only_special {
        return None;
    }
    if generic_block && unhide == Unhide::No && !whole_page {
        return None;
    }
    let found = url_filters(pattern)?;

    let (mut include, mut exclude) = (Vec::new(), Vec::new());
    if let Some(domains) = domains {
        if exception {
            // Eine Ausnahme darf weiter werden, nie enger als gedacht – außer es
            // bliebe gar keine Website übrig.
            if domains.include.is_empty() && domains.lost_include {
                return None;
            }
            if domains.include.is_empty() {
                exclude = domains.exclude;
            } else {
                include = domains.include.iter().map(|d| format!("*{d}")).collect();
            }
        } else {
            // Eine Sperre darf enger werden, nie weiter.
            if domains.lost_exclude || (domains.include.is_empty() && domains.lost_include) {
                return None;
            }
            if domains.include.is_empty() {
                exclude = domains.exclude;
            } else {
                include = narrowed(&domains.include, &domains.exclude);
            }
        }
    }
    // Alles zu sperren – oder für alles eine Ausnahme zu machen –, ohne es auf
    // einzelne Websites zu beschränken, steht in keiner brauchbaren Liste.
    if found.universal && include.is_empty() {
        return None;
    }
    Some(Net {
        exception,
        filters: found.list.into_iter().filter(|f| filter_is_safe(f)).collect(),
        universal: found.universal,
        case,
        // Bei einer Ausnahme für ganze Seiten bleiben nur die ausdrücklich genannten Typen.
        types: if only_special { plus & !minus & !T_DOC } else { types },
        party,
        include,
        exclude: exclude.iter().map(|d| format!("*{d}")).collect(),
        unhide,
        whole_page,
    })
    .filter(|net| !net.filters.is_empty())
}

/// „Gilt auf a.de, aber nicht auf x.a.de“: WebKit kann nur das eine ODER das
/// andere. Dann zählt a.de ohne seine Unterseiten – enger, aber nie falsch.
fn narrowed(include: &[String], exclude: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(include.len());
    for domain in include {
        if exclude.iter().any(|e| e == domain) {
            continue;
        }
        let has_excluded_part = exclude.iter().any(|e| e.ends_with(&format!(".{domain}")));
        if !has_excluded_part {
            out.push(format!("*{domain}"));
            continue;
        }
        out.push(domain.clone());
        let www = format!("www.{domain}");
        if !domain.starts_with("www.") && !exclude.contains(&www) {
            out.push(www);
        }
    }
    out
}

enum Cosmetic<'a> {
    Hide { domains: &'a str, selector: &'a str },
    Unhide { domains: &'a str, selector: &'a str },
    /// Skripte, erweiterte Auswahl-Sprachen und Ähnliches.
    Unsupported,
}

/// Erkennt Ausblend-Regeln (`beispiel.de##.werbung`).
fn cosmetic(line: &str) -> Option<Cosmetic<'_>> {
    for (index, _) in line.match_indices('#') {
        let domains = &line[..index];
        if domains.contains(['/', '|', '@', '"', '!']) {
            return None;
        }
        let rest = &line[index + 1..];
        if let Some(selector) = rest.strip_prefix('#') {
            return Some(match selector.is_empty() {
                true => Cosmetic::Unsupported,
                false => Cosmetic::Hide { domains, selector },
            });
        }
        if let Some(selector) = rest.strip_prefix("@#") {
            return Some(match selector.is_empty() {
                true => Cosmetic::Unsupported,
                false => Cosmetic::Unhide { domains, selector },
            });
        }
        if rest.starts_with("?#") || rest.starts_with("$#") || rest.starts_with("%#") || rest.starts_with("@") {
            return Some(Cosmetic::Unsupported);
        }
    }
    None
}

/// Nimmt nur schlichte Auswahlregeln an. Was WebKit nicht als CSS versteht,
/// lässt es selbst weg – hier fällt heraus, was etwas anderes TUN würde als
/// ausblenden (Skripte, eigene Stile) oder die ganze Seite träfe.
fn selector_ok(selector: &str) -> bool {
    const FOREIGN: &[&str] = &[
        ":-abp-",
        ":has-text(",
        ":contains(",
        ":xpath(",
        ":matches-",
        ":min-text-length(",
        ":upward(",
        ":remove(",
        ":style(",
        ":watch-attr(",
        ":nth-ancestor(",
        ":others(",
        ":if(",
        ":if-not(",
    ];
    let trimmed = selector.trim();
    if trimmed.is_empty() || trimmed.len() > 500 {
        return false;
    }
    if trimmed.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7f || matches!(c, '{' | '}')) {
        return false;
    }
    if trimmed.starts_with("+js(") || trimmed.starts_with('^') {
        return false;
    }
    if FOREIGN.iter().any(|foreign| trimmed.contains(foreign)) {
        return false;
    }
    !matches!(trimmed.to_ascii_lowercase().as_str(), "*" | "html" | "body" | "head" | ":root" | "html, body" | "html,body")
}

// ---------------------------------------------------------------------------
// Regeln schreiben

fn push_str_json(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c as u32 == 0x2028 || c as u32 == 0x2029 || c as u32 == 0x7f => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_list_json(out: &mut String, key: &str, items: &[String]) {
    let _ = write!(out, ",\"{key}\":[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_str_json(out, item);
    }
    out.push(']');
}

/// Wie WebKit die Typen nennt. `compat`: ältere Fassungen kennen die getrennten
/// Namen für Seite und eingebettete Rahmen nicht – dort entsteht dafür eine
/// zweite Regel (siehe `push_net`).
fn type_names(types: u16, compat: bool) -> Vec<&'static str> {
    let mut names: Vec<&'static str> =
        TYPE_NAMES.iter().filter(|(bit, _)| types & bit != 0).map(|(_, name)| *name).collect();
    if !compat {
        if types & T_FRAME != 0 {
            names.push("child-document");
        }
        if types & T_DOC != 0 {
            names.push("top-document");
        }
    }
    names
}

struct Trigger<'a> {
    filter: &'a str,
    case: bool,
    party: Party,
    include: &'a [String],
    exclude: &'a [String],
}

fn push_rule(out: &mut Vec<String>, trigger: &Trigger, types: &[&str], context: Option<&str>, action: &str) {
    let mut rule = String::with_capacity(160);
    rule.push_str("{\"trigger\":{\"url-filter\":");
    push_str_json(&mut rule, trigger.filter);
    if trigger.case {
        rule.push_str(",\"url-filter-is-case-sensitive\":true");
    }
    if !types.is_empty() {
        rule.push_str(",\"resource-type\":[");
        for (index, name) in types.iter().enumerate() {
            if index > 0 {
                rule.push(',');
            }
            let _ = write!(rule, "\"{name}\"");
        }
        rule.push(']');
    }
    if let Some(context) = context {
        let _ = write!(rule, ",\"load-context\":[\"{context}\"]");
    }
    match trigger.party {
        Party::Third => rule.push_str(",\"load-type\":[\"third-party\"]"),
        Party::First => rule.push_str(",\"load-type\":[\"first-party\"]"),
        Party::Any => {}
    }
    if !trigger.include.is_empty() {
        push_list_json(&mut rule, "if-domain", trigger.include);
    } else if !trigger.exclude.is_empty() {
        push_list_json(&mut rule, "unless-domain", trigger.exclude);
    }
    let _ = write!(rule, "}},\"action\":{{\"type\":\"{action}\"}}}}");
    out.push(rule);
}

/// Schreibt eine Sperre oder gewöhnliche Ausnahme für die genannten Typen.
fn push_net(out: &mut Vec<String>, net: &Net, types: u16, action: &str, compat: bool) {
    if types == 0 {
        return;
    }
    for filter in &net.filters {
        let trigger =
            Trigger { filter, case: net.case, party: net.party, include: &net.include, exclude: &net.exclude };
        let names = type_names(types, compat);
        if !names.is_empty() {
            push_rule(out, &trigger, &names, None, action);
        }
        if compat {
            match (types & T_FRAME != 0, types & T_DOC != 0) {
                (true, true) => push_rule(out, &trigger, &["document"], None, action),
                (true, false) => push_rule(out, &trigger, &["document"], Some("child-frame"), action),
                (false, true) => push_rule(out, &trigger, &["document"], Some("top-frame"), action),
                (false, false) => {}
            }
        }
    }
}

/// „Auf Seiten mit dieser Adresse gilt keine der Regeln davor.“ Gebraucht für
/// Ausnahmen, die ganze Seiten freistellen (`$document`), und für solche, die
/// dort nur das Ausblenden abschalten (`$generichide`, `$elemhide`) – die
/// stehen in der Ausblend-Liste an der passenden Stelle.
///
/// Maßgeblich ist die Adresse der SEITE, nicht die des einzelnen Abrufs: WebKit
/// wendet Ausblend-Regeln bei jedem Abruf einer Seite neu an (Bilder, Skripte
/// …), eine Ausnahme nur für den Abruf der Seite selbst bliebe wirkungslos.
fn push_page_exception(out: &mut Vec<String>, net: &Net) {
    if net.universal {
        // Ohne Adressmuster: die genannten Websites als Ganzes.
        let mut rule = String::from("{\"trigger\":{\"url-filter\":\".*\"");
        push_list_json(&mut rule, "if-domain", &net.include);
        rule.push_str("},\"action\":{\"type\":\"ignore-previous-rules\"}}");
        out.push(rule);
        return;
    }
    // Adressmuster UND Website zugleich kann WebKit hier nicht prüfen. Dann
    // zählt das Muster allein – eine Ausnahme darf weiter werden.
    for filter in &net.filters {
        let mut rule = String::from("{\"trigger\":{\"url-filter\":\".*\",\"if-top-url\":[");
        // Das Muster soll die ganze Adresse der Seite abdecken dürfen.
        let whole = if filter.ends_with('$') { filter.clone() } else { format!("{filter}.*") };
        push_str_json(&mut rule, &whole);
        rule.push(']');
        if net.case {
            rule.push_str(",\"top-url-filter-is-case-sensitive\":true");
        }
        rule.push_str("},\"action\":{\"type\":\"ignore-previous-rules\"}}");
        out.push(rule);
    }
}

fn push_hide(out: &mut Vec<String>, selector: &str, include: &[String], exclude: &[String]) {
    let mut rule = String::with_capacity(120 + selector.len());
    rule.push_str("{\"trigger\":{\"url-filter\":\".*\"");
    if !include.is_empty() {
        push_list_json(&mut rule, "if-domain", include);
    } else if !exclude.is_empty() {
        push_list_json(&mut rule, "unless-domain", exclude);
    }
    rule.push_str("},\"action\":{\"type\":\"css-display-none\",\"selector\":");
    push_str_json(&mut rule, selector);
    rule.push_str("}}");
    out.push(rule);
}

fn join(rules: &[&[String]]) -> String {
    let size: usize = rules.iter().flat_map(|part| part.iter()).map(|rule| rule.len() + 1).sum();
    let mut out = String::with_capacity(size + 2);
    out.push('[');
    for rule in rules.iter().flat_map(|part| part.iter()) {
        if out.len() > 1 {
            out.push(',');
        }
        out.push_str(rule);
    }
    out.push(']');
    out
}

// ---------------------------------------------------------------------------

/// Verzeichnis der gemeldeten Betrugs- und Schadseiten, soweit ganze Seiten
/// gemeint sind – damit der Browser selbst vor dem Öffnen warnen kann.
#[derive(Default, Debug)]
pub struct GuardIndex {
    /// Ganze Rechner (gilt auch für ihre Unterrechner).
    hosts: HashSet<String>,
    /// Einzelne Adressen: Rechner → Anfänge von Pfad und Abfrage.
    paths: HashMap<String, Vec<String>>,
}

impl GuardIndex {
    pub fn len(&self) -> usize {
        self.hosts.len() + self.paths.values().map(Vec::len).sum::<usize>()
    }

    /// Steht diese Adresse als ganze Seite auf einer der Listen?
    #[cfg(test)]
    pub fn matches(&self, url: &str) -> bool {
        self.matched(url).is_some()
    }

    /// Trifft ein Eintrag diese Adresse? Zurück kommt, was jemand übergeht,
    /// der die Warnung dazu übergeht: `h:<rechner>` – der Rechner der Adresse
    /// selbst, auch wenn die Liste einen Namen darüber meldet – oder
    /// `p:<rechner><pfad>` für eine einzelne gemeldete Adresse. Es ist immer
    /// das Engste: nicht die Nachbarn unter demselben gemeldeten Namen und
    /// nicht, was sonst noch über denselben Rechner gemeldet ist.
    pub fn matched(&self, url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let host = guard_host(&parsed)?;
        // Der Rechner selbst oder einer, unter dem er hängt.
        let mut rest = host.as_str();
        loop {
            if self.hosts.contains(rest) {
                return Some(format!("h:{host}"));
            }
            match rest.split_once('.') {
                Some((_, parent)) if parent.contains('.') => rest = parent,
                _ => break,
            }
        }
        let prefixes = self.paths.get(&host)?;
        let mut tail = parsed.path().to_string();
        if let Some(query) = parsed.query() {
            tail.push('?');
            tail.push_str(query);
        }
        let tail = plain_escapes(&tail);
        prefixes
            .iter()
            .find(|prefix| {
                tail.strip_prefix(prefix.as_str()).is_some_and(|after| {
                    // Danach muss die Adresse enden oder ein Trennzeichen folgen.
                    after.chars().next().is_none_or(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '%')))
                })
            })
            .map(|prefix| format!("p:{host}{prefix}"))
    }
}

/// Der Rechnername, wie er in den Listen steht: klein, ohne den Punkt am Ende
/// (`beispiel.de.` ist derselbe Rechner), und eine IPv4-Adresse im IPv6-Kleid
/// (`[::ffff:1.2.3.4]`) als die IPv4-Adresse, die sie ist.
pub fn guard_host(url: &url::Url) -> Option<String> {
    match url.host()? {
        url::Host::Domain(name) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            (!name.is_empty()).then_some(name)
        }
        url::Host::Ipv4(address) => Some(address.to_string()),
        url::Host::Ipv6(address) => Some(match address.to_ipv4_mapped() {
            Some(inner) => inner.to_string(),
            None => format!("[{address}]"),
        }),
    }
}

/// Gleicht aus, was sich in einer Adresse verschieden schreiben lässt, ohne
/// dass es etwas anderes bedeutet: `%70` ist ein `p`. Verschlüsselt geschrieben
/// bleiben nur Zeichen, die es sein müssen – die dann mit Großbuchstaben.
fn plain_escapes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if let (b'%', Some(high), Some(low)) = (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            let value = (high * 16 + low) as u8;
            if value.is_ascii_alphanumeric() || matches!(value, b'-' | b'.' | b'_' | b'~') {
                out.push(value as char);
            } else {
                out.push_str(&format!("%{value:02X}"));
            }
            i += 3;
        } else {
            // Die Stelle ist ein ganzes Zeichen oder dessen Anfang – bis zur nächsten Zeichengrenze übernehmen.
            let mut end = i + 1;
            while !text.is_char_boundary(end) {
                end += 1;
            }
            out.push_str(&text[i..end]);
            i = end;
        }
    }
    out
}

/// Steht diese Domain – oder eine, unter der sie hängt – in der Menge?
fn covered(sites: &HashSet<String>, domain: &str) -> bool {
    let mut rest = domain;
    loop {
        if sites.contains(rest) {
            return true;
        }
        match rest.split_once('.') {
            Some((_, parent)) => rest = parent,
            None => return false,
        }
    }
}

/// Baut das Verzeichnis aus den Listen. Aufgenommen wird nur, was eindeutig
/// eine ganze Seite meint: bloße Rechnernamen sowie Adressen mit `$all` oder
/// `$doc` – ohne Platzhalter und ohne weitere Bedingungen.
pub fn guard_index(texts: &[&str]) -> GuardIndex {
    let mut index = GuardIndex::default();
    for text in texts {
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('!') || line.starts_with('[') || line.starts_with('@') || line.len() > 2000 {
                continue;
            }
            if host_line(line) {
                index.hosts.insert(line.to_string());
                continue;
            }
            let Some(rest) = line.strip_prefix("||") else {
                continue;
            };
            let (pattern, options) = split_options(rest);
            let whole_page = options.split(',').all(|option| matches!(option, "all" | "doc" | "document"));
            if !options.is_empty() && !whole_page {
                continue;
            }
            let pattern = pattern.strip_suffix('^').unwrap_or(pattern);
            if pattern.contains(['*', '^', '|']) || !pattern.is_ascii() {
                continue;
            }
            match pattern.split_once('/') {
                None if options.is_empty() || whole_page => {
                    if host_line(&pattern.to_ascii_lowercase()) {
                        index.hosts.insert(pattern.to_ascii_lowercase());
                    }
                }
                // Eine einzelne Adresse zählt nur, wenn die Liste ausdrücklich die Seite meint.
                Some((host, path)) if whole_page && host_line(&host.to_ascii_lowercase()) => {
                    let entry = index.paths.entry(host.to_ascii_lowercase()).or_default();
                    if entry.len() < 2000 {
                        entry.push(plain_escapes(&format!("/{path}")));
                    }
                }
                _ => {}
            }
        }
    }
    index
}

/// Übersetzt Listen gemeldeter Betrugs- und Schadseiten: nur Sperren und
/// ihre Ausnahmen, nichts wird ausgeblendet.
pub fn convert_guard(texts: &[&str], compat: bool) -> Converted {
    let mut stats = Stats::default();
    let mut blocks: Vec<String> = Vec::new();
    let mut exceptions: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for text in texts {
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('!') || line.starts_with('[') || line.starts_with('#') || line.len() > 2000 {
                continue;
            }
            if !seen.insert(line) {
                continue;
            }
            let rule = if host_line(line) {
                parse_net(&format!("||{line}^"), true)
            } else if cosmetic(line).is_some() {
                None
            } else {
                parse_net(line, true)
            };
            // Ausnahmen für ganze Seiten oder fürs Ausblenden gibt es hier nicht.
            match rule.filter(|net| net.unhide == Unhide::No && !net.whole_page) {
                None => stats.skipped += 1,
                Some(net) if net.exception => {
                    let before = exceptions.len();
                    push_net(&mut exceptions, &net, net.types, "ignore-previous-rules", compat);
                    stats.exceptions += exceptions.len() - before;
                }
                Some(net) => {
                    let before = blocks.len();
                    push_net(&mut blocks, &net, net.types, "block", compat);
                    stats.blocking += blocks.len() - before;
                }
            }
        }
    }
    let lists = blocks.chunks(CHUNK).map(|part| join(&[part, &exceptions])).collect();
    Converted { lists, stats }
}

/// Übersetzt die Listen. Die erste Ergebnisliste blendet aus, die übrigen sperren.
pub fn convert(texts: &[&str], compat: bool) -> Converted {
    let mut stats = Stats::default();
    let mut blocks: Vec<String> = Vec::new();
    let mut exceptions: Vec<String> = Vec::new();
    let mut whole_pages: Vec<String> = Vec::new();
    let mut unhide_generic: Vec<String> = Vec::new();
    let mut unhide_all: Vec<String> = Vec::new();
    // Ausblenden: allgemein (überall) und je Website.
    let mut generic: Vec<(String, Vec<String>)> = Vec::new();
    let mut specific: Vec<(Vec<String>, String)> = Vec::new();
    // Auswahlregel → Websites, auf denen sie NICHT gelten soll (leer: nirgends).
    let mut spared: HashMap<String, HashSet<String>> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();

    for text in texts {
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('!') || line.starts_with('[') || line.len() > 8000 {
                continue;
            }
            if !seen.insert(line.to_string()) {
                continue;
            }
            match cosmetic(line) {
                Some(Cosmetic::Hide { domains, selector }) => {
                    if !selector_ok(selector) {
                        stats.skipped += 1;
                        continue;
                    }
                    let selector = selector.trim().to_string();
                    let domains = split_domains(domains, ',');
                    if domains.include.is_empty() {
                        if domains.lost_include || domains.lost_exclude {
                            stats.skipped += 1;
                        } else {
                            generic.push((selector, domains.exclude));
                        }
                    } else if domains.lost_exclude {
                        stats.skipped += 1;
                    } else {
                        specific.push((narrowed(&domains.include, &domains.exclude), selector));
                    }
                }
                Some(Cosmetic::Unhide { domains, selector }) => {
                    let domains = split_domains(domains, ',');
                    let entry = spared.entry(selector.trim().to_string()).or_default();
                    if domains.include.is_empty() && !domains.lost_include {
                        entry.clear();
                        entry.insert(String::new());
                    } else if !entry.contains("") {
                        // Mehr Ausnahmen je Regel führt keine echte Liste – eine
                        // manipulierte soll den Übersetzer damit nicht aufhalten.
                        let room = MAX_SPARED.saturating_sub(entry.len());
                        entry.extend(domains.include.into_iter().take(room));
                    }
                }
                Some(Cosmetic::Unsupported) => stats.skipped += 1,
                None => match parse_net(line, false) {
                    None => stats.skipped += 1,
                    Some(net) if net.exception => {
                        let before = exceptions.len() + whole_pages.len() + unhide_generic.len() + unhide_all.len();
                        match net.unhide {
                            Unhide::Generic => push_page_exception(&mut unhide_generic, &net),
                            Unhide::All => push_page_exception(&mut unhide_all, &net),
                            Unhide::No => {
                                // Eine ganze Seite freistellen: nur ohne Website-Angabe –
                                // sonst bleibt es bei den ausdrücklich genannten Typen.
                                if net.whole_page && (net.universal || (net.include.is_empty() && net.exclude.is_empty())) {
                                    push_page_exception(&mut whole_pages, &net);
                                }
                                if !net.universal || !net.include.is_empty() {
                                    push_net(&mut exceptions, &net, net.types, "ignore-previous-rules", compat);
                                }
                            }
                        }
                        let after = exceptions.len() + whole_pages.len() + unhide_generic.len() + unhide_all.len();
                        match after > before {
                            true => stats.exceptions += after - before,
                            false => stats.skipped += 1,
                        }
                    }
                    Some(net) => {
                        let before = blocks.len();
                        push_net(&mut blocks, &net, net.types, "block", compat);
                        match blocks.len() > before {
                            true => stats.blocking += blocks.len() - before,
                            false => stats.skipped += 1,
                        }
                    }
                },
            }
        }
    }

    // Ausblenden: erst allgemein, dann die Ausnahmen davon, dann je Website.
    let mut hide_open: Vec<String> = Vec::new();
    let mut hide_limited: Vec<String> = Vec::new();
    let mut hide_specific: Vec<String> = Vec::new();
    let mut seen_generic: HashSet<&str> = HashSet::new();
    for (selector, exclude) in &generic {
        if !seen_generic.insert(selector.as_str()) {
            continue;
        }
        let mut exclude: Vec<String> = exclude.clone();
        if let Some(sites) = spared.get(selector) {
            if sites.contains("") {
                continue;
            }
            exclude.extend(sites.iter().cloned());
        }
        exclude.sort();
        exclude.dedup();
        if exclude.is_empty() {
            push_hide(&mut hide_open, selector, &[], &[]);
        } else {
            let exclude: Vec<String> = exclude.iter().map(|d| format!("*{d}")).collect();
            push_hide(&mut hide_limited, selector, &[], &exclude);
        }
    }
    for (include, selector) in &specific {
        let include: Vec<String> = match spared.get(selector) {
            None => include.clone(),
            Some(sites) if sites.contains("") => Vec::new(),
            Some(sites) => {
                // Alles, worunter eine ausgenommene Website hängt (`x.a.de` → `a.de`, `de`).
                let above: HashSet<&str> = sites
                    .iter()
                    .flat_map(|site| site.match_indices('.').map(move |(dot, _)| &site[dot + 1..]))
                    .collect();
                include
                    .iter()
                    .filter(|entry| !covered(sites, entry.trim_start_matches('*')))
                    .map(|entry| {
                        // Eine Ausnahme für x.a.de: Dann gilt die Regel nur auf a.de selbst.
                        let domain = entry.trim_start_matches('*');
                        match entry.starts_with('*') && above.contains(domain) {
                            true => domain.to_string(),
                            false => entry.clone(),
                        }
                    })
                    .collect()
            }
        };
        if !include.is_empty() {
            push_hide(&mut hide_specific, selector, &include, &[]);
        }
    }
    stats.hiding = hide_open.len() + hide_limited.len() + hide_specific.len();

    // WebKit nimmt je Liste höchstens 150.000 Regeln. Die Ausnahmen stehen in
    // jedem Teil noch einmal – wären es so viele, dass ein Teil die Grenze
    // reißt, ist das keine der Listen, für die dieser Übersetzer gemacht ist.
    // Dann wird gar nichts übernommen, und es bleibt bei den Listen von vorher.
    let hide_rules = stats.hiding + unhide_generic.len() + unhide_all.len() + whole_pages.len();
    if exceptions.len() + whole_pages.len() > MAX_EXCEPTIONS || hide_rules > MAX_LIST_RULES {
        return Converted { lists: Vec::new(), stats };
    }

    let mut lists = Vec::new();
    if stats.hiding > 0 {
        lists.push(join(&[&hide_open, &hide_limited, &unhide_generic, &hide_specific, &unhide_all, &whole_pages]));
    }
    for part in blocks.chunks(CHUNK) {
        lists.push(join(&[part, &exceptions, &whole_pages]));
    }
    Converted { lists, stats }
}

/// Der Stand einer Liste, wie ihn die Kopfzeile nennt – als Ziffernfolge, die
/// sich vergleichen lässt (`! Version: 202611220546`, `! Updated:
/// 2026-11-22T00:03:04Z`, `! Last modified: Thu, 19 Nov 2026 19:00:23 +0000`).
pub fn stamp(text: &str) -> Option<String> {
    stamp_before(text, &day_digits(2))
}

/// Das Datum in `ahead` Tagen als acht Ziffern (`20261124`), nach der Uhr des Macs.
fn day_digits(ahead: u64) -> String {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Tage seit 1970 in Jahr, Monat, Tag (bürgerlicher Kalender).
    let z = (seconds / 86_400 + ahead) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}")
}

/// Wie `stamp`, mit dem spätesten Datum, das noch gilt (acht Ziffern). Ein
/// Stand „aus der Zukunft“ wird nicht geglaubt: Eine einzige untergeschobene
/// Liste mit einem Fantasie-Datum gälte sonst für immer als die neueste –
/// neuer als jede echte, die danach käme.
fn stamp_before(text: &str, latest_day: &str) -> Option<String> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let plausible = |value: &str| {
        (8..=14).contains(&value.len())
            && value.bytes().all(|b| b.is_ascii_digit())
            && &value[..8] >= "20150101"
            && &value[..8] <= latest_day
    };
    let head: Vec<&str> = text.lines().take(40).collect();
    let field = |name: &str| head.iter().find_map(|line| line.strip_prefix(name).map(str::trim));
    if let Some(value) = field("! Version:").filter(|value| plausible(value)) {
        return Some(value.to_string());
    }
    if let Some(value) = field("! Updated:") {
        let digits: String = value.chars().filter(char::is_ascii_digit).take(14).collect();
        if value.len() >= 10 && value.as_bytes()[4] == b'-' && plausible(&digits) {
            return Some(digits);
        }
    }
    // „Thu, 19 Nov 2026 19:00:23 +0000“
    let value = field("! Last modified:")?;
    let parts: Vec<&str> = value.split_whitespace().collect();
    let [_, day, month, year, time, ..] = parts.as_slice() else {
        return None;
    };
    let month = MONTHS.iter().position(|name| month.to_ascii_lowercase().starts_with(name))? + 1;
    let day: u32 = day.parse().ok()?;
    let clock: String = time.chars().filter(char::is_ascii_digit).take(6).collect();
    let digits = format!("{year}{month:02}{day:02}{clock}");
    plausible(&digits).then_some(digits)
}

/// Sieht der Text nach einer vollständigen Filterliste aus? Abgeschnittene
/// oder fremde Antworten (Fehlerseiten, Anmeldeseiten) werden nie übernommen.
pub fn plausible(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.len() >= 20_000
        && text.len() <= 12_000_000
        && (text.starts_with("[Adblock") || text.starts_with("! Title:"))
        && stamp(text).is_some()
        && text.lines().filter(|line| !line.starts_with('!')).take(1000).count() == 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(line: &str) -> Vec<serde_json::Value> {
        let converted = convert(&[line], false);
        converted.lists.iter().flat_map(|list| serde_json::from_str::<Vec<serde_json::Value>>(list).unwrap()).collect()
    }

    #[test]
    fn domain_rules_spare_the_page_itself_and_new_windows() {
        let rules = only("||werbung.example^");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["trigger"]["url-filter"], r"^[htpsw]+://([^:/]+\.)?werbung\.example\.?[:/]");
        let types: Vec<&str> =
            rules[0]["trigger"]["resource-type"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        assert!(types.contains(&"script") && types.contains(&"child-document"));
        assert!(!types.contains(&"top-document") && !types.contains(&"popup") && !types.contains(&"document"));
        assert_eq!(rules[0]["action"]["type"], "block");
        assert!(rules[0]["trigger"].get("load-type").is_none());
    }

    #[test]
    fn options_become_webkit_triggers() {
        let rules = only("/banner/*/img^$third-party,image,domain=a.example|b.example");
        // Trennzeichen am Ende: einmal mit Zeichen, einmal am Adressende.
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["trigger"]["url-filter"], "/banner/.*/img[^a-zA-Z0-9_.%-]");
        assert_eq!(rules[1]["trigger"]["url-filter"], "/banner/.*/img$");
        assert_eq!(rules[0]["trigger"]["load-type"][0], "third-party");
        assert_eq!(rules[0]["trigger"]["resource-type"], serde_json::json!(["image", "svg-document"]));
        assert_eq!(rules[0]["trigger"]["if-domain"], serde_json::json!(["*a.example", "*b.example"]));

        let rules = only("||x.example/pfad$~script,match-case");
        assert_eq!(rules[0]["trigger"]["url-filter-is-case-sensitive"], true);
        let types = rules[0]["trigger"]["resource-type"].as_array().unwrap();
        assert!(!types.iter().any(|t| t == "script") && types.iter().any(|t| t == "image"));

        let rules = only("||pop.example^$popup");
        assert_eq!(rules[0]["trigger"]["resource-type"], serde_json::json!(["popup"]));
        let rules = only("|https://genau.example/x|");
        assert_eq!(rules[0]["trigger"]["url-filter"], r"^https://genau\.example/x$");
    }

    #[test]
    fn a_block_only_gets_narrower() {
        // „Auf a.example, aber nicht auf shop.a.example“ → nur a.example selbst.
        let rules = only("/ads.js$domain=a.example|~shop.a.example");
        assert_eq!(rules[0]["trigger"]["if-domain"], serde_json::json!(["a.example", "www.a.example"]));
        // Nicht ausdrückbare Website-Angaben: lieber keine Regel als eine für alle.
        assert!(only("/ads.js$domain=a.*").is_empty());
        assert!(only("/ads.js$domain=~a.*").is_empty());
        // Nur Ausschlüsse.
        let rules = only("/ads.js$domain=~a.example");
        assert_eq!(rules[0]["trigger"]["unless-domain"], serde_json::json!(["*a.example"]));
    }

    #[test]
    fn what_cannot_be_expressed_is_left_out() {
        for line in [
            "/ad[0-9]+/",                                      // eigener regulärer Ausdruck
            "||a.example^$redirect=noop.js,script",            // Umleitung
            "||a.example^$csp=script-src 'self'",              // eingeschleuste Kopfzeile
            "||a.example^$rewrite=abp-resource:blank-js",
            "||a.example^$removeparam=utm",
            "||a.example^$object",                             // Typ, den es nicht gibt
            "||a.example^$unbekannt",
            "$script",                                         // alles, überall
            "*$third-party",
            "ad",                                              // zwei Zeichen irgendwo
            "||bücher.example^",                               // fremde Zeichen
            "a.example#?#div:-abp-has(.x)",
            "a.example#$#abort-on-property-read x",
            "a.example#%#//scriptlet('x')",
            "a.example##+js(nowoif)",
            "##div:has-text(Anzeige)",
            "##body",
            "a.*##.x",
            "@@||a.example^$genericblock",
            "@@||a.example^$sitekey=abc",
        ] {
            let converted = convert(&[line], false);
            assert!(converted.lists.is_empty(), "{line}: {:?}", converted.lists);
            assert_eq!(converted.stats.skipped, 1, "{line}");
        }
    }

    #[test]
    fn hiding_rules_and_their_exceptions() {
        let text = "##.werbung\n##.banner\n##.tot\na.example,b.example##.eigen\nc.example#@#.banner\n#@#.tot\nb.example#@#.eigen\n~d.example##.fast-ueberall\n@@||e.example^$generichide\n@@||f.example^$elemhide\n@@||g.example^$document\n||sperre.example^\n@@||sperre.example/gut.js$script";
        let converted = convert(&[text], false);
        assert_eq!(converted.lists.len(), 2);
        let hide: Vec<serde_json::Value> = serde_json::from_str(&converted.lists[0]).unwrap();
        let describe = |rule: &serde_json::Value| {
            format!(
                "{} {} {}",
                rule["action"]["type"].as_str().unwrap(),
                rule["action"]["selector"].as_str().unwrap_or("-"),
                rule["trigger"]
            )
        };
        let lines: Vec<String> = hide.iter().map(describe).collect();
        // Reihenfolge: überall → überall mit Ausschluss → Ausnahme „allgemein“ →
        // je Website → Ausnahme „alles“ → ganze Seiten.
        assert_eq!(lines.len(), 7, "{lines:#?}");
        assert!(lines[0].starts_with("css-display-none .werbung") && !lines[0].contains("domain"));
        assert!(lines[1].starts_with("css-display-none .banner") && lines[1].contains(r#""unless-domain":["*c.example"]"#));
        assert!(lines[2].starts_with("css-display-none .fast-ueberall") && lines[2].contains("*d.example"));
        assert!(lines[3].starts_with("ignore-previous-rules") && lines[3].contains("e\\\\.example"));
        // Maßgeblich ist die Adresse der Seite – sonst griffe die Ausnahme nicht.
        assert!(lines[3].contains("if-top-url") && !lines[3].contains("resource-type"));
        assert!(lines[4].starts_with("css-display-none .eigen") && lines[4].contains(r#""if-domain":["*a.example"]"#));
        assert!(lines[5].starts_with("ignore-previous-rules") && lines[5].contains("f\\\\.example"));
        assert!(lines[6].contains("if-top-url") && lines[6].contains("g\\\\.example"));
        assert!(!lines.iter().any(|l| l.contains(".tot")));

        // Sperren: erst die Sperre, dann die Ausnahmen – auch die für ganze Seiten.
        let net: Vec<serde_json::Value> = serde_json::from_str(&converted.lists[1]).unwrap();
        assert_eq!(net.len(), 3);
        assert_eq!(net[0]["action"]["type"], "block");
        assert_eq!(net[1]["action"]["type"], "ignore-previous-rules");
        assert_eq!(net[1]["trigger"]["resource-type"], serde_json::json!(["script"]));
        assert!(net[2]["trigger"]["if-top-url"][0].as_str().unwrap().ends_with("g\\.example\\.?[:/].*"));
        assert_eq!(converted.stats, Stats { blocking: 1, exceptions: 4, hiding: 4, skipped: 0 });
    }

    #[test]
    fn older_webkit_gets_two_rules_for_frames() {
        let converted = convert(&["||a.example^\n||b.example^$subdocument\n@@||c.example^$generichide\n##.x"], true);
        let all: Vec<serde_json::Value> =
            converted.lists.iter().flat_map(|l| serde_json::from_str::<Vec<serde_json::Value>>(l).unwrap()).collect();
        let text = serde_json::Value::Array(all.clone()).to_string();
        assert!(!text.contains("child-document") && !text.contains("top-document"));
        let frames: Vec<&serde_json::Value> =
            all.iter().filter(|r| r["trigger"]["load-context"] == serde_json::json!(["child-frame"])).collect();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|r| r["trigger"]["resource-type"] == serde_json::json!(["document"])));
    }

    #[test]
    fn only_safe_patterns_pass() {
        for good in [
            ".*",
            r"^[htpsw]+://([^:/]+\.)?a\.example\.?[:/]",
            r"^[htpsw]+://([^:/]+\.)?a\.example/x.*y[^a-zA-Z0-9_.%-]",
            r"/ads/.*\?id=",
            r"^https://a\.example/x$",
            r"a\{b\}c\|d\$e",
        ] {
            assert!(filter_is_safe(good), "{good}");
        }
        for bad in [
            "",
            "a|b",
            r"a\d+",
            "a{2}",
            "(a)",
            "a+",
            "a?b",
            "[abc]",
            "a^b",
            "a$b",
            "ab\\",
            "b\u{e4}r",
            "a b",
            "x*",
        ] {
            assert!(!filter_is_safe(bad), "{bad:?}");
        }
    }

    #[test]
    fn stamps_from_the_future_are_not_believed() {
        let list = |version: &str| format!("[Adblock Plus 2.0]\n! Version: {version}\n");
        assert_eq!(stamp_before(&list("202611220546"), "20261124").as_deref(), Some("202611220546"));
        assert_eq!(stamp_before(&list("20261124235959"), "20261124").as_deref(), Some("20261124235959"));
        // Ein Fantasie-Datum gälte sonst für immer als das neueste.
        assert_eq!(stamp_before(&list("99999999999999"), "20261124"), None);
        assert_eq!(stamp_before(&list("202611250000"), "20261124"), None);
        assert_eq!(stamp_before(&list("19990101"), "20261124"), None);
        assert_eq!(stamp_before("! Updated: 2031-01-01T00:00:00Z\n", "20261124"), None);
        // Das Datum nach der Uhr des Macs: acht Ziffern, und übermorgen liegt nicht vor heute.
        assert!(day_digits(0).len() == 8 && day_digits(0).bytes().all(|b| b.is_ascii_digit()));
        assert!(day_digits(2) >= day_digits(0));

        let sites: HashSet<String> = ["a.example".to_string()].into_iter().collect();
        assert!(covered(&sites, "a.example") && covered(&sites, "x.y.a.example"));
        assert!(!covered(&sites, "ba.example") && !covered(&sites, "example") && !covered(&sites, ""));
    }

    #[test]
    fn a_dot_after_the_host_name_changes_nothing() {
        // `beispiel.de.` ist derselbe Rechner wie `beispiel.de` – die Muster nehmen den Punkt hin.
        let closed = url_filters("||listed.example^").unwrap().list;
        assert_eq!(closed, vec![r"^[htpsw]+://([^:/]+\.)?listed\.example\.?[:/]".to_string()]);
        let path = url_filters("||listed.example/login").unwrap().list;
        assert_eq!(path, vec![r"^[htpsw]+://([^:/]+\.)?listed\.example\.?/login".to_string()]);
        let sep = url_filters("||listed.example^track").unwrap().list;
        assert_eq!(sep, vec![format!(r"^[htpsw]+://([^:/]+\.)?listed\.example\.?{SEP}track")]);
        // Kein vollständiger Name (Platzhalter folgt) – dann auch kein Punkt.
        let open = url_filters("||ads.example*banner").unwrap().list;
        assert_eq!(open, vec![r"^[htpsw]+://([^:/]+\.)?ads\.example.*banner".to_string()]);
        for filter in closed.iter().chain(&path).chain(&sep).chain(&open) {
            assert!(filter_is_safe(filter), "{filter}");
        }
        // Ein „vielleicht“ anderswo bleibt verboten.
        assert!(!filter_is_safe(r"^[htpsw]+://([^:/]+\.)?a\.b/x?"));
        assert!(!filter_is_safe(r"abc\/?def"));

        let index = guard_index(&["||listed.example^\n||forms.example/pfad/x^$doc\n1.2.3.4\n"]);
        for url in [
            "https://listed.example/login",
            "https://listed.example./login",
            "https://sub.LISTED.example.:8443/",
            "https://forms.example/pfad/x",
            "https://forms.example./%70fad/x?y=1",
            "http://1.2.3.4/",
            "http://[::ffff:1.2.3.4]/",
        ] {
            assert!(index.matches(url), "{url}");
        }
        for url in ["https://forms.example/pfad/xy", "https://forms.example/anders", "https://notlisted.example./", "https://example./"] {
            assert!(!index.matches(url), "{url}");
        }
        // Welcher Eintrag es war: der gemeldete Rechner – oder genau die eine gemeldete Adresse.
        assert_eq!(index.matched("https://sub.listed.example/x").as_deref(), Some("h:sub.listed.example"));
        assert_eq!(index.matched("https://listed.example./x").as_deref(), Some("h:listed.example"));
        assert_eq!(index.matched("https://forms.example/pfad/x?y=1").as_deref(), Some("p:forms.example/pfad/x"));
        assert_eq!(index.matched("https://forms.example/"), None);
        assert_eq!(plain_escapes("/%70fad/%2fx/%zz/ä%e4"), "/pfad/%2Fx/%zz/ä%E4");
    }

    #[test]
    fn reported_scam_sites_block_the_whole_host() {
        let text = "! Title: Probe\nboese.example\n10.20.30.40\n||teil.example/pfad/x^$all\n||warnung.example^$doc\n||nachladen.example/skript.js\n||ganz.example^\n@@||ganz.example/erlaubt^\nmit_unterstrich.example\n||ziel.example^$doc,to=~gut.example\nboese.example##.x\n";
        let converted = convert_guard(&[text], false);
        assert_eq!(converted.lists.len(), 1);
        let rules: Vec<serde_json::Value> = serde_json::from_str(&converted.lists[0]).unwrap();
        let types = |index: usize| -> Vec<String> {
            rules[index]["trigger"]["resource-type"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect()
        };
        // Bloßer Rechnername: alles, auch die Seite selbst und neue Fenster.
        assert_eq!(rules[0]["trigger"]["url-filter"], r"^[htpsw]+://([^:/]+\.)?boese\.example\.?[:/]");
        assert!(types(0).contains(&"top-document".to_string()) && types(0).contains(&"popup".to_string()));
        assert!(rules[1]["trigger"]["url-filter"].as_str().unwrap().contains(r"10\.20\.30\.40"));
        // $all bei einer einzelnen Adresse – zweimal, weil sie auch am Adressende greift.
        assert!(types(2).contains(&"top-document".to_string()) && types(2).contains(&"script".to_string()));
        assert_eq!(rules[3]["trigger"]["url-filter"], r"^[htpsw]+://([^:/]+\.)?teil\.example\.?/pfad/x$");
        // $doc: nur die Seite.
        assert_eq!(types(4), ["top-document"]);
        // Eine Adresse mit Pfad ohne Angabe: nur, was Seiten nachladen.
        assert!(!types(5).contains(&"top-document".to_string()) && types(5).contains(&"script".to_string()));
        assert!(types(6).contains(&"top-document".to_string()));
        // Auch Namen mit Unterstrich sind Rechnernamen.
        assert!(types(7).contains(&"top-document".to_string()));
        // Die Ausnahme steht am Ende; Unausdrückbares und Ausblend-Regeln fehlen.
        assert_eq!(rules.len(), 10);
        assert_eq!(rules[8]["action"]["type"], "ignore-previous-rules");
        assert_eq!(converted.stats, Stats { blocking: 8, exceptions: 2, hiding: 0, skipped: 2 });
        // In einer Werbeliste hieße dieselbe Zeile etwas anderes: dort nie die Seite selbst.
        let ads = convert(&["||ganz.example^"], false);
        assert!(!ads.lists[0].contains("top-document"));

        // Dasselbe als Verzeichnis für die Warnung vor dem Öffnen.
        let index = guard_index(&[text]);
        for hit in [
            "https://boese.example/",
            "http://www.tief.boese.example/anmelden?x=1",
            "https://10.20.30.40/x",
            "https://teil.example/pfad/x",
            "https://teil.example/pfad/x?weiter=1",
            "https://warnung.example/irgendwas",
            "https://ganz.example/",
        ] {
            assert!(index.matches(hit), "{hit}");
        }
        for miss in [
            "https://harmlos.example/",
            "https://nichtboese.example/",           // nur an der Punktgrenze
            "https://teil.example/pfad/xy",          // andere Adresse auf demselben Rechner
            "https://teil.example/",
            "https://nachladen.example/skript.js",   // meint nur das Nachladen
            "https://ziel.example/",                 // hatte eine Bedingung, die sich nicht prüfen lässt
            "ftp://boese.example/",
            "kein url",
        ] {
            assert!(!index.matches(miss), "{miss}");
        }
    }

    #[test]
    fn list_headers() {
        let head = "[Adblock Plus 2.0]\n! Version: 202611220546\n! Title: EasyList\n";
        assert_eq!(stamp(head).as_deref(), Some("202611220546"));
        assert_eq!(stamp("! Title: x\n! Updated: 2026-11-22T00:03:04Z\n").as_deref(), Some("20261122000304"));
        assert_eq!(stamp("! Title: x\n! Last modified: Thu, 19 Nov 2026 19:00:23 +0000\n").as_deref(), Some("20261119190023"));
        assert_eq!(stamp("! Last modified: irgendwann\n"), None);
        assert_eq!(stamp("! Version: <script>"), None);
        assert!(!plausible(head));
        assert!(!plausible("<html>Fehler</html>"));
    }

    /// Die mitgelieferten Listen als Ganzes: Jede Regel muss den Aufbau haben,
    /// den WebKit verlangt – ein Ausreißer ließe die ganze Liste scheitern.
    #[test]
    fn bundled_lists_convert_cleanly() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("filters");
        let texts: Vec<String> =
            SOURCES.iter().map(|s| std::fs::read_to_string(dir.join(format!("{}.txt", s.name))).unwrap()).collect();
        for text in &texts {
            assert!(plausible(text));
        }
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        for compat in [false, true] {
            let converted = convert(&refs, compat);
            let stats = converted.stats;
            assert!(stats.blocking > 100_000 && stats.hiding > 20_000 && stats.exceptions > 1_500, "{stats:?}");
            // Der weitaus größte Teil der Listen muss ankommen.
            assert!(stats.skipped * 50 < stats.rules(), "{stats:?}");
            assert!(converted.lists.len() >= 3 && converted.lists.len() <= 12);
            let mut total = 0;
            for list in &converted.lists {
                let rules: Vec<serde_json::Value> = serde_json::from_str(list).unwrap();
                assert!(!rules.is_empty() && rules.len() <= 60_000, "{}", rules.len());
                total += rules.len();
                for rule in &rules {
                    let trigger = rule["trigger"].as_object().unwrap();
                    let filter = trigger["url-filter"].as_str().unwrap();
                    assert!(filter_is_safe(filter), "{filter}");
                    assert!(filter.is_ascii());
                    let conditions = ["if-domain", "unless-domain", "if-top-url", "unless-top-url"]
                        .iter()
                        .filter(|key| trigger.contains_key(**key))
                        .count();
                    assert!(conditions <= 1, "{rule}");
                    for key in ["if-domain", "unless-domain"] {
                        for domain in trigger.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
                            let domain = domain.as_str().unwrap();
                            assert!(domain_ok(domain.trim_start_matches('*')), "{domain}");
                        }
                    }
                    for top in trigger.get("if-top-url").and_then(|v| v.as_array()).into_iter().flatten() {
                        let top = top.as_str().unwrap();
                        assert!(filter_is_safe(top.strip_suffix(".*").unwrap_or(top)), "{top}");
                    }
                    let action = rule["action"]["type"].as_str().unwrap();
                    assert!(matches!(action, "block" | "css-display-none" | "ignore-previous-rules"), "{action}");
                    // Eine Sperre ohne Typ-Angabe träfe auch die Seite selbst.
                    if action == "block" {
                        assert!(trigger.contains_key("resource-type"), "{rule}");
                        let types = trigger["resource-type"].as_array().unwrap();
                        let whole_document = types.iter().any(|t| t == "document") && !trigger.contains_key("load-context");
                        let page_too = types.iter().any(|t| t == "top-document")
                            || trigger.get("load-context") == Some(&serde_json::json!(["top-frame"]));
                        assert!(!whole_document || compat, "{rule}");
                        // Die Seite selbst nur bei Regeln, die sie ausdrücklich meinen
                        // ($document) – nie zusammen mit dem ganzen Vorgabe-Satz.
                        assert!(!((page_too || whole_document) && types.len() >= 8), "{rule}");
                    }
                }
            }
            assert!(total >= stats.rules());
        }
    }

    /// Schreibt die übersetzten Listen in einen Ordner – zum Gegenprüfen mit
    /// WebKit selbst. Aufruf: ZUNDER_FILTER_DUMP=<ordner> cargo test dump_lists -- --ignored
    #[test]
    #[ignore]
    fn dump_lists() {
        let Some(out) = std::env::var_os("ZUNDER_FILTER_DUMP") else {
            return;
        };
        let out = std::path::PathBuf::from(out);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("filters");
        let texts: Vec<String> =
            SOURCES.iter().map(|s| std::fs::read_to_string(dir.join(format!("{}.txt", s.name))).unwrap()).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        // Dazu die Listen gemeldeter Betrugsseiten, falls sie im Ordner liegen.
        let guard: Vec<String> = GUARD_SOURCES.iter().filter_map(|s| std::fs::read_to_string(out.join(format!("{}.txt", s.name))).ok()).collect();
        if !guard.is_empty() {
            let guard_refs: Vec<&str> = guard.iter().map(String::as_str).collect();
            let started = std::time::Instant::now();
            let converted = convert_guard(&guard_refs, false);
            println!("schutz: {:?} in {:?}", converted.stats, started.elapsed());
            for (index, list) in converted.lists.iter().enumerate() {
                std::fs::write(out.join(format!("schutz-{index}.json")), list).unwrap();
            }
        }
        for (compat, tag) in [(false, "neu"), (true, "alt")] {
            let started = std::time::Instant::now();
            let converted = convert(&refs, compat);
            println!("{tag}: {:?} in {:?}", converted.stats, started.elapsed());
            for (index, list) in converted.lists.iter().enumerate() {
                std::fs::write(out.join(format!("{tag}-{index}.json")), list).unwrap();
            }
        }
    }
}
