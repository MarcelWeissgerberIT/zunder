//! Wem gehört eine Adresse? Und sieht sie einer bekannten nur täuschend ähnlich?
//!
//! Zwei Dinge, die beim Schutz vor Betrugsseiten zusammengehören:
//!
//!   - **Der Teil, der zählt.** Von `konto.bank.example.com.sicher-login.net`
//!     sagt nur `sicher-login.net`, wem die Seite gehört. Die Adressleiste hebt
//!     genau diesen Teil hervor und lässt ihn nie aus dem Bild rutschen.
//!   - **Täuschend ähnliche Adressen.** Wer zum ersten Mal auf einer Adresse
//!     landet, die einer oft besuchten, als Lesezeichen gemerkten oder mit
//!     Passwort gesicherten Seite zum Verwechseln ähnelt (`paypa1.com`,
//!     kyrillisches „а“ statt „a“, `paypal.com.konto.example`), wird vor dem
//!     Laden gefragt.
//!
//! Verglichen wird ausschließlich mit dem, was der Browser selbst über die
//! eigenen Gewohnheiten weiß – keine Adresse verlässt dafür den Mac.

use std::collections::HashSet;

/// Endungen aus mehr als einem Teil, unter denen jede:r einen Namen bekommt –
/// Länder mit Unterbereichen und Anbieter, bei denen jede Unteradresse jemand
/// anderem gehört. Bewusst eine überschaubare Liste der verbreitetsten Fälle;
/// fehlt eine Endung, gilt wie überall sonst: die letzten zwei Teile zählen.
const SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "ltd.uk", "plc.uk", "net.uk", "sch.uk", "nhs.uk",
    "com.au", "net.au", "org.au", "edu.au", "gov.au", "id.au", "co.nz", "net.nz", "org.nz", "govt.nz",
    "ac.nz", "co.jp", "ne.jp", "or.jp", "ac.jp", "go.jp", "com.br", "net.br", "org.br", "gov.br",
    "com.cn", "net.cn", "org.cn", "gov.cn", "com.mx", "org.mx", "com.tr", "org.tr", "gov.tr", "co.in",
    "net.in", "org.in", "gov.in", "ac.in", "co.za", "org.za", "gov.za", "com.sg", "edu.sg", "gov.sg",
    "com.hk", "org.hk", "co.kr", "or.kr", "go.kr", "com.tw", "org.tw", "co.il", "org.il", "ac.il",
    "com.ar", "gob.ar", "com.pl", "org.pl", "net.pl", "co.at", "or.at", "gv.at", "ac.at", "com.ua",
    "com.ru", "co.id", "or.id", "com.my", "com.ph", "com.vn", "com.eg", "com.sa", "com.pk", "com.ng",
    "co.ke", "com.co", "com.pe", "com.ve", "co.th", "in.th", "com.es", "org.es", "com.pt", "com.gr",
    "co.hu", "com.ro", "co.rs", "com.hr", "com.cy", "com.mt", "eu.org",
    // Anbieter: Jede Unteradresse gehört jemand anderem.
    "github.io", "gitlab.io", "pages.dev", "workers.dev", "vercel.app", "netlify.app", "herokuapp.com",
    "web.app", "firebaseapp.com", "appspot.com", "blogspot.com", "wordpress.com", "azurewebsites.net",
    "cloudfront.net", "amazonaws.com", "onrender.com", "fly.dev", "glitch.me", "repl.co", "replit.app",
    "ngrok.io", "ngrok.app", "ngrok-free.app", "translate.goog", "myshopify.com", "wixsite.com",
    "weebly.com", "webflow.io", "surge.sh", "r2.dev", "trycloudflare.com", "duckdns.org", "dyndns.org",
    "ddns.net", "godaddysites.com", "square.site", "carrd.co", "notion.site", "framer.app",
];

fn is_ip(host: &str) -> bool {
    host.starts_with('[') || host.parse::<std::net::Ipv4Addr>().is_ok()
}

