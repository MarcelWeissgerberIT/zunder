//! Sicherheitscheck einer Website – für die gleichnamige eingebaute App.
//!
//! Anders als der Seitencheck (`sitecheck.rs`), der nur auswertet, was beim
//! gewöhnlichen Laden ohnehin anfällt, ruft diese App eine Adresse **eigens**
//! ab, um sie zu prüfen – gedacht, damit man die eigenen Seiten härten kann.
//! Abgerufen wird vom Browser selbst (hier, mit `ureq`), nicht von der Seite der
//! App: Die geht gar nicht ins Netz. So liest Zunder den ganzen Antwortkopf,
//! die Cookies und den Quelltext – mehr, als eine Seite im Tab über sich preisgibt.
//!
//! Gefahrlos heißt: Es werden nur **Abrufe** gemacht (GET), nichts
//! eingeschleust, nichts erraten-durchprobiert, keine Last erzeugt. Die „aktiven“
//! Prüfungen rufen eine Handvoll bekannter Pfade ab (`/.git/config`, `/.env` …)
//! und sehen nach, ob eine Adresse von `http` auf `https` führt und ob sie
//! fremde Herkünfte beim Teilen (CORS) wahllos zulässt. Das tut jeder
//! Sicherheits-Scanner; einen Angriff stellt es nicht dar.
//!
//! Die eigentliche Bewertung macht `sitecheck::assess` – dieselbe wie hinter der
//! Security-Ampel. Hier wird nur abgerufen, der Quelltext gelesen und um ein
//! paar Befunde ergänzt, die erst der eigene Abruf sichtbar macht.

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use crate::sitecheck::{assess, Cookie, Dom, DomForm, DomFrame, DomMixed, DomScript, Facts, Finding, Level, Response, HEADERS};

/// So viel vom Quelltext wird gelesen (Bytes). Mehr braucht die Prüfung nicht.
const BODY_MAX: u64 = 1_500_000;
/// So viel von einer abgeklopften Datei wird gelesen – nur der Anfang zählt.
const PROBE_MAX: u64 = 4_096;

/// So viele Prüfungen laufen höchstens gleichzeitig. Nur die App selbst stößt
/// sie an (und serialisiert sie schon), das hier ist die Grenze zur Sicherheit.
const MAX_SCANS: usize = 3;
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Meldet eine beginnende Prüfung an – `false`, wenn schon zu viele laufen.
/// Wer `true` bekommt, ruft danach `finish`.
pub fn begin() -> bool {
    let mut count = RUNNING.load(Ordering::Relaxed);
    loop {
        if count >= MAX_SCANS {
            return false;
        }
        match RUNNING.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(now) => count = now,
        }
    }
}

pub fn finish() {
    RUNNING.fetch_sub(1, Ordering::AcqRel);
}

/// Prüft die Adresse und liefert das Ergebnis, fertig für die Seite der App.
/// Läuft im Hintergrund (ein eigener Thread) – `ureq` wartet auf das Netz.
pub fn scan(input: &str) -> Value {
    let Some(url) = clean_url(input) else {
        return json!({ "state": "bad_url" });
    };

    // Der Hauptabruf: Weiterleitungen folgt er (bis zu fünf), damit die
    // Bewertung die Adresse trifft, auf der die Seite wirklich sitzt.
    let main = match fetch(&url, 5) {
        Ok(page) => page,
        Err(reason) => return json!({ "state": "unreachable", "why": reason }),
    };
    let final_url = main.url.clone();
    let https = final_url.to_ascii_lowercase().starts_with("https://");
    let host = crate::shield::host_of(&final_url);
    let site = crate::sites::domain_of(&final_url).unwrap_or_else(|| host.clone());

    let dom = read_dom(&main.body, https);
    let response = response_of(&final_url, &main.headers);
    let cookies = main.cookies();
    let only_secure = (https && mixed_count(&dom) == 0).then_some(true).or(if https { Some(false) } else { None });

    let facts = Facts {
        url: &final_url,
        response: Some(&response),
        dom: Some(&dom),
        cookies: Some(&cookies),
        only_secure,
        legacy_tls: false, // ureq/rustls nennt die Fassung nicht – das bleibt offen.
        dom_failed: false,
    };
    let mut report = assess(&facts);

    // --- Befunde, die erst der eigene Abruf zeigt --------------------------
    extra_findings(&mut report.findings, &url, &final_url, &main, https, &host);
    // Das Ernste zuerst; innerhalb einer Stufe bleibt die Reihenfolge.
    report.findings.sort_by_key(|finding| finding.level);

    let mut view = report.view(&site);
    if let Some(fields) = view.as_object_mut() {
        fields.insert("state".into(), json!("ok"));
        fields.insert("final".into(), json!(final_url));
        fields.insert("status".into(), json!(main.status));
        fields.insert("redirected".into(), json!(!same_address(&url, &final_url)));
    }
    view
}

