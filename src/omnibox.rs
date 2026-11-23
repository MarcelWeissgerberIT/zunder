//! Adressleiste: Eingaben deuten (Adresse oder Suche), Suchmaschinen,
//! Suchvorschläge und die Kurzform einer Adresse für die Anzeige.

use std::time::Duration;

pub struct Engine {
    pub id: &'static str,
    pub name: &'static str,
    pub search: &'static str,
    pub suggest: Option<&'static str>,
}

pub const ENGINES: &[Engine] = &[
    Engine {
        id: "google",
        name: "Google",
        search: "https://www.google.com/search?q={}",
        suggest: Some("https://suggestqueries.google.com/complete/search?client=firefox&ie=UTF-8&oe=UTF-8&q={}"),
    },
    Engine {
        id: "duckduckgo",
        name: "DuckDuckGo",
        search: "https://duckduckgo.com/?q={}",
        suggest: Some("https://duckduckgo.com/ac/?type=list&q={}"),
    },
    Engine {
        id: "bing",
        name: "Bing",
        search: "https://www.bing.com/search?q={}",
        suggest: Some("https://api.bing.com/osjson.aspx?query={}"),
    },
    Engine {
        id: "ecosia",
        name: "Ecosia",
        search: "https://www.ecosia.org/search?q={}",
        suggest: Some("https://ac.ecosia.org/autocomplete?type=list&q={}"),
    },
    Engine {
        id: "brave",
        name: "Brave Search",
        search: "https://search.brave.com/search?q={}",
        suggest: Some("https://search.brave.com/api/suggest?q={}"),
    },
    Engine {
        id: "startpage",
        name: "Startpage",
        search: "https://www.startpage.com/sp/search?query={}",
        suggest: None,
    },
];

pub fn engine(id: &str) -> &'static Engine {
    ENGINES.iter().find(|e| e.id == id).unwrap_or(&ENGINES[0])
}

fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

pub fn search_url(engine: &Engine, query: &str) -> String {
    engine.search.replace("{}", &encode(query.trim()))
}

/// Endungen, die nur im eigenen Netz vorkommen – dort gibt es selten TLS.
const LOCAL_TLDS: &[&str] = &[
    "local", "lan", "internal", "test", "localhost", "home", "corp", "intranet", "fritz", "box",
];

/// Vergebene Länderendungen. Damit bleibt "node.js" eine Suche, "example.de" eine Adresse.
const COUNTRY_TLDS: &str = " ac ad ae af ag ai al am ao aq ar as at au aw ax az ba bb bd be bf bg bh bi bj bm bn \
bo br bs bt bw by bz ca cc cd cf cg ch ci ck cl cm cn co cr cu cv cw cx cy cz de dj dk dm do dz ec ee eg er es et \
eu fi fj fk fm fo fr ga gd ge gf gg gh gi gl gm gn gp gq gr gs gt gu gw gy hk hm hn hr ht hu id ie il im in io iq \
ir is it je jm jo jp ke kg kh ki km kn kp kr kw ky kz la lb lc li lk lr ls lt lu lv ly ma mc md me mg mh mk ml mm \
mn mo mp mq mr ms mt mu mv mw mx my mz na nc ne nf ng ni nl no np nr nu nz om pa pe pf pg ph pk pl pm pn pr ps pt \
pw py qa re ro rs ru rw sa sb sc sd se sg sh si sk sl sm sn so sr ss st su sv sx sy sz tc td tf tg th tj tk tl tm \
tn to tr tt tv tw tz ua ug uk us uy uz va vc ve vg vi vn vu wf ws ye yt za zm zw ";

