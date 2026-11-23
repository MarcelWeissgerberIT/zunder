//! Seitencheck: Wo wäre die angezeigte Seite angreifbar?
//!
//! Ausgewertet wird nur, was beim gewöhnlichen Laden ohnehin anfällt: der
//! Antwortkopf der Seite, ihr Aufbau (Skripte, Formulare, Rahmen), die
//! Merkmale ihrer Cookies und was WebKit über die Verbindung weiß. Es gibt
//! keinen zusätzlichen Abruf und keinen Angriff. Der Check sagt, welche
//! Schutzschichten einer Seite fehlen – nicht, ob im Programm der Website eine
//! Lücke steckt; das sieht von außen niemand.
//!
//! Hier steht die reine Auswertung, damit sie sich prüfen lässt (siehe Tests).
//! Die Fakten sammeln `native.rs` (Antwortkopf, Cookies, Verbindung) und
//! `ui/check.js` (Aufbau der Seite). Alles davon stammt von der Website und
//! ist damit Auskunft, kein Befehl: Es wird gezählt, verglichen und – eng
//! begrenzt – angezeigt, mehr nicht.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::i18n::t;

/// Die Zeilen des Antwortkopfs, die der Check auswertet (klein geschrieben).
pub const HEADERS: &[&str] = &[
    "content-security-policy",
    "content-security-policy-report-only",
    "strict-transport-security",
    "x-frame-options",
    "x-content-type-options",
    "referrer-policy",
    "server",
    "x-powered-by",
];

/// Was der Server zur Seite mitgeschickt hat.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    /// Adresse des Dokuments, ohne das Stück hinter `#`.
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| *key == name).map(|(_, value)| value.as_str())
    }
}