/// Zerlegt einen Rechnernamen in den Vorsatz (Unterbereiche, mit Punkt am Ende)
/// und den Teil, der sagt, wem die Seite gehört.
pub fn split_host(host: &str) -> (String, String) {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || is_ip(&host) || !host.contains('.') {
        return (String::new(), host);
    }
    let labels: Vec<&str> = host.split('.').collect();
    let two = labels[labels.len() - 2..].join(".");
    let suffix_len = if SUFFIXES.contains(&two.as_str()) { 2 } else { 1 };
    let keep = (suffix_len + 1).min(labels.len());
    let domain = labels[labels.len() - keep..].join(".");
    let prefix = labels[..labels.len() - keep].join(".");
    let prefix = if prefix.is_empty() { prefix } else { format!("{prefix}.") };
    (prefix, domain)
}

/// Der Teil einer Adresse, der sagt, wem die Seite gehört – nur für http(s).
pub fn domain_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    Some(split_host(parsed.host_str()?).1)
}

/// Eine Adresse in drei Teilen für die Anzeige: Unterbereiche, der Teil, der
/// zählt, und der Rest (Port, Pfad, Abfrage). Zugangsdaten in der Adresse
/// (`name:passwort@`) werden nie gezeigt – sie stünden sonst vor dem Rechner
/// und sähen aus wie dessen Name.
pub fn display_parts(url: &str) -> Option<[String; 3]> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let (prefix, domain) = split_host(host);
    let prefix = prefix.strip_prefix("www.").unwrap_or(&prefix).to_string();
    let mut rest = String::new();
    if let Some(port) = parsed.port() {
        rest.push_str(&format!(":{port}"));
    }
    let tail = &parsed[url::Position::BeforePath..];
    if tail != "/" {
        rest.push_str(tail);
    }
    Some([prefix, domain, rest])
}

/// Eine Adresse ohne eingebettete Zugangsdaten.
pub fn without_credentials(url: &str) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    if parsed.username().is_empty() && parsed.password().is_none() {
        return None;
    }
    parsed.set_username("").ok()?;
    parsed.set_password(None).ok()?;
    Some(parsed.to_string())
}

// ---------------------------------------------------------------------------
// Täuschend ähnliche Adressen
// ---------------------------------------------------------------------------