/// Gängige Endungen mit mehr als zwei Buchstaben.
const KNOWN_TLDS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "int", "info", "biz", "name", "pro", "app", "dev",
    "page", "web", "blog", "shop", "store", "online", "site", "tech", "cloud", "xyz", "top", "club",
    "live", "news", "media", "email", "wiki", "world", "today", "life", "work", "works", "tools",
    "zone", "space", "fun", "games", "game", "art", "design", "photo", "photos", "video", "music",
    "studio", "agency", "company", "solutions", "services", "digital", "network", "systems",
    "software", "codes", "code", "academy", "school", "education", "university", "institute",
    "travel", "hotel", "restaurant", "cafe", "bar", "pizza", "food", "health", "care", "clinic",
    "doctor", "law", "legal", "finance", "bank", "money", "fund", "capital", "insurance", "estate",
    "house", "homes", "garden", "auto", "car", "cars", "bike", "berlin", "hamburg", "koeln",
    "cologne", "bayern", "nrw", "ruhr", "saarland", "wien", "tirol", "swiss", "zuerich", "london",
    "paris", "nyc", "tokyo", "africa", "asia", "gmbh", "ltd", "inc", "llc", "group", "team",
    "community", "social", "chat", "forum", "help", "support", "guide", "how", "link", "click",
    "one", "plus", "vip", "gold", "red", "blue", "green", "black", "pink", "moe", "icu", "cyou",
    "sbs", "mobi", "tel", "jobs", "museum", "coop", "aero", "arpa", "onion", "science", "energy",
    "earth", "eco", "bio", "farm", "run", "fit", "yoga", "film", "movie", "show", "radio", "audio",
    "press", "report", "review", "reviews", "events", "tickets", "market", "markets", "trade",
    "exchange", "ventures", "partners", "holdings", "international", "global", "center", "city",
    "town", "land", "immo", "immobilien", "reise", "reisen", "kaufen", "versicherung", "schule",
    "haus", "jetzt", "gratis", "expert", "consulting", "engineering", "technology", "computer",
    "hosting", "host", "domains", "website", "sale", "deals", "direct", "express", "delivery",
    "bot", "new", "day", "zip", "mov", "foo", "ing", "meme", "data", "search", "google", "youtube",
    "apple", "amazon", "microsoft", "sap", "bmw", "audi", "bosch",
];

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label.chars().all(|c| c.is_alphanumeric() || c == '-')
}

fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.parse::<u8>().is_ok())
}

/// Liefert das Schema, mit dem ein eingetippter Host ohne Schema geöffnet wird.
fn scheme_for_host(hostport: &str, has_path: bool) -> Option<&'static str> {
    if hostport.contains('@') {
        return None; // E-Mail-Adressen sind eine Suche
    }
    if hostport.starts_with('[') {
        return hostport.contains(']').then_some("http");
    }
    let (host, has_port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, true),
        Some(_) => return None,
        None => (hostport, false),
    };
    let host = host.strip_suffix('.').unwrap_or(host).to_lowercase();
    if host == "localhost" || is_ipv4(&host) {
        return Some("http");
    }
    let labels: Vec<&str> = host.split('.').collect();
    if !labels.iter().all(|l| valid_label(l)) {
        return None;
    }
    if labels.len() == 1 {
        // Ein einzelnes Wort ist nur mit Port oder Schrägstrich eine Adresse ("nas:8080", "router/").
        return (has_port || has_path).then_some("http");
    }
    let tld = *labels.last().unwrap();
    if LOCAL_TLDS.contains(&tld) {
        return Some("http");
    }
    let alphabetic = tld.chars().all(|c| c.is_alphabetic());
    if tld.starts_with("xn--") || (alphabetic && !tld.is_ascii()) {
        return Some("https");
    }
    let country = tld.len() == 2 && COUNTRY_TLDS.contains(&format!(" {tld} "));
    if alphabetic && (country || KNOWN_TLDS.contains(&tld)) {
        return Some("https");
    }
    None
}