/// Merkmale eines Cookies – nie sein Wert.
#[derive(Clone, Debug, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub secure: bool,
    pub http_only: bool,
    /// „lax“, „strict“ oder leer (nicht gesetzt).
    pub same_site: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct DomScript {
    /// Adresse des Skripts.
    #[serde(default)]
    pub u: String,
    /// Trägt eine Prüfsumme (`integrity`).
    #[serde(default)]
    pub i: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct DomForm {
    /// Wohin das Formular sendet.
    #[serde(default)]
    pub a: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct DomFrame {
    #[serde(default)]
    pub u: String,
    /// Läuft mit `sandbox`.
    #[serde(default)]
    pub s: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct DomMixed {
    #[serde(default)]
    pub active: u32,
    #[serde(default)]
    pub passive: u32,
}

/// Was `ui/check.js` aus der Seite liest.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Dom {
    #[serde(default)]
    pub scripts: Vec<DomScript>,
    #[serde(default)]
    pub forms: Vec<DomForm>,
    #[serde(default)]
    pub passwords: u32,
    #[serde(default)]
    pub frames: Vec<DomFrame>,
    #[serde(default)]
    pub mixed: DomMixed,
    /// Eine Content-Security-Policy, die im Dokument selbst steht (`<meta>`).
    #[serde(default)]
    pub csp: String,
}

impl Dom {
    /// Liest die Antwort des Skripts. Sie kommt aus der Seite – also wird hier
    /// noch einmal begrenzt, was das Skript schon begrenzt hat.
    pub fn parse(json: &str) -> Option<Dom> {
        if json.len() > 600_000 {
            return None;
        }
        let mut dom: Dom = serde_json::from_str(json).ok()?;
        dom.scripts.truncate(300);
        dom.forms.truncate(60);
        dom.frames.truncate(120);
        for script in &mut dom.scripts {
            clip(&mut script.u, 400);
        }
        for form in &mut dom.forms {
            clip(&mut form.a, 400);
        }
        for frame in &mut dom.frames {
            clip(&mut frame.u, 400);
        }
        clip(&mut dom.csp, 4_000);
        Some(dom)
    }
}

fn clip(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// Alles, was über die Seite bekannt ist. Was noch fehlt, ist `None`.
#[derive(Default)]
pub struct Facts<'a> {
    pub url: &'a str,
    pub response: Option<&'a Response>,
    pub dom: Option<&'a Dom>,
    pub cookies: Option<&'a [Cookie]>,
    /// Laut WebKit kam alles Gezeigte verschlüsselt an.
    pub only_secure: Option<bool>,
    /// Die Verbindung nutzt TLS 1.0 oder 1.1.
    pub legacy_tls: bool,
    /// Der Aufbau der Seite ließ sich nicht lesen.
    pub dom_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Hier ist jetzt etwas gefährdet.
    Serious,
    /// Eine Schutzschicht fehlt.
    Weak,
    /// Gut zu wissen; ändert die Ampel nicht.
    Hint,
    /// In Ordnung.
    Good,
}

/// Ein Befund: Kennung (dazu gehören die Texte `sc_<id>` und `sc_<id>_d`) und
/// was in die Texte eingesetzt wird.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub id: &'static str,
    pub args: Vec<(&'static str, String)>,
}

impl Finding {
    fn new(level: Level, id: &'static str) -> Self {
        Finding { level, id, args: Vec::new() }
    }

    /// Ein Befund für die Nachbarn dieser Datei (`security.rs`): dieselbe Form,
    /// damit die Oberfläche nichts davon merkt, dass er aus einem eigenen Abruf stammt.
    pub fn note(level: Level, id: &'static str) -> Self {
        Finding::new(level, id)
    }

    pub fn with(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.args.push((key, value.into()));
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// Der Antwortkopf war nicht bekannt – dann fehlt ein Teil der Prüfung.
    pub partial: bool,
}

impl Report {
    /// Die Ampel: 0 rot, 1 gelb, 2 grün.
    pub fn level(&self) -> u8 {
        if self.findings.iter().any(|finding| finding.level == Level::Serious) {
            0
        } else if self.partial || self.findings.iter().any(|finding| finding.level == Level::Weak) {
            1
        } else {
            2
        }
    }

    /// Wie viele Stellen angreifbar sind (ernste und Schwächen).
    pub fn count(&self) -> usize {
        self.findings.iter().filter(|finding| finding.level <= Level::Weak).count()
    }

    /// Aufbereitet für die Oberfläche: Texte in der Sprache des Browsers.
    pub fn view(&self, site: &str) -> Value {
        let fill = |key: &str, finding: &Finding| {
            let mut text = t(key).to_string();
            for (name, value) in &finding.args {
                text = text.replace(&format!("{{{name}}}"), value);
            }
            text
        };
        let items: Vec<Value> = self
            .findings
            .iter()
            .map(|finding| {
                json!({
                    "l": finding.level as u8,
                    "t": fill(&format!("sc_{}", finding.id), finding),
                    "d": fill(&format!("sc_{}_d", finding.id), finding),
                })
            })
            .collect();
        json!({
            "level": self.level(),
            "count": self.count(),
            "serious": self.findings.iter().filter(|finding| finding.level == Level::Serious).count(),
            "partial": self.partial,
            "site": site,
            "items": items,
        })
    }
}

// ---------------------------------------------------------------------------
// Auswertung
// ---------------------------------------------------------------------------

/// Sammelstellen, von denen viele Websites dieselben Skripte laden. Wird eine
/// übernommen, trifft es alle – deshalb gehört dort eine Prüfsumme dazu.
const PUBLIC_CDNS: &[&str] = &[
    "cdnjs.cloudflare.com",
    "cdn.jsdelivr.net",
    "unpkg.com",
    "code.jquery.com",
    "ajax.googleapis.com",
    "stackpath.bootstrapcdn.com",
    "maxcdn.bootstrapcdn.com",
    "cdn.bootcdn.net",
    "cdn.bootcss.com",
    "cdn.staticfile.org",
    "ajax.aspnetcdn.com",
    "cdn.rawgit.com",
    "rawgit.com",
    "raw.githack.com",
    "esm.sh",
    "cdn.skypack.dev",
];

fn scheme(url: &str) -> &str {
    let lower = url.get(..8).unwrap_or(url);
    if lower.eq_ignore_ascii_case("https://") {
        "https"
    } else if lower.get(..7).is_some_and(|start| start.eq_ignore_ascii_case("http://")) {
        "http"
    } else {
        ""
    }
}

fn host(url: &str) -> String {
    crate::shield::host_of(url)
}

/// Höchstens `max` Einträge, mit Komma verbunden; was darüber hinausgeht, wird gezählt.
fn list(items: &[String], max: usize) -> String {
    let mut text = items.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > max {
        text.push_str(&format!(" +{}", items.len() - max));
    }
    text
}

/// Wertet aus, was über die Seite bekannt ist.
pub fn assess(facts: &Facts) -> Report {
    let mut found: Vec<Finding> = Vec::new();
    let https = scheme(facts.url) == "https";
    let page_host = host(facts.url);
    let local = crate::shield::is_local_host(&page_host);
    let site = crate::sites::domain_of(facts.url).unwrap_or_else(|| page_host.clone());

    // --- Verbindung -----------------------------------------------------------
    if !https {
        if local {
            found.push(Finding::new(Level::Hint, "local"));
        } else {
            found.push(Finding::new(Level::Serious, "http"));
        }
    } else {
        if facts.legacy_tls {
            found.push(Finding::new(Level::Serious, "tls_old"));
        } else {
            found.push(Finding::new(Level::Good, "ok_https"));
        }
        let counted = facts.dom.map_or(0, |dom| dom.mixed.active + dom.mixed.passive);
        if counted > 0 || facts.only_secure == Some(false) {
            found.push(Finding::new(Level::Weak, "mixed").with("n", counted.max(1).to_string()));
        }
    }

    // --- Antwortkopf ----------------------------------------------------------
    let meta_csp = facts.dom.map(|dom| dom.csp.as_str()).filter(|csp| !csp.trim().is_empty());
    match facts.response {
        None => found.push(Finding::new(Level::Hint, "headers_unknown")),
        Some(response) => {
            if https && !local {
                match response.header("strict-transport-security").and_then(hsts_seconds) {
                    None | Some(0) => found.push(Finding::new(Level::Weak, "hsts_none")),
                    Some(seconds) if seconds < 30 * 86_400 => {
                        found.push(Finding::new(Level::Hint, "hsts_short").with("v", span(seconds)))
                    }
                    Some(_) => found.push(Finding::new(Level::Good, "ok_hsts")),
                }
            }

            // Schutz vor eingeschleustem Code.
            let enforced = response.header("content-security-policy");
            let mut policies: Vec<Csp> = enforced.map(parse_csp).unwrap_or_default();
            if let Some(meta) = meta_csp {
                policies.extend(parse_csp(meta));
            }
            if policies.is_empty() {
                if response.header("content-security-policy-report-only").is_some() {
                    found.push(Finding::new(Level::Weak, "csp_report"));
                } else {
                    found.push(Finding::new(Level::Weak, "csp_none"));
                }
            } else {
                let gaps = script_gaps(&policies);
                if gaps.is_empty() {
                    found.push(Finding::new(Level::Good, "ok_csp"));
                } else {
                    let words: Vec<String> = gaps.iter().map(|gap| t(gap).to_string()).collect();
                    found.push(Finding::new(Level::Weak, "csp_weak").with("list", words.join(", ")));
                }
            }

            // Einbetten in fremde Seiten. Eine Regel im Dokument selbst zählt
            // hier nicht – Browser beachten `frame-ancestors` nur im Antwortkopf.
            let header_policies = enforced.map(parse_csp).unwrap_or_default();
            let by_csp = header_policies.iter().any(|policy| {
                policy.get("frame-ancestors").is_some_and(|sources| {
                    !sources.iter().any(|source| matches!(source.as_str(), "*" | "https:" | "http:"))
                })
            });
            let by_header = response.header("x-frame-options").is_some_and(|value| {
                value.split(',').any(|part| {
                    let part = part.trim();
                    part.eq_ignore_ascii_case("deny") || part.eq_ignore_ascii_case("sameorigin")
                })
            });
            if by_csp || by_header {
                found.push(Finding::new(Level::Good, "ok_frame"));
            } else {
                found.push(Finding::new(Level::Weak, "frame"));
            }

            // Was die Website über sich verrät.
            let software = software(response);
            if let Some(php) = old_php(response) {
                found.push(Finding::new(Level::Weak, "php_old").with("v", php));
            }
            if !software.is_empty() {
                found.push(Finding::new(Level::Hint, "server").with("v", software));
            }
            if !response
                .header("x-content-type-options")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("nosniff"))
            {
                found.push(Finding::new(Level::Hint, "nosniff"));
            }
            if let Some(policy) = response.header("referrer-policy") {
                // Bei mehreren Angaben gilt die letzte, die der Browser kennt.
                let last = policy.split(',').map(str::trim).filter(|part| !part.is_empty()).next_back().unwrap_or("");
                if last.eq_ignore_ascii_case("unsafe-url") {
                    found.push(Finding::new(Level::Hint, "referrer").with("v", "unsafe-url"));
                }
            }
        }
    }

    // --- Aufbau der Seite -----------------------------------------------------
    if facts.dom_failed && facts.dom.is_none() {
        found.push(Finding::new(Level::Hint, "dom_unknown"));
    }
    if let Some(dom) = facts.dom {
        // Formulare und Passwortfelder.
        if https {
            let plain = dom.forms.iter().filter(|form| scheme(&form.a) == "http" && !is_local(&form.a)).count();
            if plain > 0 {
                found.push(Finding::new(Level::Serious, "form_http").with("n", plain.to_string()));
            }
        } else if dom.passwords > 0 && !local {
            found.push(Finding::new(Level::Serious, "password_http"));
        }

        // Skripte von anderswo.
        let mut hosts: Vec<String> = Vec::new();
        let mut unchecked = 0usize;
        let mut cdn_hosts: Vec<String> = Vec::new();
        let mut cdn_unchecked = 0usize;
        let mut poisoned: Vec<String> = Vec::new();
        let mut old: Vec<String> = Vec::new();
        let (mut external, mut foreign_total) = (0usize, 0usize);
        for script in &dom.scripts {
            if scheme(&script.u).is_empty() {
                continue;
            }
            external += 1;
            let from = host(&script.u);
            if from == "polyfill.io" || from.ends_with(".polyfill.io") {
                if !poisoned.contains(&from) {
                    poisoned.push(from.clone());
                }
            }
            if let Some(note) = old_library(&script.u) {
                if !old.contains(&note) {
                    old.push(note);
                }
            }
            let foreign = crate::sites::domain_of(&script.u).is_some_and(|domain| domain != site);
            if !foreign {
                continue;
            }
            foreign_total += 1;
            if !hosts.contains(&from) {
                hosts.push(from.clone());
            }
            if !script.i {
                unchecked += 1;
                if PUBLIC_CDNS.contains(&from.as_str()) {
                    cdn_unchecked += 1;
                    if !cdn_hosts.contains(&from) {
                        cdn_hosts.push(from);
                    }
                }
            }
        }
        if !poisoned.is_empty() {
            found.push(Finding::new(Level::Serious, "polyfill").with("list", list(&poisoned, 2)));
        }
        if !old.is_empty() {
            found.push(Finding::new(Level::Weak, "lib_old").with("list", list(&old, 4)));
        }
        if cdn_unchecked > 0 {
            found.push(
                Finding::new(Level::Weak, "cdn_nosri")
                    .with("n", cdn_unchecked.to_string())
                    .with("list", list(&cdn_hosts, 3)),
            );
        }
        if foreign_total > 0 {
            found.push(
                Finding::new(Level::Hint, "third_scripts")
                    .with("n", foreign_total.to_string())
                    .with("m", hosts.len().to_string())
                    .with("k", unchecked.to_string())
                    .with("list", list(&hosts, 4)),
            );
        } else if external > 0 {
            found.push(Finding::new(Level::Good, "ok_scripts"));
        }

        // Fremde Rahmen ohne Einschränkung.
        let open = dom
            .frames
            .iter()
            .filter(|frame| {
                !frame.s
                    && !scheme(&frame.u).is_empty()
                    && crate::sites::domain_of(&frame.u).is_some_and(|domain| domain != site)
            })
            .count();
        if open > 0 {
            found.push(Finding::new(Level::Hint, "frames_open").with("n", open.to_string()));
        }
    }

    // --- Cookies ----------------------------------------------------------------
    if let Some(cookies) = facts.cookies {
        let session: Vec<&Cookie> = cookies.iter().filter(|cookie| session_like(&cookie.name)).collect();
        let names = |picked: Vec<&&Cookie>| {
            let names: Vec<String> = picked.iter().map(|cookie| cookie_name(&cookie.name)).collect();
            list(&names, 3)
        };
        let plain: Vec<&&Cookie> = session.iter().filter(|cookie| https && !cookie.secure).collect();
        let readable: Vec<&&Cookie> = session.iter().filter(|cookie| !cookie.http_only).collect();
        let loose: Vec<&&Cookie> = session.iter().filter(|cookie| cookie.same_site.is_empty()).collect();
        let fine = !session.is_empty() && plain.is_empty() && readable.is_empty();
        if !plain.is_empty() {
            found.push(Finding::new(Level::Weak, "cookie_insecure").with("list", names(plain)));
        }
        if !readable.is_empty() {
            found.push(Finding::new(Level::Weak, "cookie_script").with("list", names(readable)));
        }
        if !loose.is_empty() {
            found.push(Finding::new(Level::Hint, "cookie_samesite").with("list", names(loose)));
        }
        if fine {
            found.push(Finding::new(Level::Good, "ok_cookies"));
        }
    }

    // Das Ernste zuerst; innerhalb einer Stufe bleibt die Reihenfolge der Prüfung.
    found.sort_by_key(|finding| finding.level);
    // Ohne Antwortkopf oder ohne Blick in die Seite fehlt ein Teil der Prüfung –
    // „Solide“ gibt es dann nicht.
    Report { findings: found, partial: facts.response.is_none() || (facts.dom_failed && facts.dom.is_none()) }
}

fn is_local(url: &str) -> bool {
    crate::shield::is_local_host(&host(url))
}

/// Die Dauer aus `Strict-Transport-Security` in Sekunden.
fn hsts_seconds(value: &str) -> Option<u64> {
    value.split(';').find_map(|part| {
        let (name, number) = part.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("max-age") {
            return None;
        }
        number.trim().trim_matches('"').parse::<u64>().ok()
    })
}

/// Eine Dauer in Worten, grob.
fn span(seconds: u64) -> String {
    let (count, one, many) = if seconds < 3_600 {
        (seconds / 60, "sc_minute", "sc_minutes")
    } else if seconds < 86_400 {
        (seconds / 3_600, "sc_hour", "sc_hours")
    } else {
        (seconds / 86_400, "sc_day", "sc_days")
    };
    format!("{} {}", count.max(1), t(if count <= 1 { one } else { many }))
}

/// Eine Content-Security-Policy: Regelname → Quellen (alles klein geschrieben).
type Csp = std::collections::HashMap<String, Vec<String>>;

/// Zerlegt den Wert des Kopfs. Mehrere Policies stehen durch Komma getrennt;
/// eine Seite muss jede von ihnen einhalten.
fn parse_csp(value: &str) -> Vec<Csp> {
    value
        .split(',')
        .map(|policy| {
            let mut rules = Csp::new();
            for rule in policy.split(';') {
                let mut words = rule.split_ascii_whitespace();
                if let Some(name) = words.next() {
                    // Steht eine Regel doppelt da, gilt die erste.
                    rules.entry(name.to_ascii_lowercase()).or_insert_with(|| words.map(str::to_ascii_lowercase).collect());
                }
            }
            rules
        })
        .filter(|rules| !rules.is_empty())
        .collect()
}

/// Was die Policies bei Skripten durchgehen lassen (Schlüssel der Texte). Ein
/// eingeschleustes Skript läuft nur, wenn JEDE Policy es zulässt – eine Lücke
/// zählt also erst, wenn alle sie haben.
fn script_gaps(policies: &[Csp]) -> Vec<&'static str> {
    let (mut inline, mut anywhere, mut data, mut ruled) = (true, true, true, false);
    for policy in policies {
        let Some(sources) = policy.get("script-src").or_else(|| policy.get("default-src")) else {
            continue;
        };
        ruled = true;
        let has = |word: &str| sources.iter().any(|source| source == word);
        // Mit einmaligem Kennwort oder Prüfsumme zählt 'unsafe-inline' nicht mehr.
        let pinned = sources
            .iter()
            .any(|source| source.starts_with("'nonce-") || source.starts_with("'sha256-") || source.starts_with("'sha384-") || source.starts_with("'sha512-"));
        let dynamic = has("'strict-dynamic'");
        inline &= has("'unsafe-inline'") && !pinned && !dynamic;
        anywhere &= !dynamic && (has("*") || has("https:") || has("http:"));
        data &= !dynamic && has("data:");
    }
    if !ruled {
        return vec!["sc_gap_none"];
    }
    let mut gaps = Vec::new();
    if inline {
        gaps.push("sc_gap_inline");
    }
    if anywhere {
        gaps.push("sc_gap_any");
    }
    if data {
        gaps.push("sc_gap_data");
    }
    gaps
}

/// Ein Stück wie `nginx/1.18.0` – Name, Schrägstrich, Version.
fn product(word: &str) -> bool {
    let Some((name, version)) = word.split_once('/') else {
        return false;
    };
    let plain = |text: &str| text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    !name.is_empty()
        && name.len() <= 24
        && plain(name)
        && version.len() <= 16
        && version.starts_with(|c: char| c.is_ascii_digit())
        && plain(version)
}

/// Was die Website über ihre Software sagt – nur in der Form „Name/Version“.
/// Freien Text aus dem Kopf zeigt Zunder nicht: Er stünde sonst in Zunders
/// eigener Oberfläche, als hätte der Browser ihn geschrieben.
fn software(response: &Response) -> String {
    let mut parts: Vec<String> = Vec::new();
    for name in ["server", "x-powered-by"] {
        let Some(value) = response.header(name) else {
            continue;
        };
        for word in value.split_ascii_whitespace().filter(|word| product(word)).take(3) {
            if !parts.iter().any(|part| part == word) && parts.len() < 4 {
                parts.push(word.to_string());
            }
        }
    }
    parts.join(" · ")
}

/// Meldet die Website eine PHP-Reihe, die der Hersteller nicht mehr pflegt
/// (alles vor 8)? Zurück kommt etwa „PHP 7.4“.
fn old_php(response: &Response) -> Option<String> {
    ["x-powered-by", "server"].iter().find_map(|name| {
        response.header(name)?.split_ascii_whitespace().find_map(|word| {
            let (product, version) = word.split_once('/')?;
            if !product.eq_ignore_ascii_case("php") {
                return None;
            }
            let numbers = version_of(version)?;
            (numbers.0 < 8).then(|| format!("PHP {}.{}", numbers.0, numbers.1))
        })
    })
}

/// Liest „3.4.1“, „v4.7.6“ oder „1.12“ – drei Zahlen, fehlende sind 0.
fn version_of(text: &str) -> Option<(u32, u32, u32)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut numbers = [0u32; 3];
    let mut parts = 0;
    for (index, part) in text.split('.').take(3).enumerate() {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || digits.len() > 4 {
            break;
        }
        numbers[index] = digits.parse().ok()?;
        parts += 1;
        // Nach einem Anhängsel wie „-rc1“ kommt keine Zahl mehr.
        if digits.len() != part.len() {
            break;
        }
    }
    // Eine einzelne Zahl ist keine Version (sonst zählte jede Jahreszahl im Pfad).
    (parts >= 2).then_some((numbers[0], numbers[1], numbers[2]))
}

