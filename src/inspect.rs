//! Request-Inspektor – für die gleichnamige eingebaute App.
//!
//! Man stellt eine HTTP-Anfrage zusammen (Methode, Adresse, Kopfzeilen, Rumpf)
//! und sieht die rohe Antwort: Status, Dauer, Antwortkopf und Rumpf. Gedacht,
//! um die eigenen Schnittstellen zu entwickeln und zu prüfen – wie `curl`, nur
//! im Browser. Der Abruf läuft über den Browser selbst (hier, mit `ureq`),
//! nicht über die Seite der App: So gelten die Grenzen fremder Herkünfte (CORS)
//! nicht, und man sieht jede Antwort unverändert.
//!
//! Weitergeleitet wird **nicht** (`redirects(0)`): Eine 3xx-Antwort zeigt die
//! App mit ihrem `Location`, statt ihr zu folgen – beim Prüfen will man genau
//! das sehen, was der Server geantwortet hat.
//!
//! Die Anfrage geht über den Kanal der Seiten (höchstens ~4 KB je Nachricht),
//! darum ist der Rumpf klein gehalten. Die **Antwort** kommt als Skriptzeile
//! zurück und darf größer sein (bis `BODY_MAX`).

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// So viel vom Antwort-Rumpf wird gezeigt.
const BODY_MAX: u64 = 512 * 1024;
/// So groß darf der mitgeschickte Rumpf sein (Bytes) – der Kanal der Seiten ist eng.
const REQUEST_BODY_MAX: usize = 3_000;
/// Methoden, die der Inspektor schickt.
const METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
/// Kopfzeilen, die der Browser selbst setzt – sie lassen sich nicht überschreiben
/// (sonst ließe sich die Anfrage verfälschen/schmuggeln).
const RESERVED: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "expect",
    "upgrade",
    "keep-alive",
    "trailer",
    "te",
];

/// So viele Anfragen laufen höchstens gleichzeitig. Nur die App selbst stößt sie
/// an (und serialisiert sie schon); das hier ist die Grenze zur Sicherheit.
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

struct Req {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// Stellt die Anfrage, schickt sie ab und liefert die Antwort – fertig für die
/// Seite der App. Läuft im Hintergrund (`ureq` wartet auf das Netz).
pub fn run(input: &str) -> Value {
    let Some(req) = parse(input) else {
        return json!({ "state": "bad" });
    };
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(20))
        .timeout_write(Duration::from_secs(8))
        .timeout(Duration::from_secs(25))
        .redirects(0)
        .user_agent("Zunder/1.0 (Request-Inspektor)")
        .build();

    let mut request = agent.request(&req.method, &req.url);
    for (name, value) in &req.headers {
        request = request.set(name, value);
    }

    let started = Instant::now();
    let result = if req.body.is_empty() {
        request.call()
    } else {
        request.send_bytes(req.body.as_bytes())
    };
    let response = match result {
        Ok(response) => response,
        // Eine Fehler-Antwort (4xx/5xx) ist trotzdem eine Antwort – sie zeigen wir.
        Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(_)) => return json!({ "state": "unreachable" }),
    };

    let status = response.status();
    let status_text = response.status_text().to_string();
    let final_url = response.get_url().to_string();
    let mut headers: Vec<Value> = Vec::new();
    let mut content_type = String::new();
    'collect: for name in response.headers_names() {
        for value in response.all(&name) {
            if name.eq_ignore_ascii_case("content-type") && content_type.is_empty() {
                content_type = value.to_string();
            }
            // Ein einzelner Kopfzeilen-Wert wird gedeckelt, damit ein böswilliger
            // Server die Antwort nicht aufblähen kann.
            let shown: String = value.chars().take(8_192).collect();
            headers.push(json!([name, shown]));
            if headers.len() >= 200 {
                break 'collect;
            }
        }
    }

    // Den Rumpf nur lesen, wenn er Text ist; sonst nur, wie groß er ist.
    let text_like = content_type.is_empty()
        || ["text", "json", "xml", "javascript", "csv", "html", "urlencoded", "svg"].iter().any(|kind| content_type.contains(kind));
    let mut raw: Vec<u8> = Vec::new();
    let read = response.into_reader().take(BODY_MAX + 1).read_to_end(&mut raw);
    let truncated = raw.len() as u64 > BODY_MAX;
    raw.truncate(BODY_MAX as usize);
    let elapsed = started.elapsed().as_millis() as u64;

    let (body, binary) = if read.is_err() {
        (String::new(), false)
    } else if text_like {
        (String::from_utf8_lossy(&raw).into_owned(), false)
    } else {
        (String::new(), true)
    };

    json!({
        "state": "ok",
        "method": req.method,
        "status": status,
        "statusText": status_text,
        "final": final_url,
        "timeMs": elapsed,
        "headers": headers,
        "body": body,
        "bytes": raw.len(),
        "truncated": truncated,
        "binary": binary,
    })
}