/// Deutet die Eingabe als Adresse. `None`, wenn sie eher eine Suche ist.
pub fn as_url(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();

    for scheme in ["http://", "https://"] {
        if lower.starts_with(scheme) {
            let parsed = url::Url::parse(s).ok()?;
            return parsed.host_str().is_some().then(|| parsed.to_string());
        }
    }
    if lower == "about:blank" {
        return Some("about:blank".into());
    }
    for scheme in ["file://", "data:"] {
        if lower.starts_with(scheme) {
            return url::Url::parse(s).ok().map(|u| u.to_string());
        }
    }
    // Von den internen Seiten lässt sich nur die Startseite eintippen.
    if lower.starts_with("zunder://") {
        return is_newtab(&lower).then(|| NEWTAB.to_string());
    }
    if s.contains(char::is_whitespace) {
        return None;
    }

    let end = s.find(['/', '?', '#']).unwrap_or(s.len());
    let hostport = &s[..end];
    let scheme = scheme_for_host(hostport, end < s.len())?;
    let parsed = url::Url::parse(&format!("{scheme}://{s}")).ok()?;
    parsed.host_str().is_some().then(|| parsed.to_string())
}

/// Adresse, die für eine Eingabe geladen wird: die Seite selbst oder die Suche danach.
pub fn resolve(input: &str, engine: &Engine) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    Some(as_url(s).unwrap_or_else(|| search_url(engine, s)))
}

pub const NEWTAB: &str = "zunder://newtab/";
pub const NEWTAB_PRIVATE: &str = "zunder://newtab/?private=1";

pub fn is_newtab(url: &str) -> bool {
    url.starts_with("zunder://newtab")
}

