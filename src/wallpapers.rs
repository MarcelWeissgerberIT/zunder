//! Live-Hintergründe der Startseite: kurze, nahtlos wiederholte 3D-Szenen.
//!
//! Die Dateien liegen in der App unter `Contents/Resources/wallpapers` (je
//! Hintergrund ein Video, ein Standbild und eine Vorschau; erzeugt mit
//! `scripts/build_wallpapers.sh`). Ausgeliefert werden sie nur an die
//! Startseite, über `zunder://newtab/wp/<datei>` – und nur Dateien aus der
//! festen Liste unten.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{json, Value};
use wry::http::{Request, Response};

use crate::i18n::t;

pub struct Wallpaper {
    pub id: &'static str,
    /// Schlüssel des Anzeigenamens in `i18n`.
    name: &'static str,
    /// Helles Motiv: Die Schrift der Startseite bleibt dann dunkel.
    light: bool,
}

pub const WALLPAPERS: &[Wallpaper] = &[
    Wallpaper { id: "glut", name: "wp_glut", light: false },
    Wallpaper { id: "nordlicht", name: "wp_nordlicht", light: false },
    Wallpaper { id: "tiefsee", name: "wp_tiefsee", light: false },
    Wallpaper { id: "kristall", name: "wp_kristall", light: true },
    Wallpaper { id: "duenen", name: "wp_duenen", light: true },
    Wallpaper { id: "wolken", name: "wp_wolken", light: true },
];

/// Der Ordner mit den Dateien – oder `None`, wenn die App ohne sie gebaut wurde.
fn dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        if let Some(dir) = crate::dev::var_os("ZUNDER_WALLPAPER_DIR") {
            return Some(PathBuf::from(dir));
        }
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?.parent()?.join("Resources").join("wallpapers");
        dir.is_dir().then_some(dir)
    })
    .as_ref()
}

/// Gibt es diesen Hintergrund – in der Liste und auf der Platte?
pub fn exists(id: &str) -> bool {
    WALLPAPERS.iter().any(|w| w.id == id)
        && dir().is_some_and(|dir| dir.join(format!("{id}.jpg")).is_file())
}

/// Die wählbaren Hintergründe für die Oberfläche.
pub fn list() -> Vec<Value> {
    WALLPAPERS
        .iter()
        .filter(|w| exists(w.id))
        .map(|w| json!({ "id": w.id, "name": t(w.name), "light": w.light }))
        .collect()
}

fn plain(status: u16) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Cow::Borrowed(&b""[..]))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
}

/// Liest „bytes=a-b“ (auch „bytes=a-“ und „bytes=-n“) für eine Datei der Länge `total`.
fn parse_range(header: &str, total: usize) -> Option<(usize, usize)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    let (first, last) = spec.split(',').next()?.trim().split_once('-')?;
    if total == 0 {
        return None;
    }
    let (start, end) = if first.is_empty() {
        // Die letzten n Bytes.
        let count: usize = last.parse().ok()?;
        (total.saturating_sub(count), total - 1)
    } else {
        let start: usize = first.parse().ok()?;
        let end = if last.is_empty() { total - 1 } else { last.parse::<usize>().ok()?.min(total - 1) };
        (start, end)
    };
    (start <= end && start < total).then_some((start, end))
}

/// Liefert `<id>.mp4`, `<id>.jpg` oder `<id>-thumb.jpg`. Videos gehen auch in
/// Teilen hinaus – WebKit fordert sie stückweise an.
pub fn serve(request: &Request<Vec<u8>>, file: &str) -> Response<Cow<'static, [u8]>> {
    let (stem, content_type) = if let Some(stem) = file.strip_suffix(".mp4") {
        (stem, "video/mp4")
    } else if let Some(stem) = file.strip_suffix(".jpg") {
        (stem, "image/jpeg")
    } else {
        return plain(404);
    };
    let id = stem.strip_suffix("-thumb").unwrap_or(stem);
    let (Some(dir), true) = (dir(), WALLPAPERS.iter().any(|w| w.id == id)) else {
        return plain(404);
    };
    // Der Name stammt jetzt aus der festen Liste – kein fremder Pfad möglich.
    let Ok(data) = std::fs::read(dir.join(file)) else {
        return plain(404);
    };
    let total = data.len();
    let range = request
        .headers()
        .get("Range")
        .and_then(|value| value.to_str().ok())
        .map(|value| parse_range(value, total));

    let builder = Response::builder()
        .header("Content-Type", content_type)
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "max-age=86400")
        .header("X-Content-Type-Options", "nosniff")
        .header("Cross-Origin-Resource-Policy", "same-origin");
    let response = match range {
        None => builder
            .status(200)
            .header("Content-Length", total.to_string())
            .body(Cow::Owned(data)),
        Some(Some((start, end))) => builder
            .status(206)
            .header("Content-Range", format!("bytes {start}-{end}/{total}"))
            .header("Content-Length", (end - start + 1).to_string())
            .body(Cow::Owned(data[start..=end].to_vec())),
        Some(None) => builder
            .status(416)
            .header("Content-Range", format!("bytes */{total}"))
            .body(Cow::Borrowed(&b""[..])),
    };
    response.unwrap_or_else(|_| plain(500))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_read_and_clamped() {
        assert_eq!(parse_range("bytes=0-1", 1000), Some((0, 1)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=900-5000", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("zeilen=1-2", 1000), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
    }

    #[test]
    fn only_listed_files_are_served() {
        let request = Request::builder().uri("zunder://newtab/wp/x").body(Vec::new()).unwrap();
        for file in ["../zunder.db", "glut.txt", "fremd.mp4", "glut/../../x.jpg", ""] {
            assert_eq!(serve(&request, file).status(), 404, "{file}");
        }
    }
}
