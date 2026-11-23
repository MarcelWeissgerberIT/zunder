//! Posteingang – für die gleichnamige eingebaute App (Zunder als Client zu Atlas).
//!
//! Die App zeigt offene Vorgänge aus Atlas. Der Abruf läuft über den Browser
//! selbst (`ureq`), nicht über die Seite der App: So gilt keine Grenze fremder
//! Herkünfte (CORS), und der Zugangstoken bleibt **nativ** – er liegt im
//! Schlüsselbund und wird hier an die Anfrage gehängt; die Seite sieht ihn nie.
//!
//! Wichtig fürs Vertrauen des Tokens: Die **Serveradresse** ist eine Einstellung
//! des Browsers (`BASE_KEY`), nicht etwas, das die Seite je Anfrage mitschickt –
//! sonst ließe sich der Token an einen fremden Rechner lenken. Die Seite nennt
//! nur einen **Pfad aus einer festen Liste** (`PATHS`) und eine harmlose
//! Abfragezeichenkette; den Rechner bestimmt allein die gespeicherte Adresse.

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

/// Einstellungs-Schlüssel der Atlas-Serveradresse (kein Geheimnis → Einstellung,
/// nicht Schlüsselbund).
pub const BASE_KEY: &str = "app.inbox.base.atlas";
/// Konto des Atlas-Tokens im Schlüsselbund (Dienst = der des Tresors).
const TOKEN_ACCOUNT: &str = "inbox-token-atlas";
/// Pfade, die abgerufen werden dürfen – nichts anderes.
const PATHS: &[&str] = &["/api/ingest", "/api/context", "/api/record-kinds"];
/// So viel vom Antwort-Rumpf wird gezeigt.
const BODY_MAX: u64 = 512 * 1024;
/// So lang darf der Token höchstens sein, so lang die Abfragezeichenkette.
const TOKEN_MAX: usize = 4096;
const QUERY_MAX: usize = 1024;

/// So viele Abrufe laufen höchstens gleichzeitig (die App stößt sie ohnehin
/// einzeln an; das hier ist die Grenze zur Sicherheit).
const MAX: usize = 3;
static RUNNING: AtomicUsize = AtomicUsize::new(0);

