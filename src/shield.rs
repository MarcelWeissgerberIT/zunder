//! Schutz beim Navigieren: entscheidet für jede Adresse, die ein Tab laden
//! will, ob sie so geladen wird, wie sie ist.
//!
//!   - **Nur verschlüsselt:** Eine `http://`-Adresse wird zuerst verschlüsselt
//!     versucht. Geht das nicht, fragt Zunder nach, statt still unverschlüsselt
//!     zu laden. Geräte im eigenen Netz (localhost, 192.168.x.x, `.local` …)
//!     sind ausgenommen – dort gibt es meist kein Zertifikat.
//!   - **Tracking-Anhängsel:** Kennungen wie `utm_source` oder `fbclid`, mit
//!     denen Klicks über Seiten hinweg verfolgt werden, fliegen aus der Adresse.
//!
//! Alles hier ist reine Rechnung ohne Zustand – aufgerufen wird es aus der
//! Navigationsentscheidung in `native.rs`.

use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Parameter, die nur dem Verfolgen dienen. Bewusst eine kurze, eindeutige
/// Liste: nichts, wovon der Inhalt einer Seite abhängt.
const TRACKING_PARAMS: &[&str] = &[
    "fbclid", "gclid", "gclsrc", "dclid", "gbraid", "wbraid", "msclkid", "yclid", "twclid", "ttclid",
    "igshid", "li_fat_id", "mc_eid", "mc_cid", "mkt_tok", "srsltid", "_hsenc", "_hsmi", "__hssc", "__hstc",
    "__hsfp", "hsctatracking", "oly_anon_id", "oly_enc_id", "vero_id", "s_cid", "_openstat", "wickedid",
    "ml_subscriber", "ml_subscriber_hash", "rb_clickid",
];

/// Was mit einer Adresse geschehen soll, die nicht unverändert geladen wird.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Unverschlüsselt: erst verschlüsselt versuchen bzw. nachfragen.
    Insecure(String),
    /// Stattdessen diese bereinigte Adresse laden.
    Rewrite(String),
}

fn is_tracking(name: &str) -> bool {
    // Erst entschlüsseln: „utm%5Fsource“ ist dasselbe wie „utm_source“.
    let name = url::form_urlencoded::parse(name.as_bytes())
        .next()
        .map(|(key, _)| key.to_ascii_lowercase())
        .unwrap_or_default();
    name.starts_with("utm_") || TRACKING_PARAMS.contains(&name.as_str())
}

/// Entfernt Tracking-Parameter. `None`, wenn es nichts zu entfernen gab.
pub fn strip_tracking(url: &str) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let query = parsed.query()?;
    // Die Parameter bleiben Zeichen für Zeichen, wie sie waren – nur die
    // verfolgenden fallen weg. So ändert sich an keiner Kodierung etwas.
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| !is_tracking(pair.split('=').next().unwrap_or("")))
        .collect();
    if kept.len() == query.split('&').count() {
        return None;
    }
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.set_query(Some(&kept.join("&")));
    }
    Some(parsed.to_string())
}

/// Rechner und Geräte im eigenen Netz – für sie gilt „nur verschlüsselt“ nicht.
pub fn is_local_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<Ipv4Addr>() {
        return ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified();
    }
    if let Ok(ip) = bare.parse::<Ipv6Addr>() {
        let first = ip.segments()[0];
        // ::1, fe80::/10 (verbindungslokal), fc00::/7 (privat)
        return ip.is_loopback() || (first & 0xffc0) == 0xfe80 || (first & 0xfe00) == 0xfc00;
    }
    host == "localhost"
        || !host.contains('.') // Namen ohne Punkt gibt es nur im eigenen Netz
        || [".localhost", ".local", ".test", ".internal", ".lan", ".home.arpa"]
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

/// Die verschlüsselte Fassung einer `http://`-Adresse. `None`, wenn sie einen
/// eigenen Port nennt – dann lässt sich nicht erraten, wo die verschlüsselte
/// Fassung läge.
pub fn https_version(url: &str) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "http" || parsed.port().is_some() {
        return None;
    }
    parsed.set_scheme("https").ok()?;
    Some(parsed.to_string())
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