/// Was über eine Bibliothek bekannt ist.
enum Known {
    /// Bekannte Lücken, behoben ab dieser Version.
    Fixed(u32, u32, u32),
    /// Diese Hauptversion und alles davor wird nicht mehr gepflegt.
    Ended(u32),
}

struct Library {
    name: &'static str,
    /// Wie die Datei heißt (ohne `.js`, ohne `.min`).
    files: &'static [&'static str],
    /// Wie das Paket in Adressen heißt (`/paket/1.2.3/`, `paket@1.2.3`).
    packages: &'static [&'static str],
    known: &'static [Known],
}

/// Bibliotheken, deren Version sich aus der Adresse lesen lässt, mit dem, was
/// über ältere Fassungen bekannt ist (Stand: Sicherheitshinweise der Projekte).
const LIBRARIES: &[Library] = &[
    Library {
        name: "jQuery UI",
        files: &["jquery-ui", "jquery-ui.custom"],
        packages: &["jquery-ui", "jqueryui", "jquery-ui-dist", "ui"],
        known: &[Known::Fixed(1, 13, 2)],
    },
    Library {
        name: "jQuery",
        files: &["jquery", "jquery.slim"],
        packages: &["jquery"],
        known: &[Known::Fixed(3, 5, 0)],
    },
    Library {
        name: "Bootstrap",
        files: &["bootstrap", "bootstrap.bundle"],
        packages: &["bootstrap", "twitter-bootstrap"],
        known: &[Known::Fixed(3, 4, 1), Known::Fixed(4, 3, 1)],
    },
    Library {
        name: "AngularJS",
        files: &["angular"],
        packages: &["angular", "angularjs", "angular.js"],
        known: &[Known::Ended(1)],
    },
    Library {
        name: "Vue",
        files: &["vue", "vue.runtime", "vue.common"],
        packages: &["vue"],
        known: &[Known::Ended(2)],
    },
    Library {
        name: "Lodash",
        files: &["lodash", "lodash.core"],
        packages: &["lodash", "lodash.js"],
        known: &[Known::Fixed(4, 17, 21)],
    },
    Library {
        name: "Moment.js",
        files: &["moment", "moment-with-locales"],
        packages: &["moment", "moment.js"],
        known: &[Known::Fixed(2, 29, 4)],
    },
    Library {
        name: "Handlebars",
        files: &["handlebars", "handlebars.runtime"],
        packages: &["handlebars", "handlebars.js"],
        known: &[Known::Fixed(4, 7, 7)],
    },
];