pub fn begin() -> bool {
    let mut count = RUNNING.load(Ordering::Relaxed);
    loop {
        if count >= MAX {
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

/// Was die Seite des Posteingangs möchte.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Stand melden: gespeicherte Adresse und ob ein Token hinterlegt ist.
    Status,
    /// Adresse setzen (geprüft, nur Herkunft) und – falls mitgeschickt – den
    /// Token im Schlüsselbund ablegen. Ohne Token bleibt der gespeicherte.
    Set { base: String, token: Option<String> },
    /// Den Token wegwerfen (abmelden).
    Clear,
    /// Einen Pfad aus `PATHS` samt Abfragezeichenkette abrufen.
    Get { path: String, query: String },
    /// Unlesbar oder nicht erlaubt.
    Bad,
}

/// Liest die Bitte, wie die Seite sie schickt (`{"action":…}`), und macht sie
/// sicher.
pub fn action(input: &str) -> Action {
    let Ok(value) = serde_json::from_str::<Value>(input) else {
        return Action::Bad;
    };
    match value.get("action").and_then(Value::as_str) {
        Some("status") => Action::Status,
        Some("clear") => Action::Clear,
        Some("set") => {
            let Some(base) = value.get("base").and_then(Value::as_str).and_then(clean_base) else {
                return Action::Bad;
            };
            // Ein mitgeschicktes, aber leeres Token-Feld heißt „nicht ändern“.
            match value.get("token").and_then(Value::as_str) {
                None => Action::Set { base, token: None },
                Some(token) if token.is_empty() => Action::Set { base, token: None },
                Some(token) => match clean_token(token) {
                    Some(token) => Action::Set { base, token: Some(token) },
                    None => Action::Bad,
                },
            }
        }
        Some("get") => {
            let Some(path) = value.get("path").and_then(Value::as_str).filter(|path| PATHS.contains(path)) else {
                return Action::Bad;
            };
            let query = value.get("query").and_then(Value::as_str).unwrap_or("");
            match clean_query(query) {
                Some(query) => Action::Get { path: path.to_string(), query },
                None => Action::Bad,
            }
        }
        _ => Action::Bad,
    }
}

/// Die Serveradresse auf ihre reine Herkunft (`schema://rechner[:port]`)
/// gebracht: nur `https`/`http`, ohne Zugangsdaten, ohne Pfad/Abfrage – damit
/// über die Adresse kein Pfad eingeschleust wird.
fn clean_base(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 300 {
        return None;
    }
    let parsed = url::Url::parse(trimmed).ok()?;
    let plain = matches!(parsed.scheme(), "http" | "https") && parsed.username().is_empty() && parsed.password().is_none();
    if !plain || parsed.host_str().is_none() {
        return None;
    }
    match parsed.origin() {
        url::Origin::Tuple(..) => Some(parsed.origin().ascii_serialization()),
        url::Origin::Opaque(_) => None,
    }
}

/// Ein Token, wie es der Schlüsselbund aufbewahrt: nicht leer, nicht zu lang,
/// nur sichtbare ASCII-Zeichen (keine Leer-/Steuerzeichen) – so, wie ein
/// Bearer-Token aussieht.
fn clean_token(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let ok = !trimmed.is_empty() && trimmed.len() <= TOKEN_MAX && trimmed.bytes().all(|b| (0x21..=0x7e).contains(&b));
    ok.then(|| trimmed.to_string())
}

/// Die Abfragezeichenkette (nach dem „?“): kurz und nur aus harmlosen Zeichen –
/// kein Leerzeichen, kein `?`/`#`, keine Steuerzeichen. Leer ist in Ordnung.
fn clean_query(input: &str) -> Option<String> {
    if input.len() > QUERY_MAX {
        return None;
    }
    let plain = input.bytes().all(|b| b.is_ascii_alphanumeric() || b",=&%_.:+/-@~".contains(&b));
    plain.then(|| input.to_string())
}

// ---- Token im Schlüsselbund --------------------------------------------------

fn token_get() -> Option<String> {
    security_framework::passwords::get_generic_password(crate::dev::keychain_service(), TOKEN_ACCOUNT)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|token| !token.is_empty())
}

pub fn has_token() -> bool {
    token_get().is_some()
}

pub fn set_token(token: &str) {
    let _ = security_framework::passwords::set_generic_password(crate::dev::keychain_service(), TOKEN_ACCOUNT, token.as_bytes());
}

/// Abmelden: ein leeres Token gilt als keines (`token_get` filtert es weg).
pub fn clear_token() {
    let _ = security_framework::passwords::set_generic_password(crate::dev::keychain_service(), TOKEN_ACCOUNT, b"");
}

// ---- Abruf -------------------------------------------------------------------

/// Ruft `base + path (+ ?query)` mit dem gespeicherten Token ab und liefert die
/// rohe Antwort – fertig für die Seite. Läuft im Hintergrund (`ureq` wartet aufs
/// Netz). Ohne Token: `no_token` (dann hat die App nach dem Token zu fragen).
pub fn fetch(base: &str, path: &str, query: &str) -> Value {
    let Some(token) = token_get() else {
        return json!({ "state": "no_token" });
    };
    let url = if query.is_empty() { format!("{base}{path}") } else { format!("{base}{path}?{query}") };

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(8))
        .timeout(Duration::from_secs(25))
        .redirects(0)
        .user_agent("Zunder/1.0 (Posteingang)")
        .build();

    let result = agent
        .get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .call();
    let response = match result {
        Ok(response) => response,
        // Eine 4xx/5xx-Antwort ist trotzdem eine Antwort – die zeigt die App
        // (z. B. 401 → „melde dich neu an“).
        Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(_)) => return json!({ "state": "unreachable" }),
    };

    let status = response.status();
    let mut raw: Vec<u8> = Vec::new();
    let read = response.into_reader().take(BODY_MAX + 1).read_to_end(&mut raw);
    let truncated = raw.len() as u64 > BODY_MAX;
    raw.truncate(BODY_MAX as usize);
    let body = if read.is_err() { String::new() } else { String::from_utf8_lossy(&raw).into_owned() };

    json!({
        "state": "ok",
        "status": status,
        "body": body,
        "bytes": raw.len(),
        "truncated": truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stand_abmelden_und_unsinn() {
        assert_eq!(action(r#"{"action":"status"}"#), Action::Status);
        assert_eq!(action(r#"{"action":"clear"}"#), Action::Clear);
        assert_eq!(action(r#"{"action":"quit"}"#), Action::Bad);
        assert_eq!(action("kein json"), Action::Bad);
        assert_eq!(action(r#"{"foo":1}"#), Action::Bad);
    }

    #[test]
    fn setzen_prueft_adresse_und_token() {
        // Adresse wird auf die reine Herkunft gebracht (Pfad fällt weg).
        assert_eq!(
            action(r#"{"action":"set","base":"https://aj.example.com/api/x","token":"abc.def-123"}"#),
            Action::Set { base: "https://aj.example.com".into(), token: Some("abc.def-123".into()) }
        );
        // Leeres Token-Feld = nicht ändern.
        assert_eq!(
            action(r#"{"action":"set","base":"https://a.de"}"#),
            Action::Set { base: "https://a.de".into(), token: None }
        );
        assert_eq!(
            action(r#"{"action":"set","base":"https://a.de","token":""}"#),
            Action::Set { base: "https://a.de".into(), token: None }
        );
        // Krumme Adressen und Token fallen weg.
        for bad in [
            r#"{"action":"set","base":"ftp://a.de","token":"x"}"#,
            r#"{"action":"set","base":"https://u:p@a.de","token":"x"}"#,
            r#"{"action":"set","base":"not a url","token":"x"}"#,
            r#"{"action":"set","token":"x"}"#,
            r#"{"action":"set","base":"https://a.de","token":"hat leerzeichen"}"#,
            r#"{"action":"set","base":"https://a.de","token":"zeile\numbruch"}"#,
        ] {
            assert_eq!(action(bad), Action::Bad, "{bad}");
        }
    }

    #[test]
    fn abrufen_nur_erlaubte_pfade_und_harmlose_abfrage() {
        assert_eq!(
            action(r#"{"action":"get","path":"/api/ingest","query":"kind=feature,ticket,bug"}"#),
            Action::Get { path: "/api/ingest".into(), query: "kind=feature,ticket,bug".into() }
        );
        assert_eq!(
            action(r#"{"action":"get","path":"/api/context","query":"project=atlas&shadowOwner=Marcel%20W"}"#),
            Action::Get { path: "/api/context".into(), query: "project=atlas&shadowOwner=Marcel%20W".into() }
        );
        // Ohne Abfrage ist in Ordnung.
        assert_eq!(action(r#"{"action":"get","path":"/api/record-kinds"}"#), Action::Get { path: "/api/record-kinds".into(), query: String::new() });
        // Fremder Pfad, Pfad mit Abfrage drin, böse Abfragezeichen.
        for bad in [
            r#"{"action":"get","path":"/api/secret"}"#,
            r#"{"action":"get","path":"/api/ingest?kind=x"}"#,
            r#"{"action":"get","path":"/api/ingest","query":"a b"}"#,
            r#"{"action":"get","path":"/api/ingest","query":"a#b"}"#,
            r#"{"action":"get","path":"/api/ingest","query":"a\r\nb"}"#,
            r#"{"action":"get"}"#,
        ] {
            assert_eq!(action(bad), Action::Bad, "{bad}");
        }
    }

    #[test]
    fn adresse_wird_zur_reinen_herkunft() {
        assert_eq!(clean_base("https://atlas.example.com").as_deref(), Some("https://atlas.example.com"));
        assert_eq!(clean_base("https://a.de:8443/pfad?x=1").as_deref(), Some("https://a.de:8443"));
        assert_eq!(clean_base("http://localhost:3000").as_deref(), Some("http://localhost:3000"));
        for bad in ["", "   ", "ftp://a.de", "https://u:p@a.de", "a.de", "not a url", "file:///etc"] {
            assert_eq!(clean_base(bad), None, "{bad}");
        }
    }
}