/// Bringt einen Namen auf das, was das Auge sieht: Buchstaben fremder Schriften,
/// die wie lateinische aussehen, Buchstaben mit Zeichen darüber, Ziffern, die
/// für Buchstaben stehen, und Buchstabenpaare, die wie einer wirken.
pub fn skeleton(label: &str) -> String {
    // Kodierte Namen (xn--…) erst in ihre eigentlichen Zeichen übersetzen.
    let decoded = if label.contains("xn--") {
        idna::domain_to_unicode(label).0
    } else {
        label.to_string()
    };
    let mut out = String::with_capacity(decoded.len());
    for c in decoded.to_lowercase().chars() {
        match c {
            // Ziffern, die für Buchstaben stehen; Bindestriche fallen nicht auf
            '0' => out.push('o'),
            '1' => out.push('l'),
            '-' | '\u{00ad}' | '\u{200b}'..='\u{200f}' => {}
            // Lateinisch mit Zeichen darüber oder darunter
            'à'..='å' | 'ā' | 'ă' | 'ą' | 'ǎ' => out.push('a'),
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => out.push('c'),
            'ď' | 'đ' => out.push('d'),
            'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => out.push('e'),
            'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'ɡ' => out.push('g'),
            'ĥ' | 'ħ' => out.push('h'),
            'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => out.push('i'),
            'ĵ' => out.push('j'),
            'ķ' | 'ĸ' => out.push('k'),
            'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => out.push('l'),
            'ñ' | 'ń' | 'ņ' | 'ň' => out.push('n'),
            'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => out.push('o'),
            'ŕ' | 'ŗ' | 'ř' => out.push('r'),
            'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => out.push('s'),
            'ţ' | 'ť' | 'ŧ' | 'ț' => out.push('t'),
            'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => out.push('u'),
            'ŵ' => out.push('w'),
            'ý' | 'ÿ' | 'ŷ' => out.push('y'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            // Kyrillisch
            'а' => out.push('a'),
            'в' => out.push('b'),
            'е' | 'ё' | 'є' => out.push('e'),
            'һ' | 'н' => out.push('h'),
            'і' | 'ї' | 'ӏ' => out.push('i'),
            'ј' => out.push('j'),
            'к' => out.push('k'),
            'м' => out.push('m'),
            'о' => out.push('o'),
            'р' => out.push('p'),
            'ԛ' => out.push('q'),
            'ѕ' => out.push('s'),
            'с' => out.push('c'),
            'т' => out.push('t'),
            'у' | 'ү' => out.push('y'),
            'ԝ' | 'ѡ' => out.push('w'),
            'х' => out.push('x'),
            'ԁ' => out.push('d'),
            // Griechisch
            'α' => out.push('a'),
            'β' => out.push('b'),
            'ε' => out.push('e'),
            'η' => out.push('n'),
            'ι' => out.push('i'),
            'κ' => out.push('k'),
            'ν' => out.push('v'),
            'ο' | 'σ' => out.push('o'),
            'ρ' => out.push('p'),
            'τ' => out.push('t'),
            'υ' | 'μ' => out.push('u'),
            'χ' => out.push('x'),
            'ω' => out.push('w'),
            // Armenisch
            'օ' => out.push('o'),
            'ո' => out.push('n'),
            'ս' => out.push('u'),
            'հ' => out.push('h'),
            other => out.push(other),
        }
    }
    // Paare, die zusammen wie ein Buchstabe aussehen
    out.replace("rn", "m").replace("vv", "w")
}

/// Unterscheiden sich zwei Namen nur um einen Tippfehler: ein Zeichen anders,
/// eines zu viel oder zu wenig, zwei benachbarte vertauscht?
fn one_typo_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a == b || a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    // Gemeinsamer Anfang und gemeinsames Ende abschneiden; was bleibt, ist der Unterschied.
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let (ra, rb) = (&a[start..], &b[start..]);
    let end = ra.iter().rev().zip(rb.iter().rev()).take_while(|(x, y)| x == y).count();
    let (da, db) = (&ra[..ra.len() - end], &rb[..rb.len() - end]);
    match (da.len(), db.len()) {
        (1, 1) | (1, 0) | (0, 1) => true,
        (2, 2) => da[0] == db[1] && da[1] == db[0],
        _ => false,
    }
}

/// Eine Seite, die der Mensch am Browser kennt.
#[derive(Debug, Clone)]
struct Known {
    /// Der Teil, der zählt, z. B. `paypal.com`.
    domain: String,
    /// Der Rechner, unter dem die Seite bekannt ist (`www.paypal.com`).
    host: String,
    /// Sein vorderster Teil (`paypal`) und dessen Bild fürs Auge.
    label: String,
    skeleton: String,
    /// Alles nach dem ersten Punkt (`com`, `co.uk`).
    ending: String,
    /// Lesezeichen, gesichertes Passwort oder sehr oft besucht.
    strong: bool,
}

/// Die bekannte Seite, der eine fremde Adresse zum Verwechseln ähnelt.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Der Teil, der zählt (`paypal.com`) – so heißt die Seite in der Warnung.
    pub domain: String,
    /// Der Rechner, zu dem „Zur bekannten Seite“ führt (`www.paypal.com`).
    pub host: String,
}

/// Was der Browser über die eigenen Gewohnheiten weiß – Grundlage der Prüfung.
#[derive(Debug, Default)]
pub struct SiteGuard {
    known: Vec<Known>,
    /// Alle schon einmal besuchten oder bekannten Seiten: Sie werden nie geprüft.
    visited: HashSet<String>,
    /// In dieser Sitzung ausdrücklich erlaubt.
    allowed: HashSet<String>,
    pub enabled: bool,
}