/// Erkennt eine der bekannten Bibliotheken samt Version an der Adresse ihres Skripts.
fn library(url: &str) -> Option<(&'static Library, (u32, u32, u32))> {
    let parsed = url::Url::parse(url).ok()?;
    let path = parsed.path().to_ascii_lowercase();
    let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
    let file = segments.last()?.strip_suffix(".js")?;
    let stem = file.strip_suffix(".min").unwrap_or(file);
    for candidate in LIBRARIES {
        // Der Dateiname passt – allein oder mit angehängter Version („jquery-3.4.1“).
        let mut in_name = None;
        let matches = candidate.files.iter().any(|name| {
            if stem == *name {
                return true;
            }
            let Some(rest) = stem.strip_prefix(name).and_then(|rest| rest.strip_prefix('-')) else {
                return false;
            };
            let rest = rest.strip_suffix(".slim").or_else(|| rest.strip_suffix(".custom")).unwrap_or(rest);
            in_name = version_of(rest);
            in_name.is_some()
        });
        if !matches {
            continue;
        }
        let version = in_name.or_else(|| {
            segments.iter().enumerate().find_map(|(index, segment)| {
                // „paket@1.2.3“ …
                if let Some((package, version)) = segment.split_once('@') {
                    if candidate.packages.contains(&package) {
                        return version_of(version);
                    }
                }
                // … oder „/paket/1.2.3/“.
                let before = index.checked_sub(1).and_then(|before| segments.get(before))?;
                candidate.packages.contains(before).then(|| version_of(segment)).flatten()
            })
        });
        // WordPress hängt die Version seines jQuery als „?ver=“ an.
        let version = version.or_else(|| {
            let wordpress = candidate.name == "jQuery" && path.contains("/wp-includes/js/jquery/");
            let ver = parsed.query_pairs().find(|(key, _)| key == "ver").map(|(_, value)| value.into_owned());
            wordpress.then_some(ver).flatten().and_then(|ver| version_of(&ver))
        });
        return version.map(|version| (candidate, version));
    }
    None
}