/// Macht aus der Eingabe eine abrufbare Adresse: nur `http`/`https`, ohne
/// Zugangsdaten, mit Rechnernamen. Ohne Schema wird `https` angenommen.
fn clean_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 2_000 {
        return None;
    }
    let with_scheme = if trimmed.contains("://") { trimmed.to_string() } else { format!("https://{trimmed}") };
    let parsed = url::Url::parse(&with_scheme).ok()?;
    let plain = matches!(parsed.scheme(), "http" | "https") && parsed.username().is_empty() && parsed.password().is_none();
    (plain && parsed.host_str().is_some()).then(|| parsed.to_string())
}

/// Zwei Adressen, die auf dasselbe zeigen (bis auf einen Schrägstrich am Ende).
fn same_address(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

// ---------------------------------------------------------------------------
// Abruf
// ---------------------------------------------------------------------------

struct Page {
    status: u16,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Page {
    fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }

    fn cookies(&self) -> Vec<Cookie> {
        self.headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .filter_map(|(_, value)| cookie_of(value))
            .take(50)
            .collect()
    }
}

fn agent(redirects: u32) -> ureq::Agent {
    // Knappe Fristen: Selbst alle Prüfungen zusammen bleiben unter der Zeit, die
    // die Seite auf das Ergebnis wartet (`ui/apps.js`, `scan`).
    let whole = if redirects > 0 { 14 } else { 6 };
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(8))
        .timeout_write(Duration::from_secs(5))
        .timeout(Duration::from_secs(whole))
        .redirects(redirects)
        // Ein gewöhnlicher Browser-Kennzeichner: Manche Seiten antworten einem
        // namenlosen Abruf gar nicht. Nichts darin verrät den Menschen am Browser.
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Zunder/1.0 Safari/605.1.15")
        .build()
}

/// Ruft eine Adresse ab und liest Kopf und (gedeckelt) Quelltext. Eine Antwort
/// mit Fehler-Status (403, 500 …) ist trotzdem eine Antwort – ihr Kopf wird
/// mitgelesen. Nur wenn gar nichts zurückkommt, ist es ein Fehler.
fn fetch(url: &str, redirects: u32) -> Result<Page, &'static str> {
    read_page(agent(redirects).get(url).call(), BODY_MAX)
}

/// Wie `fetch`, aber mit einer zusätzlichen Kopfzeile (für die CORS-Prüfung)
/// und ohne Weiterleitungen (für das Abklopfen bekannter Pfade).
fn fetch_with(url: &str, origin: Option<&str>) -> Result<Page, &'static str> {
    let mut request = agent(0).get(url);
    if let Some(origin) = origin {
        request = request.set("Origin", origin);
    }
    // Beim Abklopfen genügt der Anfang der Datei.
    read_page(request.call(), PROBE_MAX)
}

