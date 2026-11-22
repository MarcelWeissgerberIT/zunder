//! Passwort-Tresor: Verschlüsselung, Schlüssel, Zuordnung zu Websites,
//! Passwort-Vorschläge sowie Ein- und Ausfuhr als CSV.
//!
//! Website und Benutzername einer Anmeldung stehen lesbar in der Datenbank –
//! sonst ließe sich nicht nachsehen, ob es zu einer Seite etwas gibt. Das
//! Passwort steht dort nur verschlüsselt (ChaCha20-Poly1305), und zwar gebunden
//! an genau diese Website und diesen Namen: Ein verschlüsseltes Passwort, das
//! jemand in der Datei zu einer anderen Website umhängt, lässt sich nicht mehr
//! öffnen. Der Schlüssel liegt im Schlüsselbund von macOS und wird erst geholt,
//! wenn zum ersten Mal ein Passwort gelesen oder gesichert wird.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

/// Längstes Passwort bzw. längster Name, den der Tresor annimmt.
pub const MAX_PASSWORD: usize = 1000;
pub const MAX_USERNAME: usize = 320;

// ---------------------------------------------------------------------------
// Websites
// ---------------------------------------------------------------------------

/// Eine Website, wie der Tresor sie sieht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// Herkunft, z. B. `https://www.example.com` (mit Port, wenn er vom üblichen abweicht).
    pub origin: String,
    /// Wonach gesucht wird: Rechnername ohne „www.“, dazu ein unüblicher Port.
    pub key: String,
    /// Rechnername zum Anzeigen (ohne „www.“).
    pub host: String,
    /// Über https geladen.
    pub secure: bool,
}

/// Die Website zu einer Adresse – nur für http und https.
pub fn site_of(url: &str) -> Option<Site> {
    let parsed = url::Url::parse(url).ok()?;
    let secure = match parsed.scheme() {
        "https" => true,
        "http" => false,
        _ => return None,
    };
    let full = parsed.host_str()?.to_ascii_lowercase();
    if full.is_empty() {
        return None;
    }
    // „www.“ gehört zur selben Website – aber nur, wenn danach noch ein
    // vollständiger Name übrig bleibt.
    let host = match full.strip_prefix("www.") {
        Some(rest) if rest.contains('.') => rest.to_string(),
        _ => full.clone(),
    };
    let mut origin = format!("{}://{}", parsed.scheme(), full);
    let mut key = host.clone();
    // `port()` ist leer, wenn es der übliche Port des Schemas ist.
    if let Some(port) = parsed.port() {
        origin.push_str(&format!(":{port}"));
        key.push_str(&format!(":{port}"));
    }
    Some(Site { origin, key, host, secure })
}

/// Darf eine für `saved_origin` gesicherte Anmeldung auf `page` angeboten
/// werden? Nur auf derselben Website – und eine verschlüsselt gesicherte nie
/// auf einer unverschlüsselten Seite.
pub fn offered(saved_origin: &str, page: &Site) -> bool {
    site_of(saved_origin).is_some_and(|saved| saved.key == page.key && (page.secure || !saved.secure))
}

// ---------------------------------------------------------------------------
// Verschlüsselung
// ---------------------------------------------------------------------------

const BLOB_VERSION: u8 = 1;
const TAG_LEN: usize = 16;

/// Woran ein verschlüsseltes Passwort gebunden ist.
fn binding(origin: &str, username: &str) -> Vec<u8> {
    let mut bound = b"zunder-login-v1\0".to_vec();
    bound.extend_from_slice(origin.as_bytes());
    bound.push(0);
    bound.extend_from_slice(username.as_bytes());
    bound
}

fn cipher(key: &[u8; 32]) -> Result<LessSafeKey, VaultError> {
    UnboundKey::new(&CHACHA20_POLY1305, key)
        .map(LessSafeKey::new)
        .map_err(|_| VaultError::Other("Schlüssel unbrauchbar".into()))
}

fn seal_with(key: &[u8; 32], origin: &str, username: &str, password: &str) -> Result<Vec<u8>, VaultError> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| VaultError::Other("keine Zufallszahlen".into()))?;
    let mut data = password.as_bytes().to_vec();
    cipher(key)?
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(binding(origin, username)),
            &mut data,
        )
        .map_err(|_| VaultError::Other("Verschlüsseln fehlgeschlagen".into()))?;
    let mut blob = Vec::with_capacity(1 + NONCE_LEN + data.len());
    blob.push(BLOB_VERSION);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&data);
    Ok(blob)
}