/// Prüft eine Adresse, die im Hauptrahmen eines Tabs geladen werden soll.
/// `allowed` sind die Rechner, für die in dieser Sitzung „unverschlüsselt“
/// ausdrücklich erlaubt wurde.
pub fn check(url: &str, https_only: bool, strip: bool, allowed: &HashSet<String>) -> Option<Verdict> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let cleaned = if strip { strip_tracking(url) } else { None };
    if https_only && parsed.scheme() == "http" {
        let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
        if !is_local_host(&host) && !allowed.contains(&host) {
            return Some(Verdict::Insecure(cleaned.unwrap_or_else(|| url.to_string())));
        }
    }
    cleaned.map(Verdict::Rewrite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_parameters_are_removed_and_the_rest_is_untouched() {
        assert_eq!(
            strip_tracking("https://shop.de/artikel?id=7&utm_source=news&utm_medium=mail&farbe=rot&fbclid=AbC#bewertung"),
            Some("https://shop.de/artikel?id=7&farbe=rot#bewertung".to_string())
        );
        // Nur Tracking: Das Fragezeichen verschwindet mit.
        assert_eq!(strip_tracking("https://a.de/x?gclid=1&UTM_Campaign=y"), Some("https://a.de/x".to_string()));
        // Kodierung und Reihenfolge der übrigen Parameter bleiben, wie sie waren.
        assert_eq!(
            strip_tracking("https://a.de/s?q=tee%20%2B+milch&msclkid=9&leer=&x"),
            Some("https://a.de/s?q=tee%20%2B+milch&leer=&x".to_string())
        );
        // Verschleierte Schreibweisen fallen genauso weg.
        assert_eq!(strip_tracking("https://a.de/x?utm%5Fsource=1&%66bclid=2&id=3"), Some("https://a.de/x?id=3".to_string()));
        // Nichts zu tun – und ähnlich klingende Namen bleiben.
        assert_eq!(strip_tracking("https://a.de/x?id=1&utmost=2&gclidx=3"), None);
        assert_eq!(strip_tracking("https://a.de/x"), None);
        assert_eq!(strip_tracking("file:///tmp/a?utm_source=x"), None);
    }

    #[test]
    fn local_hosts_are_recognised() {
        for host in ["localhost", "127.0.0.1", "192.168.1.1", "10.0.0.7", "172.16.4.2", "169.254.3.3", "router", "nas.local", "app.test", "[::1]", "fe80::1", "fd12::1", "drucker.lan"] {
            assert!(is_local_host(host), "{host}");
        }
        for host in ["example.com", "8.8.8.8", "172.32.0.1", "local.example.com", "2001:db8::1", "localhost.de"] {
            assert!(!is_local_host(host), "{host}");
        }
    }

    #[test]
    fn insecure_addresses_are_held_back() {
        let none = HashSet::new();
        assert_eq!(
            check("http://example.com/a?utm_source=x", true, true, &none),
            Some(Verdict::Insecure("http://example.com/a".to_string()))
        );
        assert_eq!(check("http://example.com/a", true, false, &none), Some(Verdict::Insecure("http://example.com/a".to_string())));
        // Ausdrücklich erlaubt, im eigenen Netz oder Schutz aus: durchlassen.
        let allowed: HashSet<String> = ["example.com".to_string()].into();
        assert_eq!(check("http://example.com/a", true, true, &allowed), None);
        assert_eq!(check("http://192.168.0.1/", true, true, &none), None);
        assert_eq!(check("http://example.com/a", false, true, &none), None);
        // Verschlüsselte Seiten: nur das Tracking fällt weg.
        assert_eq!(
            check("https://example.com/a?fbclid=1", true, true, &none),
            Some(Verdict::Rewrite("https://example.com/a".to_string()))
        );
        assert_eq!(check("https://example.com/a?fbclid=1", true, false, &none), None);
        assert_eq!(check("zunder://newtab/", true, true, &none), None);
    }

    #[test]
    fn https_version_keeps_everything_but_the_scheme() {
        assert_eq!(https_version("http://example.com/a?b=1#c"), Some("https://example.com/a?b=1#c".to_string()));
        assert_eq!(https_version("http://example.com:8080/"), None);
        assert_eq!(https_version("https://example.com/"), None);
    }
}