fn label_and_ending(domain: &str) -> (&str, &str) {
    domain.split_once('.').unwrap_or((domain, ""))
}

impl SiteGuard {
    /// `sites`: (Rechnername, stark), die wichtigsten zuerst – stark heißt
    /// Lesezeichen, gesichertes Passwort oder oft besucht. `seen` sind alle
    /// Rechner, die je besucht wurden.
    pub fn rebuild(&mut self, sites: &[(String, bool)], seen: &[String]) {
        self.known.clear();
        self.visited.clear();
        for host in seen {
            let (_, domain) = split_host(host);
            if !domain.is_empty() {
                self.visited.insert(domain);
            }
        }
        for (host, strong) in sites {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            let (_, domain) = split_host(&host);
            if domain.is_empty() || is_ip(&domain) || !domain.contains('.') {
                continue;
            }
            self.visited.insert(domain.clone());
            if let Some(existing) = self.known.iter_mut().find(|k| k.domain == domain) {
                existing.strong |= *strong;
                continue;
            }
            let (label, ending) = label_and_ending(&domain);
            // Sehr kurze Namen ähneln allem Möglichen – mit ihnen wird nicht verglichen.
            if label.chars().count() < 4 {
                continue;
            }
            self.known.push(Known {
                skeleton: skeleton(label),
                label: label.to_string(),
                ending: ending.to_string(),
                strong: *strong,
                host,
                domain,
            });
        }
    }

    /// Eine Seite wurde besucht: Sie gilt ab jetzt als bekannt.
    pub fn visited(&mut self, url: &str) {
        if let Some(domain) = domain_of(url) {
            self.visited.insert(domain);
        }
    }

    /// Der Mensch will diese Seite trotz Warnung öffnen. `host` ist der
    /// Rechnername, vor dem gewarnt wurde.
    pub fn allow(&mut self, host: &str) {
        let host = host.trim().to_ascii_lowercase();
        let plausible = !host.is_empty()
            && host.len() <= 253
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
        if plausible {
            self.allowed.insert(split_host(&host).1);
        }
    }

