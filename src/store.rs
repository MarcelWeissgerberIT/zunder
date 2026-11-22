//! Dauerhafte Daten: Verlauf, Lesezeichen, Downloads, Einstellungen (SQLite).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Ordner eines Lesezeichens; leer heißt: ohne Ordner.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub folder: String,
}

/// Bringt den Namen eines Lesezeichen-Ordners in Form: ohne Steuerzeichen,
/// Leerraum zusammengefasst, höchstens 60 Zeichen. `None`, wenn nichts bleibt.
pub fn clean_folder(name: &str) -> Option<String> {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = name.chars().take(60).collect();
    (!name.is_empty()).then_some(name)
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadRow {
    pub id: i64,
    pub url: String,
    pub path: String,
    pub state: String,
    pub ts: i64,
    pub bytes: i64,
}

pub struct Store {
    db: Connection,
}

/// Bewertung eines Verlaufseintrags: häufig und kürzlich besucht steht oben.
const SCORE: &str = "((visits + 2.0 * typed) / (1.0 + (?now - last_visit) / 604800.0))";

fn like_escape(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    for c in term.chars() {
        if c == '\\' || c == '%' || c == '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Host ohne führendes "www." – Grundlage für Vervollständigung und Icons.
/// Ist das ein Seiten-Icon, wie Zunder es zeigt: ein kleines PNG als Text?
/// Adressen zum Nachladen gelten nicht – Zunders Oberfläche holt nichts aus
/// dem Netz.
pub fn is_icon(data: &str) -> bool {
    data.len() <= 12000
        && data
            .strip_prefix("data:image/png;base64,")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')))
}

/// Macht eine Datei nur für den eigenen Nutzer lesbar.
pub fn private_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// Macht einen Ordner nur für den eigenen Nutzer zugänglich.
pub fn private_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .map(|h| h.strip_prefix("www.").map(str::to_string).unwrap_or(h))
        .unwrap_or_default()
}

impl Store {
    pub fn open(dir: &Path) -> rusqlite::Result<Self> {
        let _ = std::fs::create_dir_all(dir);
        // Verlauf, Lesezeichen und die (verschlüsselten) Passwörter gehen nur
        // den eigenen Nutzer etwas an – auch auf einem Mac mit mehreren Konten.
        private_dir(dir);
        let db = Connection::open(dir.join("zunder.db"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS history (
                 id INTEGER PRIMARY KEY,
                 url TEXT NOT NULL UNIQUE,
                 title TEXT NOT NULL DEFAULT '',
                 host TEXT NOT NULL DEFAULT '',
                 visits INTEGER NOT NULL DEFAULT 0,
                 typed INTEGER NOT NULL DEFAULT 0,
                 last_visit INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS history_last ON history(last_visit DESC);
             CREATE INDEX IF NOT EXISTS history_host ON history(host);
             CREATE TABLE IF NOT EXISTS bookmarks (
                 id INTEGER PRIMARY KEY,
                 url TEXT NOT NULL UNIQUE,
                 title TEXT NOT NULL DEFAULT '',
                 created INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS favicons (
                 host TEXT PRIMARY KEY,
                 url TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS downloads (
                 id INTEGER PRIMARY KEY,
                 url TEXT NOT NULL,
                 path TEXT NOT NULL,
                 state TEXT NOT NULL,
                 ts INTEGER NOT NULL,
                 bytes INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS settings (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS logins (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 origin TEXT NOT NULL,
                 site TEXT NOT NULL,
                 username TEXT NOT NULL DEFAULT '',
                 secret BLOB NOT NULL,
                 created INTEGER NOT NULL,
                 changed INTEGER NOT NULL,
                 used INTEGER NOT NULL DEFAULT 0,
                 UNIQUE(origin, username)
             );
             CREATE INDEX IF NOT EXISTS logins_site ON logins(site);
             CREATE TABLE IF NOT EXISTS login_never (
                 site TEXT PRIMARY KEY,
                 created INTEGER NOT NULL
             );",
        )?;
        // Lesezeichen: Ordner und eine eigene Reihenfolge kamen später dazu.
        let has_folder = db
            .prepare("SELECT 1 FROM pragma_table_info('bookmarks') WHERE name = 'folder'")
            .and_then(|mut statement| statement.exists([]))
            .unwrap_or(false);
        if !has_folder {
            db.execute_batch(
                "ALTER TABLE bookmarks ADD COLUMN folder TEXT NOT NULL DEFAULT '';
                 ALTER TABLE bookmarks ADD COLUMN position INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS bookmark_folders (
                 name TEXT PRIMARY KEY,
                 position INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        // Downloads, die beim letzten Beenden noch liefen, sind abgebrochen.
        db.execute(
            "UPDATE downloads SET state = 'failed' WHERE state = 'active'",
            [],
        )?;
        for name in ["zunder.db", "zunder.db-wal", "zunder.db-shm"] {
            private_file(&dir.join(name));
        }
        Ok(Self { db })
    }

    // ----- Bekannte Seiten ---------------------------------------------------

    /// Die Seiten, die der Mensch am Browser kennt – für die Warnung vor
    /// täuschend ähnlichen Adressen. Liefert (Rechner, wichtig?), die
    /// wichtigsten zuerst, und alle Rechner, die je besucht wurden.
    /// Wichtig heißt: Lesezeichen, gesichertes Passwort oder oft besucht.
    pub fn known_hosts(&self) -> (Vec<(String, bool)>, Vec<String>) {
        let mut known: Vec<(String, bool)> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let column = |sql: &str| -> Vec<String> {
            let Ok(mut statement) = self.db.prepare(sql) else {
                return Vec::new();
            };
            let rows = statement.query_map([], |row| row.get::<_, String>(0));
            rows.map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        // Hier zählt der Rechner samt „www.“ – dorthin führt „Zur bekannten Seite“.
        for url in column("SELECT url FROM bookmarks ORDER BY created DESC LIMIT 5000") {
            let host = crate::shield::host_of(&url);
            if !host.is_empty() {
                known.push((host, true));
            }
        }
        for origin in column("SELECT DISTINCT origin FROM logins LIMIT 20000") {
            let host = crate::shield::host_of(&origin);
            if !host.is_empty() {
                known.push((host, true));
            }
        }
        if let Ok(mut statement) = self.db.prepare(
            "SELECT host, SUM(visits), SUM(typed) FROM history WHERE host != ''
             GROUP BY host ORDER BY SUM(visits) + 3 * SUM(typed) DESC LIMIT 50000",
        ) {
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
            });
            if let Ok(rows) = rows {
                for (host, visits, typed) in rows.flatten() {
                    // Einmal dort gewesen: bekannt genug, um nicht davor zu
                    // warnen. Mehrmals: bekannt genug, um damit zu vergleichen.
                    if visits >= 2 || typed >= 1 {
                        known.push((host.clone(), visits >= 8 || typed >= 2));
                    }
                    seen.push(host);
                }
            }
        }
        (known, seen)
    }

    // ----- Verlauf ---------------------------------------------------------

    pub fn record_visit(&self, url: &str, title: &str, typed: bool) {
        // Überlange Adressen und Titel gehören nicht in den Verlauf – sie
        // bremsten jede Suche darin.
        if url.len() > 2000 {
            return;
        }
        let title: String = title.chars().take(300).collect();
        let host = host_of(url);
        // Gezählt wird ein Besuch je Website höchstens alle 30 Sekunden (es sei
        // denn, die Adresse wurde eingetippt): Eine Seite, die sich in einer
        // Schleife neu lädt, schiebt sich so nicht in Vorschlägen und auf der
        // Startseite nach vorn.
        let recent: bool = self
            .db
            .query_row(
                "SELECT max(last_visit) FROM history WHERE host = ?1",
                params![host],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
            .is_some_and(|last| now() - last < 30);
        let counted = (typed || !recent) as i64;
        let _ = self.db.execute(
            "INSERT INTO history (url, title, host, visits, typed, last_visit)
             VALUES (?1, ?2, ?3, ?6, ?4, ?5)
             ON CONFLICT(url) DO UPDATE SET
                 visits = visits + ?6,
                 typed = typed + excluded.typed,
                 last_visit = excluded.last_visit,
                 title = CASE WHEN excluded.title != '' THEN excluded.title ELSE title END",
            params![url, title, host, typed as i64, now(), counted],
        );
    }

    pub fn update_title(&self, url: &str, title: &str) {
        if title.is_empty() {
            return;
        }
        let title: String = title.chars().take(300).collect();
        let _ = self.db.execute(
            "UPDATE history SET title = ?2 WHERE url = ?1",
            params![url, title],
        );
        let _ = self.db.execute(
            "UPDATE bookmarks SET title = ?2 WHERE url = ?1 AND title = ''",
            params![url, title],
        );
    }

    fn entries(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Vec<Entry> {
        let Ok(mut stmt) = self.db.prepare_cached(sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map(args, |row| {
            Ok(Entry {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                ts: row.get(3)?,
                icon: row.get(4)?,
                folder: String::new(),
            })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Treffer für die Adressleiste: jedes Wort muss in Adresse oder Titel vorkommen.
    pub fn search_history(&self, query: &str, limit: usize) -> Vec<Entry> {
        let terms: Vec<String> = query
            .split_whitespace()
            .take(4)
            .map(|t| format!("%{}%", like_escape(&t.to_lowercase())))
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut sql = String::from(
            "SELECT h.id, h.url, h.title, h.last_visit, f.url FROM history h
             LEFT JOIN favicons f ON f.host = h.host WHERE 1=1",
        );
        for i in 0..terms.len() {
            sql.push_str(&format!(
                " AND (lower(h.url) LIKE ?{n} ESCAPE '\\' OR lower(h.title) LIKE ?{n} ESCAPE '\\')",
                n = i + 1
            ));
        }
        sql.push_str(&format!(
            " ORDER BY {} DESC LIMIT {}",
            SCORE.replace("?now", &format!("?{}", terms.len() + 1)),
            limit
        ));
        let now = now();
        let mut args: Vec<&dyn rusqlite::ToSql> = terms.iter().map(|t| t as &dyn rusqlite::ToSql).collect();
        args.push(&now);
        self.entries(&sql, &args)
    }

    /// Beste Vervollständigung für angefangene Adressen ("git" -> "github.com").
    pub fn autocomplete(&self, typed: &str) -> Option<String> {
        let typed = typed.to_lowercase();
        if typed.is_empty() || typed.contains(char::is_whitespace) || typed.contains("://") {
            return None;
        }
        let host_part = typed.split('/').next().unwrap_or("");
        if host_part.is_empty() {
            return None;
        }
        let pattern = format!("{}%", like_escape(host_part));
        // Von selbst ergänzt wird nur zu Seiten, zu denen man schon einmal
        // ausdrücklich wollte: eingetippt oder als Lesezeichen gemerkt. Eine
        // Seite, auf die man bloß geraten ist, kann sich nicht in die Eingabe
        // drängen – auch nicht mit noch so vielen Aufrufen.
        let sql = format!(
            "SELECT url FROM history WHERE host LIKE ?1 ESCAPE '\\'
             AND (typed > 0 OR url IN (SELECT url FROM bookmarks))
             ORDER BY {} DESC LIMIT 40",
            SCORE.replace("?now", "?2")
        );
        let mut stmt = self.db.prepare_cached(&sql).ok()?;
        let urls: Vec<String> = stmt
            .query_map(params![pattern, now()], |row| row.get::<_, String>(0))
            .ok()?
            .flatten()
            .collect();
        if !typed.contains('/') {
            // Nur der Host wird vervollständigt.
            return urls
                .iter()
                .map(|u| host_of(u))
                .find(|h| h.starts_with(&typed) && h.len() > typed.len());
        }
        // Mit Pfad: gegen "host/pfad" prüfen.
        for url in &urls {
            let Ok(parsed) = url::Url::parse(url) else {
                continue;
            };
            let mut plain = host_of(url);
            plain.push_str(parsed.path());
            if let Some(q) = parsed.query() {
                plain.push('?');
                plain.push_str(q);
            }
            if plain.to_lowercase().starts_with(&typed) && plain.len() > typed.len() {
                return Some(plain);
            }
        }
        None
    }

    /// Verlauf für die Seitenleiste, neueste zuerst.
    pub fn history_page(&self, query: &str, limit: usize) -> Vec<Entry> {
        if query.trim().is_empty() {
            let sql = format!(
                "SELECT h.id, h.url, h.title, h.last_visit, f.url FROM history h
                 LEFT JOIN favicons f ON f.host = h.host
                 ORDER BY h.last_visit DESC LIMIT {limit}"
            );
            return self.entries(&sql, &[]);
        }
        let pattern = format!("%{}%", like_escape(&query.trim().to_lowercase()));
        let sql = format!(
            "SELECT h.id, h.url, h.title, h.last_visit, f.url FROM history h
             LEFT JOIN favicons f ON f.host = h.host
             WHERE lower(h.url) LIKE ?1 ESCAPE '\\' OR lower(h.title) LIKE ?1 ESCAPE '\\'
             ORDER BY h.last_visit DESC LIMIT {limit}"
        );
        self.entries(&sql, &[&pattern as &dyn rusqlite::ToSql])
    }

    pub fn delete_history(&self, id: i64) {
        let _ = self.db.execute("DELETE FROM history WHERE id = ?1", params![id]);
    }

    /// Löscht Verlauf, Download-Liste und die gemerkten Seiten-Icons.
    pub fn clear_history(&self) {
        let _ = self.db.execute("DELETE FROM history", []);
        let _ = self.db.execute("DELETE FROM downloads WHERE state != 'active'", []);
        let _ = self.db.execute("DELETE FROM favicons", []);
    }

    /// Meistbesuchte Seiten für die Startseite (eine je Host).
    pub fn top_sites(&self, limit: usize) -> Vec<Entry> {
        let sql = format!(
            "SELECT h.id, h.url, h.title, h.last_visit, f.url FROM history h
             LEFT JOIN favicons f ON f.host = h.host
             WHERE h.host != '' AND h.id IN (
                 SELECT id FROM history h2 WHERE h2.host = h.host
                 ORDER BY (h2.visits + 2.0 * h2.typed) DESC, length(h2.url) ASC LIMIT 1
             )
             ORDER BY (SELECT sum(visits + 2.0 * typed) FROM history h3 WHERE h3.host = h.host) DESC
             LIMIT {limit}"
        );
        self.entries(&sql, &[])
    }

    // ----- Seiten-Icons ----------------------------------------------------

    pub fn set_favicon(&self, page_url: &str, icon_url: &str) {
        let host = host_of(page_url);
        if host.is_empty() {
            return;
        }
        let _ = self.db.execute(
            "INSERT INTO favicons (host, url) VALUES (?1, ?2)
             ON CONFLICT(host) DO UPDATE SET url = excluded.url",
            params![host, icon_url],
        );
    }

    // ----- Lesezeichen -----------------------------------------------------

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.db
            .query_row("SELECT 1 FROM bookmarks WHERE url = ?1", params![url], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    /// Die höchste vergebene Position in einem Ordner – Neues kommt darüber.
    fn top_position(&self, folder: &str) -> i64 {
        self.db
            .query_row("SELECT max(position) FROM bookmarks WHERE folder = ?1", params![folder], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    pub fn add_bookmark(&self, url: &str, title: &str) {
        let _ = self.db.execute(
            "INSERT OR IGNORE INTO bookmarks (url, title, created, folder, position) VALUES (?1, ?2, ?3, '', ?4)",
            params![url, title, now(), self.top_position("") + 1],
        );
    }

    /// Legt viele Lesezeichen auf einmal an – in der gegebenen Reihenfolge, je
    /// mit Adresse, Titel und Ordner (leer: ohne). Liefert (neu angelegt, schon
    /// vorhanden).
    pub fn add_bookmarks(&self, items: &[(String, String, String)]) -> (usize, usize) {
        let _ = self.db.execute_batch("BEGIN");
        let (mut added, mut known) = (0, 0);
        let now = now();
        for (url, title, folder) in items {
            let folder = clean_folder(folder).unwrap_or_default();
            if !folder.is_empty() {
                let _ = self.db.execute("INSERT OR IGNORE INTO bookmark_folders (name) VALUES (?1)", params![folder]);
            }
            // Alle tragen denselben Zeitpunkt und dieselbe Position; die Liste
            // ordnet Gleiches nach der Reihenfolge des Anlegens – so bleibt die
            // Reihenfolge der Quelle erhalten.
            let inserted = self.db.execute(
                "INSERT OR IGNORE INTO bookmarks (url, title, created, folder) VALUES (?1, ?2, ?3, ?4)",
                params![url, title, now, folder],
            );
            match inserted {
                Ok(1) => added += 1,
                _ => known += 1,
            }
        }
        let _ = self.db.execute_batch("COMMIT");
        (added, known)
    }

    pub fn remove_bookmark(&self, url: &str) {
        let _ = self.db.execute("DELETE FROM bookmarks WHERE url = ?1", params![url]);
    }

    pub fn delete_bookmark(&self, id: i64) {
        let _ = self.db.execute("DELETE FROM bookmarks WHERE id = ?1", params![id]);
    }

    /// Ändert Titel und Adresse eines Lesezeichens. `false`, wenn es die neue
    /// Adresse schon als anderes Lesezeichen gibt.
    pub fn update_bookmark(&self, id: i64, title: &str, url: &str) -> bool {
        let title: String = title.chars().filter(|c| !c.is_control()).take(300).collect();
        self.db
            .execute("UPDATE bookmarks SET title = ?2, url = ?3 WHERE id = ?1", params![id, title.trim(), url])
            .is_ok_and(|changed| changed == 1)
    }

    /// Verschiebt ein Lesezeichen in einen Ordner (leer: ohne Ordner) und dort
    /// vor ein anderes – oder ans Ende.
    pub fn move_bookmark(&self, id: i64, folder: &str, before: Option<i64>) -> bool {
        if !folder.is_empty() && !self.bookmark_folders().iter().any(|name| name == folder) {
            return false;
        }
        let mut order: Vec<i64> = {
            let Ok(mut statement) = self
                .db
                .prepare("SELECT id FROM bookmarks WHERE folder = ?1 AND id != ?2 ORDER BY position DESC, created DESC, id")
            else {
                return false;
            };
            let rows = statement.query_map(params![folder, id], |row| row.get::<_, i64>(0));
            rows.map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        let at = before.and_then(|before| order.iter().position(|other| *other == before)).unwrap_or(order.len());
        order.insert(at, id);
        let _ = self.db.execute_batch("BEGIN");
        let moved = self.db.execute("UPDATE bookmarks SET folder = ?2 WHERE id = ?1", params![id, folder]);
        // Der erste bekommt die höchste Position.
        let count = order.len() as i64;
        for (index, entry) in order.iter().enumerate() {
            let _ = self.db.execute("UPDATE bookmarks SET position = ?2 WHERE id = ?1", params![entry, count - index as i64]);
        }
        let _ = self.db.execute_batch("COMMIT");
        moved.is_ok_and(|changed| changed == 1)
    }

    /// Die Ordner der Lesezeichen, in ihrer Reihenfolge.
    pub fn bookmark_folders(&self) -> Vec<String> {
        let Ok(mut statement) = self.db.prepare("SELECT name FROM bookmark_folders ORDER BY position DESC, name COLLATE NOCASE") else {
            return Vec::new();
        };
        let rows = statement.query_map([], |row| row.get::<_, String>(0));
        rows.map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn add_folder(&self, name: &str) -> bool {
        let Some(name) = clean_folder(name) else {
            return false;
        };
        self.db
            .execute("INSERT OR IGNORE INTO bookmark_folders (name) VALUES (?1)", params![name])
            .is_ok_and(|added| added == 1)
    }

    /// Benennt einen Ordner um; seine Lesezeichen ziehen mit.
    pub fn rename_folder(&self, old: &str, new: &str) -> bool {
        let Some(new) = clean_folder(new) else {
            return false;
        };
        if new == old || self.bookmark_folders().iter().any(|name| *name == new) {
            return false;
        }
        let _ = self.db.execute_batch("BEGIN");
        let renamed = self.db.execute("UPDATE bookmark_folders SET name = ?2 WHERE name = ?1", params![old, new]);
        let _ = self.db.execute("UPDATE bookmarks SET folder = ?2 WHERE folder = ?1", params![old, new]);
        let _ = self.db.execute_batch("COMMIT");
        renamed.is_ok_and(|changed| changed == 1)
    }

    /// Löst einen Ordner auf. Seine Lesezeichen bleiben – ohne Ordner.
    pub fn delete_folder(&self, name: &str) {
        let _ = self.db.execute_batch("BEGIN");
        let _ = self.db.execute("UPDATE bookmarks SET folder = '' WHERE folder = ?1", params![name]);
        let _ = self.db.execute("DELETE FROM bookmark_folders WHERE name = ?1", params![name]);
        let _ = self.db.execute_batch("COMMIT");
    }

    pub fn bookmarks(&self, query: &str, limit: usize) -> Vec<Entry> {
        // Der Host steht nicht in der Tabelle – das Icon wird unten nachgeschlagen.
        let columns = "id, url, title, created, folder";
        let order = "ORDER BY position DESC, created DESC, id";
        let pattern = (!query.trim().is_empty()).then(|| format!("%{}%", like_escape(&query.trim().to_lowercase())));
        let sql = match &pattern {
            None => format!("SELECT {columns} FROM bookmarks {order} LIMIT {limit}"),
            Some(_) => format!(
                "SELECT {columns} FROM bookmarks
                 WHERE lower(url) LIKE ?1 ESCAPE '\\' OR lower(title) LIKE ?1 ESCAPE '\\'
                 {order} LIMIT {limit}"
            ),
        };
        let Ok(mut statement) = self.db.prepare_cached(&sql) else {
            return Vec::new();
        };
        let read = |row: &rusqlite::Row<'_>| {
            Ok(Entry {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get(2)?,
                ts: row.get(3)?,
                icon: None,
                folder: row.get(4)?,
            })
        };
        let rows = match &pattern {
            Some(pattern) => statement.query_map(params![pattern], read),
            None => statement.query_map([], read),
        };
        let mut list: Vec<Entry> = rows.map(|rows| rows.flatten().collect()).unwrap_or_default();
        for entry in &mut list {
            entry.icon = self.favicon_for(&entry.url);
        }
        list
    }

    pub fn favicon_for(&self, page_url: &str) -> Option<String> {
        let host = host_of(page_url);
        if host.is_empty() {
            return None;
        }
        self.db
            .query_row("SELECT url FROM favicons WHERE host = ?1", params![host], |row| row.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
            // Aus früheren Fassungen können noch Adressen dastehen – die zeigt niemand mehr.
            .filter(|icon| is_icon(icon))
    }

    // ----- Downloads -------------------------------------------------------

    pub fn add_download(&self, url: &str, path: &str) -> i64 {
        let _ = self.db.execute(
            "INSERT INTO downloads (url, path, state, ts) VALUES (?1, ?2, 'active', ?3)",
            params![url, path, now()],
        );
        self.db.last_insert_rowid()
    }

    pub fn finish_download(&self, id: i64, ok: bool, bytes: i64) {
        let _ = self.db.execute(
            "UPDATE downloads SET state = ?2, bytes = ?3 WHERE id = ?1",
            params![id, if ok { "done" } else { "failed" }, bytes],
        );
    }

    pub fn downloads(&self, limit: usize) -> Vec<DownloadRow> {
        let sql = format!(
            "SELECT id, url, path, state, ts, bytes FROM downloads ORDER BY id DESC LIMIT {limit}"
        );
        let Ok(mut stmt) = self.db.prepare_cached(&sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |row| {
            Ok(DownloadRow {
                id: row.get(0)?,
                url: row.get(1)?,
                path: row.get(2)?,
                state: row.get(3)?,
                ts: row.get(4)?,
                bytes: row.get(5)?,
            })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn download_path(&self, id: i64) -> Option<String> {
        self.db
            .query_row("SELECT path FROM downloads WHERE id = ?1", params![id], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn clear_downloads(&self) {
        let _ = self.db.execute("DELETE FROM downloads WHERE state != 'active'", []);
    }

    // ----- Einstellungen ---------------------------------------------------

    pub fn setting(&self, key: &str) -> Option<String> {
        self.db
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) {
        let _ = self.db.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        );
    }

    // ----- Anmeldungen (Passwort-Tresor) ------------------------------------
    //
    // Hier liegt nur die Ablage. Das Passwort kommt verschlüsselt an und geht
    // verschlüsselt hinaus – siehe `vault.rs`.

    fn login_rows(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Vec<LoginRow> {
        let Ok(mut stmt) = self.db.prepare_cached(sql) else {
            return Vec::new();
        };
        let rows = stmt.query_map(args, |row| {
            Ok(LoginRow {
                id: row.get(0)?,
                origin: row.get(1)?,
                username: row.get(2)?,
                secret: row.get(3)?,
                changed: row.get(4)?,
                used: row.get(5)?,
            })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Anmeldungen einer Website (Schlüssel aus `vault::site_of`), zuletzt benutzte zuerst.
    pub fn logins_for_site(&self, site: &str) -> Vec<LoginRow> {
        self.login_rows(
            &format!("SELECT {LOGIN_COLUMNS} FROM logins WHERE site = ?1 ORDER BY used DESC, changed DESC, id DESC"),
            &[&site as &dyn rusqlite::ToSql],
        )
    }

    pub fn login(&self, id: i64) -> Option<LoginRow> {
        self.login_rows(
            &format!("SELECT {LOGIN_COLUMNS} FROM logins WHERE id = ?1"),
            &[&id as &dyn rusqlite::ToSql],
        )
        .into_iter()
        .next()
    }

    /// Alle Anmeldungen für die Verwaltung, nach Website sortiert.
    pub fn logins(&self, query: &str, limit: usize) -> Vec<LoginRow> {
        if query.trim().is_empty() {
            return self.login_rows(
                &format!("SELECT {LOGIN_COLUMNS} FROM logins ORDER BY site, username LIMIT {limit}"),
                &[],
            );
        }
        let pattern = format!("%{}%", like_escape(&query.trim().to_lowercase()));
        self.login_rows(
            &format!(
                "SELECT {LOGIN_COLUMNS} FROM logins
                 WHERE lower(origin) LIKE ?1 ESCAPE '\\' OR lower(username) LIKE ?1 ESCAPE '\\'
                 ORDER BY site, username LIMIT {limit}"
            ),
            &[&pattern as &dyn rusqlite::ToSql],
        )
    }

    pub fn login_count(&self) -> i64 {
        self.db
            .query_row("SELECT count(*) FROM logins", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// Legt eine Anmeldung an oder ersetzt das Passwort einer vorhandenen
    /// (gleiche Website, gleicher Name). Liefert ihre Nummer.
    pub fn save_login(&self, origin: &str, site: &str, username: &str, secret: &[u8]) -> Option<i64> {
        let now = now();
        self.db
            .query_row(
                "INSERT INTO logins (origin, site, username, secret, created, changed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(origin, username) DO UPDATE SET secret = excluded.secret, changed = excluded.changed
                 RETURNING id",
                params![origin, site, username, secret, now],
                |row| row.get(0),
            )
            .ok()
    }

    /// Ändert Name und Passwort einer Anmeldung. Schlägt fehl, wenn es den
    /// neuen Namen für diese Website schon gibt.
    pub fn update_login(&self, id: i64, username: &str, secret: &[u8]) -> bool {
        self.db
            .execute(
                "UPDATE logins SET username = ?2, secret = ?3, changed = ?4 WHERE id = ?1",
                params![id, username, secret, now()],
            )
            .is_ok_and(|changed| changed == 1)
    }

    pub fn login_exists(&self, origin: &str, username: &str) -> bool {
        self.db
            .query_row(
                "SELECT 1 FROM logins WHERE origin = ?1 AND username = ?2",
                params![origin, username],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    /// Hängt eine Anmeldung an eine andere Herkunft derselben Website um
    /// (von http auf https). Schlägt fehl, wenn es sie dort schon gibt.
    pub fn rebind_login(&self, id: i64, origin: &str, site: &str, secret: &[u8]) -> bool {
        self.db
            .execute(
                "UPDATE logins SET origin = ?2, site = ?3, secret = ?4, changed = ?5 WHERE id = ?1",
                params![id, origin, site, secret, now()],
            )
            .is_ok_and(|changed| changed == 1)
    }

    /// Viele Schreibvorgänge am Stück (Einfuhr): erst am Ende festschreiben.
    pub fn begin(&self) {
        let _ = self.db.execute_batch("BEGIN");
    }

    pub fn commit(&self) {
        let _ = self.db.execute_batch("COMMIT");
    }

    pub fn touch_login(&self, id: i64) {
        let _ = self.db.execute("UPDATE logins SET used = ?2 WHERE id = ?1", params![id, now()]);
    }

    pub fn delete_login(&self, id: i64) {
        let _ = self.db.execute("DELETE FROM logins WHERE id = ?1", params![id]);
    }

    /// Websites, für die nie nach dem Sichern gefragt werden soll.
    pub fn never_save(&self, site: &str) {
        let _ = self.db.execute(
            "INSERT OR IGNORE INTO login_never (site, created) VALUES (?1, ?2)",
            params![site, now()],
        );
    }

    pub fn is_never_save(&self, site: &str) -> bool {
        self.db
            .query_row("SELECT 1 FROM login_never WHERE site = ?1", params![site], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    pub fn never_save_list(&self) -> Vec<String> {
        let Ok(mut stmt) = self.db.prepare_cached("SELECT site FROM login_never ORDER BY site") else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |row| row.get::<_, String>(0));
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn allow_save(&self, site: &str) {
        let _ = self.db.execute("DELETE FROM login_never WHERE site = ?1", params![site]);
    }
}

const LOGIN_COLUMNS: &str = "id, origin, username, secret, changed, used";

/// Eine gesicherte Anmeldung. `secret` ist das verschlüsselte Passwort.
#[derive(Debug, Clone)]
pub struct LoginRow {
    pub id: i64,
    pub origin: String,
    pub username: String,
    pub secret: Vec<u8>,
    pub changed: i64,
    pub used: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("zunder-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Store::open(&dir).unwrap(), dir)
    }

    #[test]
    fn history_search_and_completion() {
        let (store, dir) = temp_store("history");
        store.record_visit("https://www.example.com/", "Beispiel", true);
        store.record_visit("https://www.example.com/", "Beispiel", false);
        store.record_visit("https://github.com/rust-lang/rust", "Rust auf GitHub", false);
        store.record_visit("https://exotisch.de/100%_sicher", "Sonderzeichen", false);

        assert_eq!(store.autocomplete("exa").as_deref(), Some("example.com"));
        // Von selbst ergänzt wird nur zu Seiten, zu denen man ausdrücklich wollte:
        // Auf github.com ist man bisher nur geraten.
        assert_eq!(store.autocomplete("git"), None);
        store.add_bookmark("https://github.com/rust-lang/rust", "Rust");
        assert_eq!(store.autocomplete("git").as_deref(), Some("github.com"));
        assert_eq!(
            store.autocomplete("github.com/ru").as_deref(),
            Some("github.com/rust-lang/rust")
        );
        // Eine Seite, die sich in Schleife neu lädt, zählt nicht jedes Mal als Besuch …
        for _ in 0..50 {
            store.record_visit("https://exotisch.de/schleife", "", false);
        }
        let visits: i64 = store
            .db
            .query_row("SELECT sum(visits) FROM history WHERE host = 'exotisch.de'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(visits, 1);
        // … und drängt sich auch nicht in die Eingabe.
        assert_eq!(store.autocomplete("exo"), None);
        // Überlange Adressen kommen gar nicht in den Verlauf, Titel werden gekürzt.
        store.record_visit(&format!("https://lang.example/{}", "a".repeat(3000)), "x", false);
        store.record_visit("https://lang.example/", &"t".repeat(5000), true);
        let longest: i64 = store.db.query_row("SELECT max(length(title)) FROM history", [], |row| row.get(0)).unwrap();
        assert_eq!(longest, 300);
        assert_eq!(store.search_history("lang.example", 5).len(), 1);
        assert_eq!(store.autocomplete("gibtsnicht"), None);
        assert_eq!(store.autocomplete("zwei worte"), None);

        let hits = store.search_history("rust github", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://github.com/rust-lang/rust");
        // Platzhalterzeichen der Datenbank werden wörtlich gesucht.
        assert_eq!(store.search_history("100%_", 5).len(), 1);
        assert_eq!(store.search_history("%", 5).len(), 1);

        assert_eq!(store.top_sites(10).len(), 4);
        store.clear_history();
        assert!(store.history_page("", 10).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bookmarks_downloads_settings() {
        let (store, dir) = temp_store("misc");
        assert!(!store.is_bookmarked("https://a.de/"));
        store.add_bookmark("https://a.de/", "A");
        store.add_bookmark("https://a.de/", "A noch einmal");
        assert!(store.is_bookmarked("https://a.de/"));
        assert_eq!(store.bookmarks("", 10).len(), 1);
        store.remove_bookmark("https://a.de/");
        assert!(!store.is_bookmarked("https://a.de/"));

        let id = store.add_download("https://a.de/x.zip", "/tmp/x.zip");
        assert_eq!(store.downloads(10)[0].state, "active");
        store.finish_download(id, true, 42);
        assert_eq!(store.downloads(10)[0].state, "done");
        assert_eq!(store.download_path(id).as_deref(), Some("/tmp/x.zip"));

        assert_eq!(store.setting("engine"), None);
        store.set_setting("engine", "duckduckgo");
        store.set_setting("engine", "brave");
        assert_eq!(store.setting("engine").as_deref(), Some("brave"));

        // Ein Seiten-Icon ist ein kleines Bild als Text – eine Adresse zum
        // Nachladen zeigt Zunder nicht mehr, auch keine aus früheren Fassungen.
        store.set_favicon("https://www.a.de/seite", "https://a.de/icon.png");
        assert_eq!(store.favicon_for("https://a.de/"), None);
        store.set_favicon("https://www.a.de/seite", "data:image/png;base64,iVBORw0KGgo=");
        assert_eq!(store.favicon_for("https://a.de/").as_deref(), Some("data:image/png;base64,iVBORw0KGgo="));
        assert!(!is_icon("data:image/svg+xml;base64,PHN2Zz4="));
        assert!(!is_icon("data:image/png;base64,"));
        assert!(!is_icon("data:image/png;base64,abc\"><script>"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn logins_and_bulk_bookmarks() {
        let (store, dir) = temp_store("logins");
        assert_eq!(store.login_count(), 0);
        let first = store.save_login("https://a.de", "a.de", "anna", b"eins").unwrap();
        let other = store.save_login("https://a.de", "a.de", "berta", b"zwei").unwrap();
        // Gleiche Website, gleicher Name: Das Passwort wird ersetzt, kein zweiter Eintrag.
        assert_eq!(store.save_login("https://a.de", "a.de", "anna", b"drei"), Some(first));
        assert_eq!(store.login_count(), 2);
        assert_eq!(store.login(first).unwrap().secret, b"drei");
        store.save_login("https://b.de", "b.de", "", b"vier");

        store.touch_login(other);
        let here = store.logins_for_site("a.de");
        assert_eq!(here.len(), 2);
        // Zuletzt benutzt steht vorn.
        assert_eq!(here[0].username, "berta");
        assert!(store.logins_for_site("gibtsnicht.de").is_empty());

        assert_eq!(store.logins("", 10).len(), 3);
        assert_eq!(store.logins("BERT", 10).len(), 1);
        assert_eq!(store.logins("b.de", 10).len(), 1);
        assert_eq!(store.logins("%", 10).len(), 0);

        // Umbenennen geht – aber nicht auf einen Namen, den es dort schon gibt.
        assert!(store.update_login(first, "anne", b"fuenf"));
        assert!(!store.update_login(first, "berta", b"sechs"));
        assert_eq!(store.login(first).unwrap().username, "anne");
        store.delete_login(first);
        assert!(store.login(first).is_none());

        assert!(!store.is_never_save("a.de"));
        store.never_save("a.de");
        store.never_save("a.de");
        assert!(store.is_never_save("a.de"));
        assert_eq!(store.never_save_list(), vec!["a.de".to_string()]);
        store.allow_save("a.de");
        assert!(!store.is_never_save("a.de"));

        // Viele Lesezeichen auf einmal: Vorhandenes zählt als bekannt, die Reihenfolge bleibt.
        store.add_bookmark("https://zwei.de/", "Zwei");
        let items = vec![
            ("https://eins.de/".to_string(), "Eins".to_string(), String::new()),
            ("https://zwei.de/".to_string(), "Zwei neu".to_string(), String::new()),
            ("https://drei.de/".to_string(), "Drei".to_string(), "  Arbeit \n".to_string()),
        ];
        assert_eq!(store.add_bookmarks(&items), (2, 1));
        let urls: Vec<String> = store.bookmarks("", 10).into_iter().map(|b| b.url).collect();
        assert_eq!(urls.iter().position(|u| u == "https://eins.de/") < urls.iter().position(|u| u == "https://drei.de/"), true);
        // Der Ordner aus der Einfuhr ist angelegt, sein Name in Form gebracht.
        assert_eq!(store.bookmark_folders(), vec!["Arbeit".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bookmark_folders_order_and_editing() {
        let (store, dir) = temp_store("folders");
        for (url, title) in [("https://a.de/", "A"), ("https://b.de/", "B"), ("https://c.de/", "C")] {
            store.add_bookmark(url, title);
        }
        let order = |folder: &str| -> Vec<String> {
            store.bookmarks("", 50).into_iter().filter(|b| b.folder == folder).map(|b| b.title).collect()
        };
        let id = |title: &str| store.bookmarks("", 50).into_iter().find(|b| b.title == title).unwrap().id;
        // Das Neueste steht vorn.
        assert_eq!(order(""), ["C", "B", "A"]);
        // Umsortieren: A vor C, B ans Ende.
        assert!(store.move_bookmark(id("A"), "", Some(id("C"))));
        assert_eq!(order(""), ["A", "C", "B"]);
        assert!(store.move_bookmark(id("C"), "", None));
        assert_eq!(order(""), ["A", "B", "C"]);
        // Ordner: anlegen, hineinlegen, umbenennen, auflösen.
        assert!(store.add_folder("  Lesen  "));
        assert!(!store.add_folder("Lesen"));
        assert!(!store.add_folder("  \u{7}  "));
        assert!(!store.move_bookmark(id("B"), "Gibt es nicht", None));
        assert!(store.move_bookmark(id("B"), "Lesen", None));
        assert!(store.move_bookmark(id("C"), "Lesen", Some(id("B"))));
        assert_eq!(order("Lesen"), ["C", "B"]);
        assert_eq!(order(""), ["A"]);
        assert!(store.rename_folder("Lesen", "Später lesen"));
        assert!(!store.rename_folder("Später lesen", "Später lesen"));
        assert_eq!(order("Später lesen"), ["C", "B"]);
        // Ein neues Lesezeichen kommt ohne Ordner und steht dort vorn.
        store.add_bookmark("https://d.de/", "D");
        assert_eq!(order(""), ["D", "A"]);
        // Titel und Adresse ändern – aber nicht auf eine Adresse, die es schon gibt.
        assert!(store.update_bookmark(id("A"), "A neu", "https://a.de/neu"));
        assert!(!store.update_bookmark(id("D"), "D", "https://a.de/neu"));
        assert!(store.is_bookmarked("https://a.de/neu") && !store.is_bookmarked("https://a.de/"));
        store.delete_folder("Später lesen");
        assert!(store.bookmark_folders().is_empty());
        assert_eq!(store.bookmarks("", 50).iter().filter(|b| b.folder.is_empty()).count(), 4);
        let _ = std::fs::remove_dir_all(dir);
    }
}