/// Liest die Anfrage, wie die Seite sie schickt, und macht sie sicher: bekannte
/// Methode, abrufbare Adresse (`http`/`https`, ohne Zugangsdaten), harmlose
/// Kopfzeilen (die vom Browser gesetzten lassen sich nicht überschreiben),
/// kleiner Rumpf.
fn parse(input: &str) -> Option<Req> {
    let value: Value = serde_json::from_str(input).ok()?;
    let method = value.get("method")?.as_str()?.trim().to_ascii_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return None;
    }
    let url = clean_url(value.get("url")?.as_str()?)?;

    let mut headers: Vec<(String, String)> = Vec::new();
    if let Some(list) = value.get("headers").and_then(Value::as_array) {
        for pair in list.iter().take(50) {
            let Some(pair) = pair.as_array() else { continue };
            let (Some(name), Some(content)) = (pair.first().and_then(Value::as_str), pair.get(1).and_then(Value::as_str)) else {
                continue;
            };
            let name = name.trim();
            // Ein gültiger Kopfzeilen-Name, nicht reserviert, Wert ohne Steuerzeichen.
            let ok_name = !name.is_empty()
                && name.len() <= 100
                && name.bytes().all(|b| b.is_ascii_graphic() && !b":()<>@,;\\\"/[]?={}".contains(&b))
                && !RESERVED.contains(&name.to_ascii_lowercase().as_str());
            let value = content.trim();
            // Nur, was der Transport auch schickt (Leerzeichen, Tab, 0x21–0x7E).
            // Ein Wert mit Umlaut o. Ä. fällt weg, statt die ganze Anfrage als
            // „nicht erreichbar“ scheitern zu lassen.
            let ok_value = value.len() <= 2_000 && value.bytes().all(|b| b == b' ' || b == b'\t' || (0x21..=0x7e).contains(&b));
            if ok_name && ok_value {
                headers.push((name.to_string(), value.to_string()));
            }
        }
    }

    let body = value.get("body").and_then(Value::as_str).unwrap_or("");
    if body.len() > REQUEST_BODY_MAX {
        return None;
    }

    Some(Req { method, url, headers, body: body.to_string() })
}

/// Macht aus der Eingabe eine abrufbare Adresse: nur `http`/`https`, ohne
/// Zugangsdaten. Ohne Schema wird `https` angenommen.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn req(json: &str) -> Option<Req> {
        parse(json)
    }

    #[test]
    fn anfragen_werden_gelesen_und_gesichert() {
        let r = req(r#"{"method":"post","url":"example.com/api","headers":[["Accept","application/json"],["X-Token","abc"]],"body":"{\"a\":1}"}"#).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://example.com/api");
        assert_eq!(r.headers, vec![("Accept".into(), "application/json".into()), ("X-Token".into(), "abc".into())]);
        assert_eq!(r.body, "{\"a\":1}");
    }

    #[test]
    fn reservierte_und_krumme_kopfzeilen_fallen_weg() {
        let r = req(r#"{"method":"GET","url":"https://a.de/","headers":[["Host","evil"],["Content-Length","9"],["X-Ok","1"],["Bad Name","x"],["Inject","a\r\nSet-Cookie: b"],["X-Uni","wért"]]}"#).unwrap();
        // Nur die harmlose Zeile bleibt (Umlaut-Wert fällt weg, nicht nur CRLF).
        assert_eq!(r.headers, vec![("X-Ok".into(), "1".into())]);
    }

    #[test]
    fn nur_erlaubte_methoden_und_adressen() {
        assert!(req(r#"{"method":"TRACE","url":"https://a.de/"}"#).is_none());
        assert!(req(r#"{"method":"GET","url":"ftp://a.de/"}"#).is_none());
        assert!(req(r#"{"method":"GET","url":"https://u:p@a.de/"}"#).is_none());
        assert!(req(r#"{"method":"GET","url":""}"#).is_none());
        assert!(req(r#"{"url":"https://a.de/"}"#).is_none());
        // Ein zu großer Rumpf wird abgelehnt.
        let big = format!(r#"{{"method":"POST","url":"https://a.de/","body":"{}"}}"#, "x".repeat(REQUEST_BODY_MAX + 1));
        assert!(req(&big).is_none());
    }

    #[test]
    fn ohne_schema_wird_https() {
        assert_eq!(clean_url("example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(clean_url("http://a.de/x").as_deref(), Some("http://a.de/x"));
        assert_eq!(clean_url("localhost:3000/health").as_deref(), Some("https://localhost:3000/health"));
    }
}