fn read_page(result: Result<ureq::Response, ureq::Error>, body_max: u64) -> Result<Page, &'static str> {
    let response = match result {
        Ok(response) => response,
        // Ein Fehler-Status trägt seinen Kopf; den wollen wir sehen.
        Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(_)) => return Err("transport"),
    };
    let status = response.status();
    let url = response.get_url().to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    for name in response.headers_names() {
        // `all` fängt mehrfach gesetzte Zeilen (etwa mehrere Set-Cookie) ein.
        for value in response.all(&name) {
            headers.push((name.clone(), value.to_string()));
            if headers.len() >= 200 {
                break;
            }
        }
    }
    let is_text = response
        .header("content-type")
        .is_none_or(|value| value.contains("text") || value.contains("html") || value.contains("xml") || value.contains("json"));
    let mut body = String::new();
    if is_text {
        let mut raw: Vec<u8> = Vec::new();
        let _ = response.into_reader().take(body_max).read_to_end(&mut raw);
        body = String::from_utf8_lossy(&raw).into_owned();
    }
    Ok(Page { status, url, headers, body })
}

// ---------------------------------------------------------------------------
// Antwortkopf und Cookies in die Form bringen, die `sitecheck` erwartet
// ---------------------------------------------------------------------------

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
}

fn response_of(url: &str, headers: &[(String, String)]) -> Response {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for &name in HEADERS {
        if let Some(value) = header(headers, name) {
            out.push((name, value.to_string()));
        }
    }
    Response { url: url.to_string(), headers: out }
}

fn cookie_of(set: &str) -> Option<Cookie> {
    let mut parts = set.split(';');
    let first = parts.next()?.trim();
    let name = first.split('=').next()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let (mut secure, mut http_only, mut same_site) = (false, false, String::new());
    for attr in parts {
        let attr = attr.trim();
        if attr.eq_ignore_ascii_case("secure") {
            secure = true;
        } else if attr.eq_ignore_ascii_case("httponly") {
            http_only = true;
        } else if let Some((key, value)) = attr.split_once('=') {
            if key.trim().eq_ignore_ascii_case("samesite") {
                same_site = value.trim().to_ascii_lowercase();
            }
        }
    }
    Some(Cookie { name, secure, http_only, same_site })
}

// ---------------------------------------------------------------------------
// Quelltext lesen: dieselben Merkmale, die `ui/check.js` aus dem DOM liest
// ---------------------------------------------------------------------------

fn mixed_count(dom: &Dom) -> u32 {
    dom.mixed.active + dom.mixed.passive
}