fn open_with(key: &[u8; 32], origin: &str, username: &str, blob: &[u8]) -> Result<String, VaultError> {
    if blob.len() < 1 + NONCE_LEN + TAG_LEN || blob[0] != BLOB_VERSION {
        return Err(VaultError::Damaged);
    }
    let nonce = Nonce::try_assume_unique_for_key(&blob[1..1 + NONCE_LEN]).map_err(|_| VaultError::Damaged)?;
    let mut data = blob[1 + NONCE_LEN..].to_vec();
    let plain = cipher(key)?
        .open_in_place(nonce, Aad::from(binding(origin, username)), &mut data)
        .map_err(|_| VaultError::Damaged)?;
    String::from_utf8(plain.to_vec()).map_err(|_| VaultError::Damaged)
}

// ---------------------------------------------------------------------------
// Schlüssel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum VaultError {
    /// Es gibt noch keinen Schlüssel (und damit nichts zu lesen).
    NoKey,
    /// Es sind Passwörter gesichert, aber der Schlüssel dazu ist nicht mehr da.
    /// Ein neuer entsteht dann nicht von selbst – das würde den Fehler nur verdecken.
    KeyLost,
    /// macOS hat den Zugriff auf den Schlüsselbund nicht erlaubt.
    Denied(String),
    /// Der Eintrag passt nicht zum Schlüssel oder ist beschädigt.
    Damaged,
    Other(String),
}

const KEY_ACCOUNT: &str = "vault-key";
/// Der Schlüsselbund kennt den gesuchten Eintrag nicht.
const ERR_NOT_FOUND: i32 = -25300;

/// Ein Bau mit Prüfhilfe benutzt einen eigenen Eintrag – der echte bleibt unberührt.
fn key_service() -> &'static str {
    crate::dev::keychain_service()
}

fn hex_key(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

/// Schalter, die zusammen mit dem Schlüssel im Schlüsselbund liegen – dort
/// kann sie nur Zunder selbst ändern, anders als eine Einstellung in der
/// Datenbank, die jedes Programm umschreiben könnte.
const FLAG_FILL_WITHOUT_AUTH: u8 = 1;

/// Hält den Schlüssel, sobald er einmal geholt wurde. Damit fragt macOS je
/// Lauf der App höchstens einmal nach.
#[derive(Default)]
pub struct Vault {
    key: Option<[u8; 32]>,
    flags: u8,
    /// Der Schlüssel kam aus der Umgebung (nur mit Prüfhilfe): nichts geht in den Schlüsselbund.
    detached: bool,
}

/// Schreibt Schlüssel und Schalter in den Schlüsselbund.
fn store_key(key: &[u8; 32], flags: u8) -> Result<(), VaultError> {
    let mut data = key.to_vec();
    data.push(flags);
    let result = security_framework::passwords::set_generic_password(key_service(), KEY_ACCOUNT, &data)
        .map_err(|e| VaultError::Denied(e.to_string()));
    data.fill(0);
    result
}

impl Drop for Vault {
    fn drop(&mut self) {
        if let Some(key) = self.key.as_mut() {
            key.fill(0);
        }
    }
}

impl Vault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Holt den Schlüssel. Gibt es noch keinen, entsteht mit `create` ein
    /// neuer. Wichtig: Verweigert macOS den Zugriff, entsteht keiner – sonst
    /// wären die vorhandenen Passwörter mit einem Schlag unlesbar.
    fn key(&mut self, create: bool) -> Result<[u8; 32], VaultError> {
        if let Some(key) = self.key {
            return Ok(key);
        }
        // Nur mit Prüfhilfe: Schlüssel aus der Umgebung, ohne Schlüsselbund.
        if let Some(key) = crate::dev::var("ZUNDER_VAULT_KEY").as_deref().and_then(hex_key) {
            self.key = Some(key);
            self.detached = true;
            return Ok(key);
        }
        match security_framework::passwords::get_generic_password(key_service(), KEY_ACCOUNT) {
            Ok(mut bytes) => {
                // 32 Bytes Schlüssel, dahinter ein Byte mit Schaltern.
                if bytes.len() != 32 && bytes.len() != 33 {
                    return Err(VaultError::Damaged);
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes[..32]);
                self.flags = bytes.get(32).copied().unwrap_or(0);
                bytes.fill(0);
                self.key = Some(key);
                Ok(key)
            }
            Err(error) if error.code() == ERR_NOT_FOUND => {
                if !create {
                    return Err(VaultError::NoKey);
                }
                let mut key = [0u8; 32];
                SystemRandom::new()
                    .fill(&mut key)
                    .map_err(|_| VaultError::Other("keine Zufallszahlen".into()))?;
                store_key(&key, 0)?;
                self.flags = 0;
                self.key = Some(key);
                Ok(key)
            }
            Err(error) => Err(VaultError::Denied(error.to_string())),
        }
    }

    /// Wie `key`, zum Schreiben. `fresh` heißt: Es ist noch nichts gesichert –
    /// nur dann darf ein neuer Schlüssel entstehen. Fehlt er, obwohl schon
    /// Passwörter da sind, ist er verloren gegangen.
    fn key_for_writing(&mut self, fresh: bool) -> Result<[u8; 32], VaultError> {
        match self.key(fresh) {
            Err(VaultError::NoKey) => Err(VaultError::KeyLost),
            other => other,
        }
    }

    /// Verschlüsselt ein Passwort für genau diese Website und diesen Namen.
    pub fn seal(&mut self, origin: &str, username: &str, password: &str, fresh: bool) -> Result<Vec<u8>, VaultError> {
        let key = self.key_for_writing(fresh)?;
        seal_with(&key, origin, username, password)
    }

    /// Darf ohne Touch ID ausgefüllt werden? Ohne Schlüssel (oder ohne Zugang
    /// zu ihm) gilt die sichere Antwort: nein.
    pub fn fill_without_auth(&mut self) -> bool {
        self.key(false).is_ok() && self.flags & FLAG_FILL_WITHOUT_AUTH != 0
    }

    pub fn set_fill_without_auth(&mut self, allowed: bool, fresh: bool) -> Result<(), VaultError> {
        // Ohne Schlüssel gilt ohnehin „mit Abfrage“ – dafür muss keiner entstehen.
        if !allowed && matches!(self.key(false), Err(VaultError::NoKey)) {
            return Ok(());
        }
        let key = self.key_for_writing(fresh)?;
        let flags = if allowed {
            self.flags | FLAG_FILL_WITHOUT_AUTH
        } else {
            self.flags & !FLAG_FILL_WITHOUT_AUTH
        };
        if !self.detached {
            store_key(&key, flags)?;
        }
        self.flags = flags;
        Ok(())
    }

    pub fn open(&mut self, origin: &str, username: &str, blob: &[u8]) -> Result<String, VaultError> {
        let key = self.key(false)?;
        open_with(&key, origin, username, blob)
    }
}

