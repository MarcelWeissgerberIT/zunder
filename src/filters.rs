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

/// Höchstens so viele Sperren je übersetzter Liste (dazu kommen die Ausnahmen).
const CHUNK: usize = 40_000;

/// Vor dem Rechnernamen: Schema (http, https, ws, wss) und beliebig viele
/// vorangestellte Namensteile – `||beispiel.de` trifft auch `a.b.beispiel.de`.
const PREFIX: &str = r"^[htpsw]+://([^:/]+\.)?";
/// Ein Trennzeichen im Sinn der Listen: alles außer Buchstaben, Ziffern und `_ . % -`.
const SEP: &str = "[^a-zA-Z0-9_.%-]";
/// Nach einem vollständigen Rechnernamen: Port oder Pfad.
const HOST_END: &str = "[:/]";

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
        let plain = !host.is_empty() && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
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
    let mut last_star = false;
    for c in core.chars() {
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
                    Some(next) if next.is_ascii_punctuation() => i += 2,
                    _ => return false,
                }
            }
            b'[' => {
                if rest[i..].starts_with(SEP) {
                    i += SEP.len();
                } else if rest[i..].starts_with(HOST_END) {
                    i += HOST_END.len();
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

/// Liest eine Sperr- oder Ausnahmeregel. `None`: nicht genau ausdrückbar.
fn parse_net(line: &str) -> Option<Net> {
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
                        entry.extend(domains.include);
                    }
                }
                Some(Cosmetic::Unsupported) => stats.skipped += 1,
                None => match parse_net(line) {
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
            Some(sites) => include
                .iter()
                .filter(|entry| {
                    let domain = entry.trim_start_matches('*');
                    !sites.iter().any(|site| domain == site || domain.ends_with(&format!(".{site}")))
                })
                .flat_map(|entry| {
                    // Eine Ausnahme für x.a.de: Dann gilt die Regel nur auf a.de selbst.
                    let domain = entry.trim_start_matches('*');
                    let inner = entry.starts_with('*') && sites.iter().any(|site| site.ends_with(&format!(".{domain}")));
                    match inner {
                        true => vec![domain.to_string()],
                        false => vec![entry.clone()],
                    }
                })
                .collect(),
        };
        if !include.is_empty() {
            push_hide(&mut hide_specific, selector, &include, &[]);
        }
    }
    stats.hiding = hide_open.len() + hide_limited.len() + hide_specific.len();

    let mut lists = Vec::new();
    if stats.hiding > 0 {
        lists.push(join(&[&hide_open, &hide_limited, &unhide_generic, &hide_specific, &unhide_all, &whole_pages]));
    }
    for part in blocks.chunks(CHUNK) {
        lists.push(join(&[part, &exceptions, &whole_pages]));
    }
    Converted { lists, stats }
}

/// Der Stand einer Liste, wie ihn die Kopfzeile nennt (`! Version: 202611220546`).
pub fn stamp(text: &str) -> Option<String> {
    text.lines().take(40).find_map(|line| {
        let value = line.strip_prefix("! Version:")?.trim();
        let plausible = (8..=14).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit());
        plausible.then(|| value.to_string())
    })
}

/// Sieht der Text nach einer vollständigen Filterliste aus? Abgeschnittene
/// oder fremde Antworten (Fehlerseiten, Anmeldeseiten) werden nie übernommen.
pub fn plausible(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.len() >= 20_000
        && text.len() <= 12_000_000
        && text.starts_with("[Adblock")
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
        assert_eq!(rules[0]["trigger"]["url-filter"], r"^[htpsw]+://([^:/]+\.)?werbung\.example[:/]");
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
        assert!(net[2]["trigger"]["if-top-url"][0].as_str().unwrap().ends_with("g\\.example[:/].*"));
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
            r"^[htpsw]+://([^:/]+\.)?a\.example[:/]",
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
    fn list_headers() {
        let head = "[Adblock Plus 2.0]\n! Version: 202611220546\n! Title: EasyList\n";
        assert_eq!(stamp(head).as_deref(), Some("202611220546"));
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