/// Ist das Skript eine veraltete Fassung einer bekannten Bibliothek? Zurück
/// kommt der Hinweis für die Anzeige, etwa „jQuery 1.12.4 (behoben ab 3.5.0)“.
fn old_library(url: &str) -> Option<String> {
    let (found, version) = library(url)?;
    let shown = format!("{} {}.{}.{}", found.name, version.0, version.1, version.2);
    // Zwei Reihen mit je eigener Korrektur (Bootstrap 3 und 4): Es zählt die
    // Korrektur der eigenen Reihe; wer davor liegt, braucht die erste.
    let fixes: Vec<(u32, u32, u32)> = found
        .known
        .iter()
        .filter_map(|known| match known {
            Known::Fixed(a, b, c) => Some((*a, *b, *c)),
            Known::Ended(_) => None,
        })
        .collect();
    if let Some(Known::Ended(major)) = found.known.iter().find(|known| matches!(known, Known::Ended(_))) {
        return (version.0 <= *major).then(|| format!("{shown} ({})", t("sc_lib_ended")));
    }
    let fix = fixes.iter().find(|fix| fix.0 == version.0).or_else(|| {
        // Eine Reihe ohne eigene Korrektur: betroffen ist sie nur, wenn sie
        // älter ist als jede korrigierte.
        fixes.first().filter(|first| version.0 < first.0)
    })?;
    (version < *fix).then(|| {
        format!("{shown} ({})", t("sc_lib_fixed").replace("{v}", &format!("{}.{}.{}", fix.0, fix.1, fix.2)))
    })
}

/// Trägt ein Cookie dem Namen nach eine Anmeldung oder Sitzung?
fn session_like(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    // Diese sollen Skripte lesen können – das ist ihr Zweck.
    if name.contains("csrf") || name.contains("xsrf") {
        return false;
    }
    name.contains("sess")
        || name.ends_with("sid")
        || name.contains("auth")
        || name.contains("login")
        || name.contains("jwt")
        || name.contains("token")
        || name.contains("remember")
}

/// Der Name eines Cookies für die Anzeige: nur harmlose Zeichen, begrenzt.
fn cookie_name(name: &str) -> String {
    let clean: String =
        name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).take(32).collect();
    if clean.is_empty() {
        "?".to_string()
    } else {
        clean
    }
}

/// Der Stand des Checks für die Seite eines Tabs.
#[derive(Default)]
pub struct State {
    pub response: Option<Response>,
    pub dom: Option<Dom>,
    pub cookies: Option<Vec<Cookie>>,
    pub only_secure: Option<bool>,
    pub legacy_tls: bool,
    /// Zunders Skript bekam aus der Seite keine brauchbare Antwort.
    pub dom_failed: bool,
    /// Welcher Prüflauf gerade gilt – Antworten älterer Läufe verfallen.
    pub run: u64,
    /// Fertig aufbereitet für die Oberfläche.
    pub view: Value,
}

impl State {
    /// Rechnet neu, was über die Seite an dieser Adresse bekannt ist.
    pub fn refresh(&mut self, url: &str) {
        if scheme(url).is_empty() {
            self.view = Value::Null;
            return;
        }
        let report = assess(&Facts {
            url,
            response: self.response.as_ref(),
            dom: self.dom.as_ref(),
            cookies: self.cookies.as_deref(),
            only_secure: self.only_secure,
            legacy_tls: self.legacy_tls,
            dom_failed: self.dom_failed,
        });
        let site = crate::sites::domain_of(url).unwrap_or_else(|| host(url));
        self.view = report.view(&site);
    }
}

const CHECK_JS: &str = include_str!("../ui/check.js");

impl crate::app::App {
    /// Sieht sich die Seite eines Tabs an: ihren Aufbau, ihre Cookies, ihre
    /// Verbindung. Die Antworten kommen als Ereignisse zurück.
    pub fn check_page(&mut self, window: tao::window::WindowId, tab_id: crate::app::TabId) {
        self.next_check += 1;
        let run = self.next_check;
        let proxy = self.proxy.clone();
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tab) = win.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(webview) = &tab.webview else {
            return;
        };
        if scheme(&tab.url).is_empty() {
            return;
        }
        tab.check.run = run;
        if tab.check.response.is_none() {
            tab.check.response = crate::native::response_for(webview, &tab.url);
        }
        let (only_secure, legacy_tls) = crate::native::connection_facts(webview);
        tab.check.only_secure = Some(only_secure);
        tab.check.legacy_tls = legacy_tls;
        let answer = proxy.clone();
        crate::native::eval_isolated(webview, CHECK_JS, move |json| {
            let event = Event::Dom { window, tab: tab_id, run, json };
            let _ = answer.send_event(crate::app::UserEvent::SiteCheck(event));
        });
        crate::native::cookie_flags(webview, &host(&tab.url), move |cookies| {
            let event = Event::Cookies { window, tab: tab_id, run, cookies };
            let _ = proxy.send_event(crate::app::UserEvent::SiteCheck(event));
        });
        tab.check.refresh(&tab.url);
        win.dirty = true;
    }

    /// Ein Stück der Prüfung ist da.
    pub fn check_event(&mut self, event: Event) {
        let (window, tab_id, run) = match &event {
            Event::Dom { window, tab, run, .. } | Event::Cookies { window, tab, run, .. } => (*window, *tab, *run),
        };
        let Some(win) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(tab) = win.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        // Die Seite hat inzwischen gewechselt oder wird schon neu geprüft.
        if tab.check.run != run {
            return;
        }
        match event {
            Event::Dom { json, .. } => match json.as_deref().and_then(Dom::parse) {
                Some(dom) => {
                    tab.check.dom = Some(dom);
                    tab.check.dom_failed = false;
                }
                // Keine brauchbare Antwort: Was die Seite jetzt enthält, ist unbekannt.
                None => {
                    tab.check.dom = None;
                    tab.check.dom_failed = true;
                }
            },
            Event::Cookies { cookies, .. } => tab.check.cookies = Some(cookies),
        }
        tab.check.refresh(&tab.url);
        win.dirty = true;
    }
}