// ---------------------------------------------------------------------------
// Passwort-Vorschlag
// ---------------------------------------------------------------------------

/// Zeichen ohne die leicht verwechselbaren (l, I, O, 0, 1).
const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Ein neues Passwort: 18 zufällige Zeichen in drei Gruppen, durch Bindestriche
/// getrennt – gut 100 Bit, und im Notfall noch abzutippen. Enthält immer
/// Klein- und Großbuchstaben, eine Ziffer und (mit dem Bindestrich) ein
/// Sonderzeichen, weil viele Seiten das verlangen.
pub fn generate_password() -> String {
    let rng = SystemRandom::new();
    // Größtes Vielfaches der Zeichenzahl unter 256 – so ist jedes Zeichen gleich wahrscheinlich.
    let limit = (256 / ALPHABET.len()) * ALPHABET.len();
    loop {
        let mut chars = Vec::with_capacity(18);
        while chars.len() < 18 {
            let mut buffer = [0u8; 32];
            if rng.fill(&mut buffer).is_err() {
                continue;
            }
            for byte in buffer {
                if (byte as usize) < limit && chars.len() < 18 {
                    chars.push(ALPHABET[byte as usize % ALPHABET.len()]);
                }
            }
        }
        let lower = chars.iter().any(u8::is_ascii_lowercase);
        let upper = chars.iter().any(u8::is_ascii_uppercase);
        let digit = chars.iter().any(u8::is_ascii_digit);
        if lower && upper && digit {
            let text = String::from_utf8(chars).unwrap_or_default();
            return format!("{}-{}-{}", &text[0..6], &text[6..12], &text[12..18]);
        }
    }
}

// ---------------------------------------------------------------------------
// CSV: Ein- und Ausfuhr
// ---------------------------------------------------------------------------

/// Zerlegt CSV nach RFC 4180: Felder in Anführungszeichen dürfen Kommas,
/// Zeilenumbrüche und (verdoppelt) Anführungszeichen enthalten.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            other => field.push(other),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    // Leerzeilen bedeuten nichts.
    rows.retain(|r| r.iter().any(|f| !f.is_empty()));
    rows
}

#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub url: String,
    pub username: String,
    pub password: String,
}