/// Liest aus dem Quelltext der Seite, was die Bewertung über ihren Aufbau wissen
/// will: Skripte (Adresse, Prüfsumme), Formulare (Ziel), Rahmen (Adresse,
/// `sandbox`), Passwortfelder, eine `<meta>`-CSP und unverschlüsselt geladene
/// Teile. Kein echter Parser – ein genügsamer, gedeckelter Durchgang.
fn read_dom(body: &str, https: bool) -> Dom {
    let mut dom = Dom::default();
    let mut mixed = 0u32;
    for tag in tags(body) {
        // Schluss-Tags (`</form>`) tragen nichts bei – nur die Start-Tags zählen.
        if tag.starts_with('/') {
            continue;
        }
        let name = tag_name(&tag);
        match name.as_str() {
            "script" => {
                if let Some(src) = attr(&tag, "src") {
                    if dom.scripts.len() < 300 {
                        dom.scripts.push(DomScript { i: has_attr(&tag, "integrity"), u: src.clone() });
                    }
                    if https && is_plain_http(&src) {
                        mixed += 1;
                    }
                }
            }
            "form" => {
                if dom.forms.len() < 60 {
                    dom.forms.push(DomForm { a: attr(&tag, "action").unwrap_or_default() });
                }
            }
            "iframe" => {
                if let Some(src) = attr(&tag, "src") {
                    if dom.frames.len() < 120 {
                        dom.frames.push(DomFrame { s: has_attr(&tag, "sandbox"), u: src.clone() });
                    }
                    if https && is_plain_http(&src) {
                        mixed += 1;
                    }
                }
            }
            "input" => {
                if attr(&tag, "type").is_some_and(|kind| kind.eq_ignore_ascii_case("password")) {
                    dom.passwords = dom.passwords.saturating_add(1);
                }
            }
            "meta" => {
                if attr(&tag, "http-equiv").is_some_and(|value| value.eq_ignore_ascii_case("content-security-policy")) {
                    if let Some(content) = attr(&tag, "content") {
                        if dom.csp.is_empty() {
                            dom.csp = content;
                        }
                    }
                }
            }
            "img" | "audio" | "video" | "source" | "embed" | "track" => {
                if https {
                    if let Some(src) = attr(&tag, "src") {
                        if is_plain_http(&src) {
                            mixed += 1;
                        }
                    }
                }
            }
            "link" => {
                let loads = attr(&tag, "rel").is_some_and(|rel| {
                    let rel = rel.to_ascii_lowercase();
                    rel.contains("stylesheet") || rel.contains("preload") || rel.contains("prefetch") || rel.contains("icon")
                });
                if https && loads {
                    if let Some(href) = attr(&tag, "href") {
                        if is_plain_http(&href) {
                            mixed += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if dom.csp.len() > 4_000 {
        dom.csp.truncate(4_000);
    }
    // Aktiv/passiv trennt die Bewertung nicht – sie sieht nur auf die Summe.
    dom.mixed = DomMixed { active: 0, passive: mixed.min(9_999) };
    dom
}

fn is_plain_http(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("http://") && !crate::shield::is_local_host(&crate::shield::host_of(url))
}

/// Die Rümpfe aller Start-Tags im Quelltext (ohne `<` und `>`). Anführungszeichen
/// werden geachtet, damit ein `>` in einem Attribut das Tag nicht früh beendet.
/// Gedeckelt, damit ein riesiger Quelltext die Prüfung nicht aufhält.
fn tags(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() && out.len() < 4_000 {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        // Kommentare überspringen.
        if bytes[index..].starts_with(b"<!--") {
            index += body[index..].find("-->").map_or(bytes.len() - index, |end| end + 3);
            continue;
        }
        let start = index + 1;
        let mut end = start;
        let mut quote = 0u8;
        while end < bytes.len() {
            let byte = bytes[end];
            if quote != 0 {
                if byte == quote {
                    quote = 0;
                }
            } else if byte == b'"' || byte == b'\'' {
                quote = byte;
            } else if byte == b'>' {
                break;
            }
            end += 1;
        }
        if start < end && end <= bytes.len() {
            out.push(body[start..end].to_string());
        }
        index = end + 1;
    }
    out
}

fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn has_attr(tag: &str, name: &str) -> bool {
    attr_find(tag, name).is_some()
}

/// Der Wert eines Attributs (`name="…"`, `name='…'` oder `name=…`), ohne Rücksicht
/// auf Groß- und Kleinschreibung des Namens. Höchstens 400 Zeichen.
fn attr(tag: &str, name: &str) -> Option<String> {
    let rest = attr_find(tag, name)?;
    let rest = rest.trim_start();
    let value = match rest.strip_prefix('=') {
        None => return Some(String::new()), // Attribut ohne Wert.
        Some(after) => {
            let after = after.trim_start();
            if let Some(inner) = after.strip_prefix('"') {
                inner.split('"').next().unwrap_or("")
            } else if let Some(inner) = after.strip_prefix('\'') {
                inner.split('\'').next().unwrap_or("")
            } else {
                after.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("")
            }
        }
    };
    let mut value = value.trim().to_string();
    if value.len() > 400 {
        let mut end = 400;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    Some(value)
}

/// Findet das Attribut und gibt den Rest hinter seinem Namen zurück (beginnt oft
/// mit `=`). Der Name muss an einer Wortgrenze stehen – `data-src` ist nicht `src`.
fn attr_find<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let needle = name.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(at) = lower[from..].find(&needle) {
        let at = from + at;
        let before_ok = at == 0 || !matches!(lower.as_bytes()[at - 1], b'-' | b'a'..=b'z' | b'0'..=b'9' | b'_');
        let after = at + needle.len();
        let after_ok = after >= lower.len() || matches!(lower.as_bytes()[after], b'=' | b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>');
        if before_ok && after_ok {
            return Some(&tag[after..]);
        }
        from = at + needle.len();
    }
    None
}

// ---------------------------------------------------------------------------
// Befunde, die erst der eigene Abruf zeigt (sichere, aktive Prüfungen)
// ---------------------------------------------------------------------------

/// Bekannte Pfade, die nie offen im Netz stehen sollten – und woran man die
/// echte Datei erkennt (nicht eine „200“-Auffangseite). Nur ein Abruf je Pfad.
const EXPOSED: &[(&str, &str)] = &[
    (".git/config", "[core]"),
    (".git/HEAD", "ref:"),
    // „ENV“ ist ein Kennwort für `looks_exposed`: eine echte Zuweisung, nicht
    // bloß ein Gleichheitszeichen irgendwo (sonst meldete sich jede JSON-Antwort).
    (".env", "ENV"),
    ("server-status", "Apache Server Status"),
];

fn extra_findings(found: &mut Vec<Finding>, requested: &str, final_url: &str, main: &Page, https: bool, host: &str) {
    let local = crate::shield::is_local_host(host);

    // Führt die unverschlüsselte Adresse von selbst auf https? (Nur prüfen, wenn
    // die geprüfte Adresse https ist und nicht im eigenen Netz liegt.)
    if https && !local && requested.to_ascii_lowercase().starts_with("https://") {
        if let Some(origin) = origin_of(final_url) {
            let plain = origin.replacen("https://", "http://", 1);
            if let Ok(page) = fetch_with(&plain, None) {
                let moved = (300..400).contains(&page.status)
                    && page.header("location").is_some_and(|to| to.trim().to_ascii_lowercase().starts_with("https://"));
                if !moved && page.status < 400 {
                    found.push(note(Level::Weak, "no_https_redirect"));
                }
            }
        }
    }

    // CORS: Spiegelt die Seite eine beliebige fremde Herkunft wider? Mit
    // „Credentials“ ist das ernst – fremde Seiten könnten dann angemeldet lesen.
    let probe = "https://zunder-cors-probe.example";
    if let Ok(page) = fetch_with(final_url, Some(probe)) {
        if let Some(allow) = page.header("access-control-allow-origin") {
            let allow = allow.trim();
            let credentials = page.header("access-control-allow-credentials").is_some_and(|value| value.trim().eq_ignore_ascii_case("true"));
            if allow == probe && credentials {
                found.push(note(Level::Serious, "cors_credentials"));
            } else if allow == probe {
                found.push(note(Level::Weak, "cors_any"));
            } else if allow == "*" && credentials {
                // Browser verbieten diese Kombination zwar – als Absicht ist sie dennoch ein Warnzeichen.
                found.push(note(Level::Hint, "cors_star"));
            }
        }
    }

    // Offen liegende Dateien – aber nur bei einer echten Website, nie im eigenen Netz.
    if !local {
        let base = origin_of(final_url).unwrap_or_default();
        let mut open: Vec<String> = Vec::new();
        for (path, sign) in EXPOSED {
            if open.len() >= 4 {
                break;
            }
            let address = format!("{}/{}", base.trim_end_matches('/'), path);
            if let Ok(page) = fetch_with(&address, None) {
                if looks_exposed(&page, sign) {
                    open.push((*path).to_string());
                }
            }
        }
        if !open.is_empty() {
            found.push(note(Level::Serious, "exposed").with("list", open.join(", ")));
        }
    }

    // Verzeichnis-Auflistung: Der Server zeigt den Inhalt eines Ordners.
    let body_lower = main.body.get(..main.body.len().min(60_000)).unwrap_or(&main.body).to_ascii_lowercase();
    if body_lower.contains("<title>index of /") || body_lower.contains(">directory listing for") {
        found.push(note(Level::Weak, "dir_listing"));
    }

    // Neuere Trennschichten, die der Seitencheck (noch) nicht prüft – als Hinweis.
    if https && !local {
        if main.header("cross-origin-opener-policy").is_none() {
            found.push(note(Level::Hint, "no_coop"));
        }
        if main.header("permissions-policy").is_none() {
            found.push(note(Level::Hint, "no_permissions"));
        }
    }
}

fn note(level: Level, id: &'static str) -> Finding {
    Finding::note(level, id)
}

/// Die Herkunft einer Adresse: `https://host[:port]`.
fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let scheme = parsed.scheme();
    match parsed.port() {
        Some(port) => Some(format!("{scheme}://{host}:{port}")),
        None => Some(format!("{scheme}://{host}")),
    }
}

/// Sieht die Antwort auf einen bekannten Pfad nach der echten Datei aus – und
/// nicht nach einer Seite, die der Server auf alles mit „200“ ausliefert?
fn looks_exposed(page: &Page, sign: &str) -> bool {
    if page.status != 200 {
        return false;
    }
    let body = page.body.trim_start();
    if body.is_empty() {
        return false;
    }
    // Eine HTML-Seite ist fast immer die Auffangseite, nicht die rohe Datei.
    let head = body.get(..body.len().min(15)).unwrap_or(body).to_ascii_lowercase();
    let html = page.header("content-type").is_some_and(|value| value.contains("html"))
        || head.starts_with("<!doctype")
        || head.starts_with("<html")
        || head.starts_with("<?xml");
    if html {
        return false;
    }
    // Die Datei muss ihr erwartetes Kennzeichen tragen – „ENV“ heißt: wenigstens
    // eine Zeile ist eine echte Umgebungs-Zuweisung (NAME=Wert). Dort zählt auch
    // eine Antwort, die wie JSON aussieht, als Auffangseite (nicht als `.env`).
    if sign == "ENV" {
        !head.starts_with('{') && !head.starts_with('[') && page.body.lines().take(200).any(env_assignment)
    } else {
        page.body.contains(sign)
    }
}

/// Sieht eine Zeile nach `NAME=…` aus (Großbuchstaben, Ziffern, Unterstrich)?
fn env_assignment(line: &str) -> bool {
    let line = line.trim_start();
    let name: String = line.chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && line[name.len()..].trim_start().starts_with('=')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adressen_werden_geprueft_und_ergaenzt() {
        assert_eq!(clean_url("example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(clean_url("http://example.com/x").as_deref(), Some("http://example.com/x"));
        assert_eq!(clean_url("  https://a.de/p?q=1 ").as_deref(), Some("https://a.de/p?q=1"));
        for bad in ["", "   ", "ftp://a.de/", "javascript:alert(1)", "https://u:p@a.de/", "file:///etc/passwd", "no spaces here"] {
            assert_eq!(clean_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn quelltext_wird_gelesen() {
        let body = r#"
            <!doctype html><html><head>
            <meta http-equiv="Content-Security-Policy" content="default-src 'self'">
            <script src="https://cdn.example/a.js"></script>
            <script src="https://cdn.example/b.js" integrity="sha384-x"></script>
            <script src="http://evil.example/c.js"></script>
            <link rel="stylesheet" href="http://evil.example/s.css">
            <img src="http://evil.example/p.png">
            </head><body>
            <form action="http://evil.example/post"><input type="text"><input Type='PASSWORD'></form>
            <iframe src="https://widget.example/w" sandbox></iframe>
            <!-- <script src="http://ignored.example/x.js"></script> -->
            </body></html>"#;
        let dom = read_dom(body, true);
        assert_eq!(dom.scripts.len(), 3);
        assert!(dom.scripts.iter().any(|s| s.u.contains("b.js") && s.i));
        assert!(dom.scripts.iter().any(|s| s.u.contains("a.js") && !s.i));
        assert_eq!(dom.forms.len(), 1);
        assert_eq!(dom.forms[0].a, "http://evil.example/post");
        assert_eq!(dom.frames.len(), 1);
        assert!(dom.frames[0].s);
        assert_eq!(dom.passwords, 1);
        assert_eq!(dom.csp, "default-src 'self'");
        // Unverschlüsselt geladen: Skript, Stil, Bild (der Kommentar zählt nicht).
        assert_eq!(mixed_count(&dom), 3);
    }

    #[test]
    fn attribute_sauber_gelesen() {
        let tag = r#"script src = "https://a.example/x.js?y=1&z=2"  defer integrity=sha384-abc data-src="nope""#;
        assert_eq!(attr(tag, "src").as_deref(), Some("https://a.example/x.js?y=1&z=2"));
        assert_eq!(attr(tag, "integrity").as_deref(), Some("sha384-abc"));
        assert!(has_attr(tag, "defer"));
        assert!(!has_attr(tag, "async"));
        // „src“ darf nicht in „data-src“ gefunden werden.
        assert_eq!(attr("div data-role=x", "role"), None);
        // Ein Attribut ohne Wert.
        assert_eq!(attr("iframe sandbox", "sandbox").as_deref(), Some(""));
    }

    #[test]
    fn cookies_aus_dem_kopf() {
        let cookie = cookie_of("sid=abc123; Path=/; Secure; HttpOnly; SameSite=Lax").unwrap();
        assert_eq!(cookie.name, "sid");
        assert!(cookie.secure && cookie.http_only);
        assert_eq!(cookie.same_site, "lax");
        let loose = cookie_of("theme=dark; Path=/").unwrap();
        assert!(!loose.secure && !loose.http_only && loose.same_site.is_empty());
        assert_eq!(cookie_of("   ; Path=/"), None);
    }

    #[test]
    fn offene_dateien_nur_echt_erkannt() {
        let page = |status, ctype: &str, body: &str| Page {
            status,
            url: "https://a.de/.git/config".into(),
            headers: vec![("content-type".into(), ctype.into())],
            body: body.into(),
        };
        assert!(looks_exposed(&page(200, "text/plain", "[core]\n\trepositoryformatversion = 0"), "[core]"));
        // Auffangseite: 200, aber HTML – keine echte Datei.
        assert!(!looks_exposed(&page(200, "text/html", "<!doctype html><html>…"), "[core]"));
        // Da, aber ohne das erwartete Kennzeichen.
        assert!(!looks_exposed(&page(200, "text/plain", "nichts dergleichen"), "[core]"));
        // Nicht gefunden.
        assert!(!looks_exposed(&page(404, "text/html", "weg"), "[core]"));
        // „.env“: nur eine echte Zuweisung zählt, nicht irgendein Gleichheitszeichen.
        assert!(looks_exposed(&page(200, "text/plain", "# Kommentar\nDB_PASSWORD=geheim\nPORT=3000"), "ENV"));
        assert!(!looks_exposed(&page(200, "application/json", "{\"a\":\"b=c\"}"), "ENV"));
        assert!(!looks_exposed(&page(200, "text/plain", "preis = 5 euro"), "ENV"));
        assert!(env_assignment("API_KEY=abc") && env_assignment("  PORT = 3000") && env_assignment("_X=1"));
        assert!(!env_assignment("hallo=welt") && !env_assignment("=leer") && !env_assignment("# nur kommentar"));
        // Eine kurze, nicht-HTML-Antwort ohne Kennzeichen bleibt unauffällig.
        assert!(!looks_exposed(&page(200, "text/plain", "ok"), "[core]"));
    }

    #[test]
    fn herkunft_einer_adresse() {
        assert_eq!(origin_of("https://a.de/pfad?x=1").as_deref(), Some("https://a.de"));
        assert_eq!(origin_of("http://a.de:8080/x").as_deref(), Some("http://a.de:8080"));
        assert_eq!(origin_of("nicht"), None);
    }
}