    /// Sieht diese Adresse einer bekannten Seite täuschend ähnlich? Liefert
    /// die bekannte Seite. Geprüft werden nur Seiten, auf denen man noch nie war.
    pub fn check(&self, url: &str) -> Option<Match> {
        if !self.enabled {
            return None;
        }
        let parsed = url::Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let host = parsed.host_str()?.trim_end_matches('.').to_ascii_lowercase();
        if is_ip(&host) || !host.contains('.') {
            return None;
        }
        let (prefix, domain) = split_host(&host);
        if self.visited.contains(&domain) || self.allowed.contains(&domain) {
            return None;
        }
        let (label, ending) = label_and_ending(&domain);
        let found = |known: &Known| Some(Match { domain: known.domain.clone(), host: known.host.clone() });

        // 1. Eine bekannte Adresse steckt vorn in der fremden:
        //    `paypal.com.konto.example` oder `www-paypal-com.example`.
        let front = format!(".{}", prefix);
        let labels: Vec<Vec<&str>> = prefix
            .split('.')
            .filter(|l| !l.is_empty())
            .chain(std::iter::once(label))
            .map(|l| l.split('-').collect())
            .collect();
        for known in self.known.iter().filter(|k| k.strong) {
            let dotted = format!(".{}.", known.domain);
            let dashed = known.domain.replace('.', "-");
            let parts = dashed.matches('-').count() + 1;
            let in_prefix = front.contains(&dotted);
            let in_label = labels.iter().any(|pieces| pieces.windows(parts).any(|w| w.join("-") == dashed));
            if in_prefix || in_label {
                return found(known);
            }
        }

        // 2. Fürs Auge derselbe Name, aber andere Zeichen: `paypa1`, kyrillisches „а“.
        let seen = skeleton(label);
        for known in &self.known {
            if known.label != label && known.skeleton == seen {
                return found(known);
            }
        }

        // 3. Ein Tippfehler neben einer wichtigen Seite mit derselben Endung.
        if label.chars().count() >= 5 {
            for known in self.known.iter().filter(|k| k.strong && k.ending == ending) {
                if known.label.chars().count() >= 6 && one_typo_apart(&known.label, label) {
                    return found(known);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_that_counts() {
        assert_eq!(split_host("www.example.com"), ("www.".into(), "example.com".into()));
        assert_eq!(split_host("example.com"), (String::new(), "example.com".into()));
        assert_eq!(split_host("a.b.shop.co.uk"), ("a.b.".into(), "shop.co.uk".into()));
        assert_eq!(split_host("co.uk"), (String::new(), "co.uk".into()));
        assert_eq!(split_host("nutzer.github.io"), (String::new(), "nutzer.github.io".into()));
        assert_eq!(split_host("x.nutzer.github.io"), ("x.".into(), "nutzer.github.io".into()));
        assert_eq!(split_host("localhost"), (String::new(), "localhost".into()));
        assert_eq!(split_host("192.168.1.1"), (String::new(), "192.168.1.1".into()));
        assert_eq!(split_host("[::1]"), (String::new(), "[::1]".into()));
        assert_eq!(split_host("Konto.Bank.EXAMPLE.com."), ("konto.bank.".into(), "example.com".into()));
        // Der Klassiker: Der bekannte Name steht vorn, gehören tut die Seite jemand anderem.
        assert_eq!(
            split_host("accounts.google.com.sicher-login.net"),
            ("accounts.google.com.".into(), "sicher-login.net".into())
        );
        assert_eq!(domain_of("https://www.example.com/a?b").as_deref(), Some("example.com"));
        assert_eq!(domain_of("file:///tmp/x"), None);
    }

    #[test]
    fn address_in_three_parts() {
        let parts = |url: &str| display_parts(url).unwrap();
        assert_eq!(parts("https://www.example.com/"), [String::new(), "example.com".into(), String::new()]);
        assert_eq!(
            parts("https://konto.bank.example.com/a/b?c=1#d"),
            ["konto.bank.".to_string(), "example.com".into(), "/a/b?c=1#d".into()]
        );
        assert_eq!(parts("http://localhost:8741/login.html"), [String::new(), "localhost".into(), ":8741/login.html".into()]);
        // Zugangsdaten in der Adresse täuschen einen anderen Rechner vor – sie werden nicht gezeigt.
        assert_eq!(
            parts("https://accounts.google.com@evil.example/login"),
            [String::new(), "evil.example".into(), "/login".into()]
        );
        assert_eq!(
            without_credentials("https://accounts.google.com:x@evil.example/login").as_deref(),
            Some("https://evil.example/login")
        );
        assert_eq!(without_credentials("https://evil.example/login"), None);
        assert!(display_parts("zunder://newtab/").is_none());
        assert!(display_parts("about:blank").is_none());
    }

    #[test]
    fn what_the_eye_sees() {
        assert_eq!(skeleton("paypa1"), "paypal");
        assert_eq!(skeleton("g00gle"), "google");
        assert_eq!(skeleton("pay-pal"), "paypal");
        assert_eq!(skeleton("rnicrosoft"), "microsoft");
        assert_eq!(skeleton("applé"), "apple");
        // kyrillisches а, р und е
        assert_eq!(skeleton("\u{0430}\u{0440}\u{0440}l\u{0435}"), "apple");
        // dasselbe, wie es in der Adresse steht
        assert_eq!(skeleton("xn--pypal-4ve"), "paypal");
        assert_eq!(skeleton("zunder"), "zunder");
    }

    #[test]
    fn typos() {
        assert!(one_typo_apart("google", "gogle"));
        assert!(one_typo_apart("google", "gooogle"));
        assert!(one_typo_apart("google", "googel"));
        assert!(one_typo_apart("paypal", "paypai"));
        assert!(!one_typo_apart("google", "google"));
        assert!(!one_typo_apart("github", "gitlab"));
        assert!(!one_typo_apart("amazon", "amazing"));
    }

    fn guard() -> SiteGuard {
        let mut guard = SiteGuard { enabled: true, ..Default::default() };
        guard.rebuild(
            &[
                ("www.paypal.com".to_string(), true),
                ("github.com".to_string(), true),
                ("accounts.google.com".to_string(), true),
                ("www.sparkasse.de".to_string(), true),
                ("heise.de".to_string(), false),
                ("ab.de".to_string(), true),
            ],
            &["wikipedia.org".to_string(), "gitlab.com".to_string(), "paypal-community.com".to_string()],
        );
        guard
    }

    #[test]
    fn lookalikes_are_caught() {
        let guard = guard();
        let like = |url: &str| guard.check(url).map(|m| m.domain);
        // Andere Zeichen, gleiches Bild
        assert_eq!(like("https://paypa1.com/login").as_deref(), Some("paypal.com"));
        assert_eq!(like("https://www.pay-pal.com/").as_deref(), Some("paypal.com"));
        assert_eq!(like("https://xn--pypal-4ve.com/").as_deref(), Some("paypal.com"));
        assert_eq!(like("https://g00gle.net/").as_deref(), Some("google.com"));
        // Tippfehler neben einer wichtigen Seite
        assert_eq!(like("https://githuub.com/").as_deref(), Some("github.com"));
        assert_eq!(like("https://sparkase.de/").as_deref(), Some("sparkasse.de"));
        // Die bekannte Adresse steckt vorn in der fremden
        assert_eq!(like("https://paypal.com.konto-pruefen.example/").as_deref(), Some("paypal.com"));
        assert_eq!(like("https://www.paypal.com.sicher.example.org/x").as_deref(), Some("paypal.com"));
        assert_eq!(like("https://www-paypal-com.example.net/").as_deref(), Some("paypal.com"));
        assert_eq!(like("https://login-github-com-auth.example.net/").as_deref(), Some("github.com"));
    }

    #[test]
    fn ordinary_sites_pass() {
        let guard = guard();
        for url in [
            "https://www.paypal.com/",            // die echte
            "https://paypal.de/",                 // gleicher Name, andere Endung: keine Täuschung der Zeichen
            "https://gitlab.com/",                // schon besucht
            "https://paypal-community.com/",      // schon besucht
            "https://example.org/",
            "https://github.blog/",
            "https://heisse.de/",                 // Tippfehler-Prüfung nur neben wichtigen Seiten mit langem Namen
            "https://abc.de/",                    // zu kurze Namen werden nicht verglichen
            "https://sparkasse-musterstadt.de/",  // enthält den Namen, aber nicht die ganze Adresse
            "http://192.168.1.1/",
            "http://localhost:3000/",
            "zunder://newtab/",
        ] {
            assert_eq!(guard.check(url), None, "{url}");
        }
        // „Zur bekannten Seite“ führt zu dem Rechner, unter dem sie bekannt ist.
        assert_eq!(guard.check("https://paypa1.com/").map(|m| m.host).as_deref(), Some("www.paypal.com"));
    }

    #[test]
    fn allowed_and_visited_are_left_alone() {
        let mut guard = guard();
        assert!(guard.check("https://paypa1.com/").is_some());
        guard.allow("www.paypa1.com");
        assert_eq!(guard.check("https://paypa1.com/"), None);
        assert_eq!(guard.check("https://login.paypa1.com/"), None);
        assert!(guard.check("https://githuub.com/").is_some());
        guard.visited("https://www.githuub.com/irgendwas");
        assert_eq!(guard.check("https://githuub.com/"), None);
        // Unbrauchbares wird nicht erlaubt.
        guard.allow("a b<script>");
        guard.enabled = false;
        assert_eq!(guard.check("https://xn--pypal-4ve.com/"), None);
    }
}