/// Liest die Passwort-Ausfuhr eines anderen Browsers oder Passwortmanagers.
/// Erkannt wird sie an der Kopfzeile (Chrome, Edge, Brave, Safari, Firefox,
/// Bitwarden, 1Password und alles, was Spalten für Adresse und Passwort nennt).
pub fn read_export(text: &str) -> Result<Vec<Imported>, &'static str> {
    let rows = parse_csv(text);
    let Some(header) = rows.first() else {
        return Err("empty");
    };
    let find = |names: &[&str]| {
        header
            .iter()
            .position(|h| names.contains(&h.trim().to_lowercase().as_str()))
    };
    let url = find(&["url", "login_uri", "website", "web site", "login url", "uri", "origin", "adresse"]);
    let username = find(&["username", "login_username", "user", "user name", "login", "email", "benutzername"]);
    let password = find(&["password", "login_password", "passwort"]);
    let (Some(url), Some(password)) = (url, password) else {
        return Err("format");
    };
    let cell = |row: &Vec<String>, index: Option<usize>| {
        index.and_then(|i| row.get(i)).cloned().unwrap_or_default()
    };
    let mut items = Vec::new();
    for row in rows.iter().skip(1) {
        let item = Imported {
            url: cell(row, Some(url)).trim().to_string(),
            username: cell(row, username).trim().to_string(),
            password: cell(row, Some(password)),
        };
        if !item.url.is_empty() && !item.password.is_empty() {
            items.push(item);
        }
    }
    Ok(items)
}