/// Adresse hinter einer Fehlerseite (`zunder://error/?u=…`).
pub fn error_target(url: &str) -> Option<String> {
    if !url.starts_with("zunder://error") {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    let target = parsed.query_pairs().find(|(k, _)| k == "u").map(|(_, v)| v.into_owned())?;
    // Hinter einer Fehlerseite steht immer eine Webadresse – nichts anderes
    // wird von dort wieder geladen (Neu laden, Tab duplizieren, Sitzung).
    let web = url::Url::parse(&target).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
    web.then_some(target)
}

/// Die Adresse, die von einer Fehlerseite aus noch einmal geladen werden darf –
/// durch „Neu laden“, „Tab duplizieren“ oder die Sitzung vom letzten Mal.
/// Hinter der Warnung „will in dein eigenes Netz“ steht keine: Dorthin wollte
/// eine Website, nicht der Mensch am Browser. Dieses Ziel lädt nur der Knopf
/// auf der Warnseite selbst – sonst wäre ⌘R ein Ja.
pub fn error_retry(url: &str) -> Option<String> {
    let target = error_target(url)?;
    let parsed = url::Url::parse(url).ok()?;
    let local = parsed.query_pairs().any(|(key, value)| key == "v" && value == "localnet");
    (!local).then_some(target)
}

/// Adresse der Seite, deren Quelltext ein Tab zeigt (`zunder://source/?id=…&u=…`).
pub fn source_target(url: &str) -> Option<String> {
    if !url.starts_with("zunder://source") {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    parsed.query_pairs().find(|(k, _)| k == "u").map(|(_, v)| v.into_owned())
}

/// Die Adresse, wie sie in der Adressleiste steht (volle Fassung).
pub fn bar_url(url: &str) -> String {
    // Leer ist die Adressleiste nur auf der Startseite. Eine leere Seite, die
    // ein Skript geöffnet hat, soll nicht wie die Startseite aussehen.
    if is_newtab(url) {
        return String::new();
    }
    if let Some(page) = source_target(url) {
        return format!("view-source:{page}");
    }
    let shown = error_target(url).unwrap_or_else(|| url.to_string());
    // Zugangsdaten in der Adresse (name:passwort@…) werden nie gezeigt: Sie
    // stünden vor dem Rechner und sähen aus wie dessen Name.
    crate::sites::without_credentials(&shown).unwrap_or(shown)
}

/// Kurzform für die Anzeige, solange die Adressleiste nicht bearbeitet wird.
pub fn pretty_url(url: &str) -> String {
    // Eine eingebaute App steht mit ihrem Namen da.
    if let Some(app) = crate::apps::by_url(url) {
        return crate::i18n::t(app.name).to_string();
    }
    let full = bar_url(url);
    let rest = full
        .strip_prefix("https://")
        .or_else(|| full.strip_prefix("http://"))
        .unwrap_or(&full);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    match rest.strip_suffix('/') {
        Some(host) if !host.contains('/') => host.to_string(),
        _ => rest.to_string(),
    }
}

/// Sicherheitslage einer Adresse für das Symbol in der Adressleiste.
pub fn security(url: &str) -> &'static str {
    let shown = bar_url(url);
    if shown.is_empty() {
        "none"
    } else if url.starts_with("zunder://error") {
        "error"
    } else if shown.starts_with("https://") {
        "secure"
    } else if shown.starts_with("http://") {
        // Der eigene Rechner gilt wie überall als sicherer Ursprung.
        let local = url::Url::parse(&shown)
            .ok()
            .and_then(|u| u.host_str().map(|h| h == "localhost" || h == "127.0.0.1" || h == "[::1]"))
            .unwrap_or(false);
        if local {
            "internal"
        } else {
            "insecure"
        }
    } else if shown.starts_with("file://") {
        "file"
    } else {
        "internal"
    }
}

/// Holt Suchvorschläge der Suchmaschine. Läuft in einem eigenen Thread.
pub fn fetch_suggestions(engine: &Engine, query: &str) -> Vec<String> {
    let Some(template) = engine.suggest else {
        return Vec::new();
    };
    let url = template.replace("{}", &encode(query));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(1500))
        .build();
    let Ok(response) = agent.get(&url).set("Accept", "application/json").call() else {
        return Vec::new();
    };
    let Ok(body) = response.into_string() else {
        return Vec::new();
    };
    if body.len() > 256 * 1024 {
        return Vec::new();
    }
    // Format: ["anfrage", ["vorschlag 1", "vorschlag 2", …], …]
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return Vec::new();
    };
    value
        .get(1)
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty() && s.len() < 200)
                .take(5)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Entfernt bekannte reine Tracking-Parameter aus einer Adresse (`utm_…`,
/// `fbclid`, `gclid` …); alles andere bleibt unangetastet. Liefert die
/// bereinigte Adresse und wie viele Parameter entfernt wurden. Ohne Abfrage
/// oder ohne Treffer bleibt die Adresse Zeichen für Zeichen, wie sie war.
pub fn clean_tracking(url: &str) -> (String, usize) {
    const EXACT: &[&str] = &[
        "fbclid", "gclid", "gclsrc", "gbraid", "wbraid", "dclid", "msclkid", "yclid", "twclid",
        "ttclid", "igshid", "igsh", "mibextid", "mc_eid", "mc_cid", "mkt_tok", "vero_id",
        "oly_enc_id", "oly_anon_id", "_hsenc", "_hsmi", "wickedid",
    ];
    let Ok(mut parsed) = url::Url::parse(url) else {
        return (url.to_string(), 0);
    };
    if parsed.query().is_none() {
        return (url.to_string(), 0);
    }
    let before = parsed.query_pairs().count();
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| {
            let lower = key.to_ascii_lowercase();
            !(lower.starts_with("utm_") || EXACT.contains(&lower.as_str()))
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let removed = before - kept.len();
    if removed == 0 {
        return (url.to_string(), 0);
    }
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        let mut pairs = parsed.query_pairs_mut();
        pairs.clear();
        for (key, value) in &kept {
            pairs.append_pair(key, value);
        }
    }
    (parsed.to_string(), removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_parameter_werden_entfernt() {
        let (clean, n) = clean_tracking("https://example.com/p?utm_source=a&utm_medium=b&id=5&fbclid=xyz");
        assert_eq!(clean, "https://example.com/p?id=5");
        assert_eq!(n, 3);
        // Ohne Tracker unverändert (Zeichen für Zeichen).
        assert_eq!(clean_tracking("https://example.com/p?id=5&q=hi"), ("https://example.com/p?id=5&q=hi".into(), 0));
        // Nur Tracker → gar keine Abfrage mehr.
        assert_eq!(clean_tracking("https://example.com/p?utm_source=a&gclid=1"), ("https://example.com/p".into(), 2));
        // Keine Abfrage bzw. keine Adresse → unverändert.
        assert_eq!(clean_tracking("https://example.com/p"), ("https://example.com/p".into(), 0));
        assert_eq!(clean_tracking("kein url"), ("kein url".into(), 0));
    }

    #[test]
    fn urls_are_recognised() {
        assert_eq!(as_url("example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(as_url("https://a.de/x y").as_deref(), Some("https://a.de/x%20y"));
        assert_eq!(as_url("localhost:3000").as_deref(), Some("http://localhost:3000/"));
        assert_eq!(as_url("192.168.1.1").as_deref(), Some("http://192.168.1.1/"));
        assert_eq!(as_url("fritz.box").as_deref(), Some("http://fritz.box/"));
        assert_eq!(as_url("nas:8080").as_deref(), Some("http://nas:8080/"));
        assert_eq!(as_url("router/").as_deref(), Some("http://router/"));
        assert_eq!(as_url("github.com/rust-lang").as_deref(), Some("https://github.com/rust-lang"));
        assert_eq!(as_url("müller.de").as_deref(), Some("https://xn--mller-kva.de/"));
        assert_eq!(as_url("about:blank").as_deref(), Some("about:blank"));
    }

    #[test]
    fn searches_are_recognised() {
        for query in [
            "rust browser",
            "wetter",
            "node.js",
            "name@example.com",
            "was ist 3.14",
            "javascript:alert(1)",
            "foo:bar",
            "",
        ] {
            assert_eq!(as_url(query), None, "{query}");
        }
        let url = resolve("rust & wry", engine("duckduckgo")).unwrap();
        assert_eq!(url, "https://duckduckgo.com/?q=rust+%26+wry");
    }

    #[test]
    fn display_forms() {
        assert_eq!(pretty_url("https://www.example.com/"), "example.com");
        assert_eq!(pretty_url("https://example.com/a/b?c=1"), "example.com/a/b?c=1");
        assert_eq!(pretty_url(NEWTAB), "");
        assert_eq!(
            bar_url("zunder://error/?u=https%3A%2F%2Fnix.invalid%2F&c=-1003"),
            "https://nix.invalid/"
        );
        // Was vor dem @ steht, ist nicht der Rechner – und wird nicht gezeigt.
        assert_eq!(pretty_url("https://accounts.google.com@evil.example/login"), "evil.example/login");
        assert_eq!(bar_url("https://www.bank.de:x@evil.example/"), "https://evil.example/");
        assert_eq!(as_url("zunder://error/?u=https%3A%2F%2Fbank.de%2F"), None);
        assert_eq!(as_url("zunder://ui/index.html"), None);
        assert_eq!(as_url("zunder://newtab/").as_deref(), Some(NEWTAB));
        assert_eq!(error_target("zunder://error/?u=file%3A%2F%2F%2Fetc%2Fpasswd"), None);
        assert_eq!(error_target("zunder://error/?u=data%3Atext%2Fhtml%2Cx"), None);
        // Neu laden, duplizieren, Sitzung: die Seite hinter einem Ladefehler ja,
        // das Ziel hinter der Warnung „will in dein eigenes Netz“ nicht.
        assert_eq!(error_retry("zunder://error/?u=https%3A%2F%2Fbank.de%2F&c=-1003").as_deref(), Some("https://bank.de/"));
        assert_eq!(error_retry("zunder://error/?v=lookalike&u=https%3A%2F%2Fbank.de%2F").as_deref(), Some("https://bank.de/"));
        assert_eq!(error_retry("zunder://error/?v=localnet&u=http%3A%2F%2F192.168.1.1%2Fapply&f=x.example"), None);
        assert_eq!(error_target("zunder://error/?v=localnet&u=http%3A%2F%2F192.168.1.1%2Fapply").as_deref(), Some("http://192.168.1.1/apply"));
        assert_eq!(bar_url("about:blank"), "about:blank");
        assert_eq!(security("https://example.com/"), "secure");
        assert_eq!(security("http://example.com/"), "insecure");
        assert_eq!(security(NEWTAB), "none");
    }
}