/// Antworten der Seite und des Cookie-Speichers auf einen Prüflauf.
pub enum Event {
    Dom { window: tao::window::WindowId, tab: crate::app::TabId, run: u64, json: Option<String> },
    Cookies { window: tao::window::WindowId, tab: crate::app::TabId, run: u64, cookies: Vec<Cookie> },
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Dom { run, json, .. } => write!(f, "Dom(run {run}, {} bytes)", json.as_ref().map_or(0, String::len)),
            Event::Cookies { run, cookies, .. } => write!(f, "Cookies(run {run}, {})", cookies.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(url: &str, headers: &[(&'static str, &str)]) -> Response {
        Response { url: url.to_string(), headers: headers.iter().map(|(name, value)| (*name, value.to_string())).collect() }
    }

    fn ids(report: &Report, level: Level) -> Vec<&'static str> {
        report.findings.iter().filter(|finding| finding.level == level).map(|finding| finding.id).collect()
    }

    fn script(url: &str, checked: bool) -> DomScript {
        DomScript { u: url.to_string(), i: checked }
    }

    #[test]
    fn a_hardened_page_is_green() {
        let response = response(
            "https://shop.example.com/",
            &[
                ("strict-transport-security", "max-age=31536000; includeSubDomains"),
                ("content-security-policy", "default-src 'self'; script-src 'self' 'nonce-abc' 'unsafe-inline'; frame-ancestors 'none'"),
                ("x-content-type-options", "nosniff"),
                ("referrer-policy", "strict-origin-when-cross-origin"),
                ("server", "cloudflare"),
            ],
        );
        let dom = Dom { scripts: vec![script("https://shop.example.com/app.js", false)], ..Dom::default() };
        let cookies = vec![Cookie { name: "session".into(), secure: true, http_only: true, same_site: "lax".into() }];
        let report = assess(&Facts {
            url: "https://shop.example.com/",
            response: Some(&response),
            dom: Some(&dom),
            cookies: Some(&cookies),
            only_secure: Some(true),
            legacy_tls: false,
            dom_failed: false,
        });
        assert_eq!(report.level(), 2, "{:?}", report.findings);
        assert_eq!(report.count(), 0);
        assert_eq!(ids(&report, Level::Good), vec!["ok_https", "ok_hsts", "ok_csp", "ok_frame", "ok_scripts", "ok_cookies"]);
        assert!(ids(&report, Level::Hint).is_empty());
    }

    #[test]
    fn a_bare_page_shows_where_it_is_open() {
        let response = response("https://blog.example.org/", &[("server", "nginx/1.18.0 (Ubuntu)"), ("x-powered-by", "PHP/7.4.3")]);
        let dom = Dom {
            scripts: vec![
                script("https://code.jquery.com/jquery-1.12.4.min.js", false),
                script("https://cdnjs.cloudflare.com/ajax/libs/twitter-bootstrap/3.3.7/js/bootstrap.min.js", true),
                script("https://www.googletagmanager.com/gtm.js?id=GTM-X", false),
                script("https://blog.example.org/wp-includes/js/jquery/jquery.min.js?ver=3.7.1", false),
            ],
            frames: vec![DomFrame { u: "https://www.youtube.com/embed/x".into(), s: false }],
            ..Dom::default()
        };
        let cookies = vec![
            Cookie { name: "PHPSESSID".into(), secure: false, http_only: false, same_site: String::new() },
            Cookie { name: "_ga".into(), secure: false, http_only: false, same_site: String::new() },
            Cookie { name: "XSRF-TOKEN".into(), secure: true, http_only: false, same_site: "lax".into() },
        ];
        let report = assess(&Facts {
            url: "https://blog.example.org/post",
            response: Some(&response),
            dom: Some(&dom),
            cookies: Some(&cookies),
            only_secure: Some(true),
            legacy_tls: false,
            dom_failed: false,
        });
        assert_eq!(report.level(), 1);
        assert_eq!(
            ids(&report, Level::Weak),
            vec!["hsts_none", "csp_none", "frame", "php_old", "lib_old", "cdn_nosri", "cookie_insecure", "cookie_script"]
        );
        assert_eq!(ids(&report, Level::Hint), vec!["server", "nosniff", "third_scripts", "frames_open", "cookie_samesite"]);
        let arg = |id: &str, key: &str| {
            report.findings.iter().find(|finding| finding.id == id).and_then(|finding| {
                finding.args.iter().find(|(name, _)| *name == key).map(|(_, value)| value.clone())
            })
        };
        // Nur „Name/Version“ wird gezeigt – „(Ubuntu)“ nicht.
        assert_eq!(arg("server", "v").as_deref(), Some("nginx/1.18.0 · PHP/7.4.3"));
        assert_eq!(arg("php_old", "v").as_deref(), Some("PHP 7.4"));
        // Beide alten Bibliotheken, die aktuelle von WordPress nicht.
        let old = arg("lib_old", "list").unwrap();
        assert!(old.contains("jQuery 1.12.4") && old.contains("Bootstrap 3.3.7") && !old.contains("3.7.1"), "{old}");
        // Nur das Skript ohne Prüfsumme von der Sammelstelle zählt hier.
        assert_eq!(arg("cdn_nosri", "n").as_deref(), Some("1"));
        assert_eq!(arg("cdn_nosri", "list").as_deref(), Some("code.jquery.com"));
        assert_eq!(arg("third_scripts", "n").as_deref(), Some("3"));
        assert_eq!(arg("third_scripts", "k").as_deref(), Some("2"));
        // Das Sitzungs-Cookie fällt auf, das Zähl-Cookie und das für Skripte gedachte nicht.
        assert_eq!(arg("cookie_script", "list").as_deref(), Some("PHPSESSID"));
        assert_eq!(arg("cookie_insecure", "list").as_deref(), Some("PHPSESSID"));
    }

    #[test]
    fn plain_connections_and_plain_forms_are_serious() {
        let dom = Dom { passwords: 1, ..Dom::default() };
        let report = assess(&Facts { url: "http://example.com/login", dom: Some(&dom), ..Facts::default() });
        assert_eq!(report.level(), 0);
        assert_eq!(ids(&report, Level::Serious), vec!["http", "password_http"]);
        // Ohne Antwortkopf gilt die Prüfung als unvollständig.
        assert!(report.partial);
        // Ebenso, wenn sich die Seite nicht lesen ließ – auch mit bestem Antwortkopf wird es dann nicht grün.
        let fine = response(
            "https://a.example/",
            &[
                ("strict-transport-security", "max-age=31536000"),
                ("content-security-policy", "default-src 'self'; frame-ancestors 'none'"),
                ("x-content-type-options", "nosniff"),
            ],
        );
        let unread = assess(&Facts { url: "https://a.example/", response: Some(&fine), dom_failed: true, ..Facts::default() });
        assert!(unread.partial && unread.level() == 1 && ids(&unread, Level::Hint).contains(&"dom_unknown"));
        let read = assess(&Facts { url: "https://a.example/", response: Some(&fine), dom: Some(&Dom::default()), ..Facts::default() });
        assert!(!read.partial && read.level() == 2, "{:?}", read.findings);

        // Ein Gerät im eigenen Netz ist ohne Verschlüsselung kein Alarm.
        let report = assess(&Facts { url: "http://192.168.1.1/", dom: Some(&dom), ..Facts::default() });
        assert!(ids(&report, Level::Serious).is_empty());
        assert_eq!(report.level(), 1);

        // Verschlüsselte Seite, aber das Formular sendet offen; dazu alte Verschlüsselung und polyfill.io.
        let response = response("https://example.com/", &[]);
        let dom = Dom {
            forms: vec![DomForm { a: "http://example.com/send".into() }, DomForm { a: "https://example.com/ok".into() }],
            scripts: vec![script("https://cdn.polyfill.io/v3/polyfill.min.js", false)],
            mixed: DomMixed { active: 0, passive: 2 },
            ..Dom::default()
        };
        let report = assess(&Facts {
            url: "https://example.com/",
            response: Some(&response),
            dom: Some(&dom),
            legacy_tls: true,
            ..Facts::default()
        });
        assert_eq!(ids(&report, Level::Serious), vec!["tls_old", "form_http", "polyfill"]);
        assert!(ids(&report, Level::Weak).contains(&"mixed"));
        assert_eq!(report.level(), 0);
    }

    #[test]
    fn content_security_policy_is_read_for_what_it_lets_through() {
        let gaps = |value: &str| script_gaps(&parse_csp(value));
        assert!(gaps("default-src 'self'").is_empty());
        assert_eq!(gaps("img-src *"), vec!["sc_gap_none"]);
        assert_eq!(gaps("script-src 'self' 'unsafe-inline'"), vec!["sc_gap_inline"]);
        assert_eq!(gaps("script-src * data: 'unsafe-inline'"), vec!["sc_gap_inline", "sc_gap_any", "sc_gap_data"]);
        // Mit Kennwort oder Prüfsumme zählt 'unsafe-inline' nicht, mit 'strict-dynamic' auch die Platzhalter nicht.
        assert!(gaps("script-src 'nonce-R4nd0m' 'unsafe-inline' https: 'strict-dynamic'").is_empty());
        assert_eq!(gaps("script-src 'sha256-abc' 'unsafe-inline' https:"), vec!["sc_gap_any"]);
        // Zwei Policies: Eine Lücke zählt nur, wenn beide sie haben.
        assert!(gaps("script-src 'unsafe-inline' *, script-src 'self'").is_empty());
        assert_eq!(gaps("script-src 'unsafe-inline' *, img-src 'self'"), vec!["sc_gap_inline", "sc_gap_any"]);
        // Groß- und Kleinschreibung, doppelte Regeln (die erste gilt).
        assert_eq!(gaps("Script-Src 'UNSAFE-INLINE'; script-src 'self'"), vec!["sc_gap_inline"]);

        // Nur beobachtet, nicht durchgesetzt.
        let watching = response("https://a.example/", &[("content-security-policy-report-only", "default-src 'self'")]);
        let report = assess(&Facts { url: "https://a.example/", response: Some(&watching), ..Facts::default() });
        assert!(ids(&report, Level::Weak).contains(&"csp_report"));
        // Eine Policy im Dokument schützt vor Skripten, aber nicht vor dem Einbetten.
        let bare = response("https://a.example/", &[]);
        let dom = Dom { csp: "script-src 'self'".into(), ..Dom::default() };
        let report = assess(&Facts { url: "https://a.example/", response: Some(&bare), dom: Some(&dom), ..Facts::default() });
        assert!(ids(&report, Level::Good).contains(&"ok_csp"));
        assert!(ids(&report, Level::Weak).contains(&"frame"));
        // Einbetten: X-Frame-Options oder frame-ancestors – aber nicht mit Platzhalter.
        for (headers, protected) in [
            (vec![("x-frame-options", "SAMEORIGIN")], true),
            (vec![("x-frame-options", "ALLOW-FROM https://x.example")], false),
            (vec![("content-security-policy", "frame-ancestors 'self' https://partner.example")], true),
            (vec![("content-security-policy", "frame-ancestors *")], false),
        ] {
            let response = response("https://a.example/", &headers);
            let report = assess(&Facts { url: "https://a.example/", response: Some(&response), ..Facts::default() });
            assert_eq!(ids(&report, Level::Good).contains(&"ok_frame"), protected, "{headers:?}");
        }
    }

    #[test]
    fn transport_rules_and_durations() {
        assert_eq!(hsts_seconds("max-age=31536000; includeSubDomains; preload"), Some(31_536_000));
        assert_eq!(hsts_seconds("includeSubDomains; Max-Age=\"600\""), Some(600));
        assert_eq!(hsts_seconds("includeSubDomains"), None);
        let check = |value: &str| {
            let response = response("https://a.example/", &[("strict-transport-security", value)]);
            let report = assess(&Facts { url: "https://a.example/", response: Some(&response), ..Facts::default() });
            report.findings.iter().map(|finding| finding.id).filter(|id| id.contains("hsts")).collect::<Vec<_>>()
        };
        assert_eq!(check("max-age=0"), vec!["hsts_none"]);
        assert_eq!(check("max-age=300"), vec!["hsts_short"]);
        assert_eq!(check("max-age=15552000"), vec!["ok_hsts"]);
    }

    #[test]
    fn libraries_are_recognised_by_their_address() {
        let found = |url: &str| library(url).map(|(library, version)| (library.name, version));
        assert_eq!(found("https://code.jquery.com/jquery-3.4.1.slim.min.js"), Some(("jQuery", (3, 4, 1))));
        assert_eq!(found("https://ajax.googleapis.com/ajax/libs/jquery/1.12.4/jquery.min.js"), Some(("jQuery", (1, 12, 4))));
        assert_eq!(found("https://cdn.jsdelivr.net/npm/jquery@3.7.1/dist/jquery.min.js"), Some(("jQuery", (3, 7, 1))));
        assert_eq!(found("https://code.jquery.com/ui/1.12.1/jquery-ui.js"), Some(("jQuery UI", (1, 12, 1))));
        assert_eq!(found("https://x.example/js/jquery-ui-1.13.2.custom.min.js"), Some(("jQuery UI", (1, 13, 2))));
        assert_eq!(found("https://stackpath.bootstrapcdn.com/bootstrap/4.3.1/js/bootstrap.bundle.min.js"), Some(("Bootstrap", (4, 3, 1))));
        assert_eq!(found("https://ajax.googleapis.com/ajax/libs/angularjs/1.6.9/angular.min.js"), Some(("AngularJS", (1, 6, 9))));
        assert_eq!(found("https://cdn.jsdelivr.net/npm/vue@2.6.14/dist/vue.min.js"), Some(("Vue", (2, 6, 14))));
        assert_eq!(found("https://cdnjs.cloudflare.com/ajax/libs/lodash.js/4.17.15/lodash.min.js"), Some(("Lodash", (4, 17, 15))));
        assert_eq!(found("https://cdnjs.cloudflare.com/ajax/libs/moment.js/2.24.0/moment-with-locales.min.js"), Some(("Moment.js", (2, 24, 0))));
        assert_eq!(found("https://x.example/handlebars-v4.7.6.js"), Some(("Handlebars", (4, 7, 6))));
        // Erweiterungen und Namensvettern sind nicht die Bibliothek selbst; ohne Version gibt es kein Urteil.
        for url in [
            "https://x.example/js/jquery.validate.min.js",
            "https://x.example/js/jquery-migrate-3.3.2.min.js",
            "https://x.example/2024/jquery.min.js",
            "https://x.example/assets/jquery.min.js?ver=6.4.2",
            "https://x.example/js/app.js",
            "not a url",
        ] {
            assert_eq!(found(url), None, "{url}");
        }

        let old = |url: &str| old_library(url).is_some();
        assert!(old("https://code.jquery.com/jquery-3.4.1.min.js"));
        assert!(!old("https://code.jquery.com/jquery-3.5.0.min.js"));
        assert!(old("https://code.jquery.com/ui/1.13.1/jquery-ui.js"));
        // Bootstrap: jede Reihe hat ihre eigene Korrektur, die 5er-Reihe ist nicht betroffen.
        assert!(old("https://x.example/bootstrap/3.3.7/js/bootstrap.min.js"));
        assert!(!old("https://x.example/bootstrap/3.4.1/js/bootstrap.min.js"));
        assert!(old("https://x.example/bootstrap/4.1.3/js/bootstrap.min.js"));
        assert!(!old("https://x.example/bootstrap/4.6.2/js/bootstrap.min.js"));
        assert!(!old("https://x.example/bootstrap/5.3.3/js/bootstrap.bundle.min.js"));
        assert!(old("https://x.example/bootstrap/2.3.2/js/bootstrap.min.js"));
        // Abgekündigte Reihen: AngularJS 1 und Vue 2 – Vue 3 nicht.
        assert!(old("https://x.example/angularjs/1.8.3/angular.min.js"));
        assert!(old("https://unpkg.com/vue@2.7.16/dist/vue.min.js"));
        assert!(!old("https://unpkg.com/vue@3.4.21/dist/vue.global.prod.js"));
        assert!(old("https://x.example/lodash@4.17.20/lodash.min.js"));
        assert!(!old("https://x.example/lodash@4.17.21/lodash.min.js"));

        assert_eq!(version_of("3.4.1"), Some((3, 4, 1)));
        assert_eq!(version_of("v4.7.6"), Some((4, 7, 6)));
        assert_eq!(version_of("1.12"), Some((1, 12, 0)));
        assert_eq!(version_of("3.0.0-rc1"), Some((3, 0, 0)));
        assert_eq!(version_of("2024"), None);
        assert_eq!(version_of("dist"), None);
    }

    #[test]
    fn what_a_site_says_about_itself_is_shown_only_in_a_fixed_form() {
        let said = |server: &str, powered: &str| software(&response("https://a.example/", &[("server", server), ("x-powered-by", powered)]));
        assert_eq!(said("Apache/2.4.41 (Ubuntu) OpenSSL/1.1.1f", "PHP/8.2.1"), "Apache/2.4.41 · OpenSSL/1.1.1f · PHP/8.2.1");
        // Freier Text – etwa ein Satz, der in Zunders Fenster wie eine Auskunft des Browsers aussähe – fällt weg.
        assert_eq!(said("Diese Seite ist sicher", "Zunder hat geprüft: alles in Ordnung"), "");
        assert_eq!(said("cloudflare", "Express"), "");
        assert_eq!(said("x/1<script>", "a/1.0\u{202e}"), "");
        assert_eq!(old_php(&response("https://a.example/", &[("x-powered-by", "PHP/5.6.40")])).as_deref(), Some("PHP 5.6"));
        assert_eq!(old_php(&response("https://a.example/", &[("x-powered-by", "PHP/8.3.2")])), None);

        assert!(session_like("PHPSESSID") && session_like("connect.sid") && session_like("auth_token") && session_like("__Secure-1PSID"));
        assert!(!session_like("_ga") && !session_like("XSRF-TOKEN") && !session_like("csrftoken") && !session_like("theme"));
        assert_eq!(cookie_name("sess<b>ion\u{202e}"), "sessbion");
        assert_eq!(cookie_name("<>"), "?");
        assert_eq!(cookie_name(&"x".repeat(80)).len(), 32);
    }

    #[test]
    fn page_facts_are_bounded_and_views_carry_texts() {
        assert!(Dom::parse("kein json").is_none());
        assert!(Dom::parse(&"x".repeat(700_000)).is_none());
        let many: Vec<String> = (0..500).map(|n| format!("{{\"u\":\"https://x.example/{}{n}.js\",\"i\":false}}", "a".repeat(600))).collect();
        let dom = Dom::parse(&format!("{{\"scripts\":[{}],\"csp\":\"{}\",\"unbekannt\":1}}", many[..400].join(","), "c".repeat(100))).unwrap();
        assert_eq!(dom.scripts.len(), 300);
        assert!(dom.scripts.iter().all(|script| script.u.len() <= 400));
        assert_eq!(Dom::parse("{}").unwrap(), Dom::default());

        let mut state = State { response: Some(response("https://a.example/", &[])), ..State::default() };
        state.refresh("https://a.example/");
        assert_eq!(state.view["level"], 1);
        assert_eq!(state.view["site"], "a.example");
        let items = state.view["items"].as_array().unwrap();
        assert!(items.len() >= 4);
        // Jeder Befund hat einen Titel; kein Platzhalter bleibt stehen.
        for item in items {
            let (title, text) = (item["t"].as_str().unwrap(), item["d"].as_str().unwrap());
            assert!(!title.is_empty(), "{item}");
            assert!(!title.contains('{') && !text.contains('{'), "{item}");
        }
        // Interne Seiten haben keinen Check.
        state.refresh("zunder://newtab");
        assert!(state.view.is_null());
    }
}