fn csv_field(value: &str) -> String {
    let plain = !value.is_empty()
        && !value.contains([',', '"', '\n', '\r'])
        && !value.starts_with(' ')
        && !value.ends_with(' ');
    if plain {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

/// Schreibt Anmeldungen in dem Format, das Chrome, Edge, Brave, Safari und
/// die gängigen Passwortmanager einlesen (name, url, username, password).
pub fn write_export(rows: &[(String, String, String)]) -> String {
    let mut out = String::from("name,url,username,password\n");
    for (origin, username, password) in rows {
        let name = site_of(origin).map(|s| s.host).unwrap_or_default();
        out.push_str(&format!(
            "{},{},{},{}\n",
            csv_field(&name),
            csv_field(origin),
            csv_field(username),
            csv_field(password)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_and_matching() {
        let site = site_of("https://www.Example.com/login?next=/").unwrap();
        assert_eq!(site.origin, "https://www.example.com");
        assert_eq!(site.key, "example.com");
        assert_eq!(site.host, "example.com");
        assert!(site.secure);

        let port = site_of("http://localhost:8080/a").unwrap();
        assert_eq!(port.origin, "http://localhost:8080");
        assert_eq!(port.key, "localhost:8080");
        assert!(!port.secure);
        // Der übliche Port gehört nicht zur Herkunft.
        assert_eq!(site_of("https://example.com:443/").unwrap().origin, "https://example.com");
        // „www.“ allein ist kein Vorsatz, der wegfallen darf.
        assert_eq!(site_of("https://www.de/").unwrap().key, "www.de");

        assert!(site_of("zunder://newtab/").is_none());
        assert!(site_of("file:///tmp/x.html").is_none());
        assert!(site_of("javascript:alert(1)").is_none());
        assert!(site_of("kein url").is_none());

        // Gleiche Website, mit und ohne „www.“.
        assert!(offered("https://example.com", &site));
        assert!(offered("https://www.example.com", &site_of("https://example.com/").unwrap()));
        // Andere Website, anderer Unterbereich, anderer Port: nie.
        assert!(!offered("https://example.org", &site));
        assert!(!offered("https://login.example.com", &site));
        assert!(!offered("https://example.com.evil.net", &site));
        assert!(!offered("https://example.com:8443", &site));
        // Verschlüsselt gesichert → nie auf der unverschlüsselten Seite.
        let plain = site_of("http://example.com/").unwrap();
        assert!(!offered("https://example.com", &plain));
        // Unverschlüsselt gesichert → auch auf der verschlüsselten Fassung.
        assert!(offered("http://example.com", &site));
        assert!(offered("http://example.com", &plain));
    }

    #[test]
    fn sealing_is_bound_to_site_and_name() {
        let key = [7u8; 32];
        let blob = seal_with(&key, "https://bank.example", "anna", "g3heim-Wort ü").unwrap();
        assert_eq!(open_with(&key, "https://bank.example", "anna", &blob).unwrap(), "g3heim-Wort ü");
        // Derselbe Text ergibt jedes Mal etwas anderes.
        assert_ne!(blob, seal_with(&key, "https://bank.example", "anna", "g3heim-Wort ü").unwrap());
        // Das Passwort steht nicht lesbar darin.
        assert!(!blob.windows(6).any(|w| w == b"g3heim"));
        // Umgehängt zu einer anderen Website oder einem anderen Namen: unlesbar.
        assert_eq!(open_with(&key, "https://evil.example", "anna", &blob), Err(VaultError::Damaged));
        assert_eq!(open_with(&key, "https://bank.example", "berta", &blob), Err(VaultError::Damaged));
        // Falscher Schlüssel, verändert, abgeschnitten.
        assert_eq!(open_with(&[8u8; 32], "https://bank.example", "anna", &blob), Err(VaultError::Damaged));
        let mut bent = blob.clone();
        let last = bent.len() - 1;
        bent[last] ^= 1;
        assert_eq!(open_with(&key, "https://bank.example", "anna", &bent), Err(VaultError::Damaged));
        assert_eq!(open_with(&key, "https://bank.example", "anna", &blob[..10]), Err(VaultError::Damaged));
        assert_eq!(open_with(&key, "https://bank.example", "anna", &[]), Err(VaultError::Damaged));
        // Auch ein leeres Passwort lässt sich hin- und zurückwandeln.
        let empty = seal_with(&key, "https://a.example", "", "").unwrap();
        assert_eq!(open_with(&key, "https://a.example", "", &empty).unwrap(), "");
    }

    #[test]
    fn generated_passwords() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let password = generate_password();
            assert_eq!(password.len(), 20);
            let groups: Vec<&str> = password.split('-').collect();
            assert_eq!(groups.len(), 3);
            assert!(groups.iter().all(|g| g.len() == 6));
            assert!(password.chars().any(|c| c.is_ascii_lowercase()));
            assert!(password.chars().any(|c| c.is_ascii_uppercase()));
            assert!(password.chars().any(|c| c.is_ascii_digit()));
            assert!(!password.contains(['l', 'I', 'O', '0', '1']));
            assert!(seen.insert(password));
        }
    }

    #[test]
    fn csv_round_trip() {
        let rows = vec![
            ("https://www.example.com".to_string(), "anna@example.com".to_string(), "ein,fach".to_string()),
            ("https://b.example".to_string(), String::new(), "mit \"Zitat\"\nund Umbruch".to_string()),
            ("http://localhost:8080".to_string(), " rand ".to_string(), "=SUMME(A1)".to_string()),
        ];
        let text = write_export(&rows);
        assert!(text.starts_with("name,url,username,password\n"));
        let back = read_export(&text).unwrap();
        assert_eq!(back.len(), 3);
        for (item, (origin, username, password)) in back.iter().zip(&rows) {
            assert_eq!(&item.url, origin);
            assert_eq!(item.username, username.trim());
            assert_eq!(&item.password, password);
        }
    }

    #[test]
    fn csv_from_other_browsers() {
        // Safari
        let safari = "Title,URL,Username,Password,Notes,OTPAuth\nBeispiel,https://example.com/,anna,pw1,,\n";
        let items = read_export(safari).unwrap();
        assert_eq!(items, vec![Imported { url: "https://example.com/".into(), username: "anna".into(), password: "pw1".into() }]);
        // Firefox: andere Reihenfolge, Felder in Anführungszeichen, Windows-Zeilenenden, BOM
        let firefox = "\u{feff}\"url\",\"username\",\"password\",\"httpRealm\"\r\n\"https://a.de\",\"u\",\"p,1\",\"\"\r\n\r\n";
        let items = read_export(firefox).unwrap();
        assert_eq!(items[0].password, "p,1");
        // Bitwarden
        let bitwarden = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,login,X,,,0,https://x.io/login,ich,geheim,\n";
        assert_eq!(read_export(bitwarden).unwrap()[0].username, "ich");
        // Zeilen ohne Adresse oder Passwort fallen weg.
        let partial = "url,username,password\n,anna,pw\nhttps://a.de,anna,\nhttps://b.de,,pw\n";
        assert_eq!(read_export(partial).unwrap().len(), 1);
        // Unbekanntes Format und leere Datei.
        assert_eq!(read_export("a,b,c\n1,2,3\n"), Err("format"));
        assert_eq!(read_export(""), Err("empty"));
    }

    #[test]
    fn key_from_hex() {
        assert_eq!(hex_key(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(hex_key("zu kurz"), None);
        assert_eq!(hex_key(&"zz".repeat(32)), None);
    }
}
