//! Eingebettete PDFs, die WebKit leer lässt.
//!
//! Web-Apps zeigen ein PDF oft in einem Rahmen, dessen Adresse mit `blob:`
//! beginnt – die Datei liegt dann nur im Speicher der Seite. WebKit hängt an
//! eine solche Adresse die Schutzregeln der Seite, die sie angelegt hat, auch
//! `frame-ancestors`. Verbietet eine Seite damit, dass *andere* sie einbetten
//! (`frame-ancestors 'none'`), trifft das Verbot ihren eigenen PDF-Rahmen: Er
//! bleibt leer. Andere Browser werten die Regel nur für Antworten aus dem Netz.
//!
//! Zunder hilft dem Rahmen auf die Sprünge (`ui/pdfs.js`, in Zunders
//! abgeschotteter Skriptwelt): Das Skript holt die Datei aus der Seite, gibt
//! sie dem Browser und lässt den Rahmen sie unter einer eigenen Adresse laden –
//! `zunder-pdf://<kennung>/…`. Hier steht, was der Browser dabei annimmt,
//! aufbewahrt und wieder herausgibt:
//!
//!   - Angenommen wird nur von Zunders eigenem Skript: Jede Lieferung beginnt
//!     mit einem Schlüssel, den Webseiten nicht kennen.
//!   - Nur PDFs bis 64 MB – und ausgeliefert stets als PDF, nie als etwas, das
//!     ein Browser ausführen würde.
//!   - Eine Kennung gilt nur in dem Tab, der die Datei geliefert hat, und nur,
//!     bis er eine andere Seite lädt.
//!
//! Am Ende der Datei steht die umgekehrte Lage: Wo WebKit PDFs gar nicht
//! anzeigt (Härtung, Stealth-Fenster), macht es aus einem eingebetteten PDF
//! von sich aus einen Download – den sagt Zunder ab.

use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Schema der eigenen Adresse. Der Browser lädt sie nur in einem Rahmen, nie
/// als Seite (siehe `native::nav_policy`).
pub const SCHEME: &str = "zunder-pdf";
/// Länge des Schlüssels, mit dem Zunders Skript jede Lieferung beginnt.
pub const KEY_LEN: usize = 32;
/// Länge einer Kennung.
const TOKEN_LEN: usize = 32;
/// Rechnername der Annahmestelle: `zunder-pdf://put/`.
const PUT_HOST: &str = "put";

/// Wie viel der Browser aufbewahrt.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Größte einzelne Datei.
    pub doc: usize,
    /// Dateien je Tab – die ältesten weichen.
    pub view_docs: usize,
    /// Bytes je Tab.
    pub view_bytes: usize,
    /// Bytes über alle Tabs.
    pub bytes: usize,
    /// Wie viel ein Tab in kurzer Zeit abliefern darf: Jede Lieferung
    /// beschäftigt den Browser selbst (nicht nur die Seite), und eine Seite
    /// kann beliebig viele Rahmen anlegen.
    pub span: Duration,
    pub span_puts: usize,
    pub span_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            doc: 64 << 20,
            view_docs: 12,
            view_bytes: 128 << 20,
            bytes: 320 << 20,
            span: Duration::from_secs(30),
            span_puts: 40,
            span_bytes: 256 << 20,
        }
    }
}

/// Was eine Anfrage an `zunder-pdf:` will.
#[derive(Debug, PartialEq, Eq)]
pub enum Ask {
    /// Zunders Skript liefert eine Datei ab.
    Put,
    /// Ein Rahmen holt die Datei mit dieser Kennung.
    Doc(String),
    Unknown,
}

pub fn route(method: &str, url: &str) -> Ask {
    let Ok(parsed) = url::Url::parse(url) else {
        return Ask::Unknown;
    };
    if parsed.scheme() != SCHEME || !parsed.username().is_empty() || parsed.port().is_some() {
        return Ask::Unknown;
    }
    let host = parsed.host_str().unwrap_or("");
    if method.eq_ignore_ascii_case("POST") && host == PUT_HOST {
        Ask::Put
    } else if method.eq_ignore_ascii_case("GET") && is_token(host) {
        Ask::Doc(host.to_string())
    } else {
        Ask::Unknown
    }
}

fn is_token(text: &str) -> bool {
    text.len() == TOKEN_LEN && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// 32 zufällige Hex-Zeichen – für den Schlüssel und für jede Kennung. Ohne
/// Zufall gibt es keine: Dann nimmt der Browser nichts an.
pub fn random_hex() -> Option<String> {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Ein PDF beginnt mit `%PDF-`; üblich ist, die Marke in den ersten 1024 Bytes
/// gelten zu lassen (manche Programme schreiben etwas davor).
pub fn looks_like_pdf(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    head.windows(5).any(|window| window == b"%PDF-")
}

/// Vergleich ohne frühen Abbruch – wie lange er dauert, verrät nichts darüber,
/// wie viele Zeichen des Schlüssels schon stimmen.
fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0u8, |sum, (a, b)| sum | (a ^ b)) == 0
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref());
    out
}

/// Warum der Browser eine Lieferung nicht annimmt.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Refusal {
    /// Ohne den Schlüssel von Zunders Skript.
    Key,
    Size,
    /// Kein PDF.
    Kind,
    /// Der Zufallsgenerator des Systems gab nichts her.
    Random,
    /// Der Tab hat in kurzer Zeit schon zu viel abgeliefert.
    Busy,
}

struct Doc {
    view: usize,
    token: String,
    digest: [u8; 32],
    bytes: Rc<[u8]>,
}

/// Die Ablage: die jüngste Datei steht hinten.
#[derive(Default)]
pub struct Shelf {
    docs: VecDeque<Doc>,
    /// Die Lieferungen der letzten Zeit: wann, aus welchem Tab, wie groß.
    recent: VecDeque<(Instant, usize, usize)>,
    limits: Limits,
}

impl Shelf {
    #[cfg(test)]
    fn with(limits: Limits) -> Self {
        Shelf { docs: VecDeque::new(), recent: VecDeque::new(), limits }
    }

    /// Nimmt eine Lieferung an: `body` ist der Schlüssel, gefolgt von der Datei.
    /// Liefert die Kennung, unter der der Tab `view` sie abrufen kann.
    pub fn put(
        &mut self,
        view: usize,
        key: &str,
        body: &[u8],
        now: Instant,
        token: impl FnOnce() -> Option<String>,
    ) -> Result<String, Refusal> {
        if key.len() != KEY_LEN || body.len() < KEY_LEN {
            return Err(Refusal::Key);
        }
        let (given, file) = body.split_at(KEY_LEN);
        if !same(given, key.as_bytes()) {
            return Err(Refusal::Key);
        }
        if file.is_empty() || file.len() > self.limits.doc {
            return Err(Refusal::Size);
        }
        if !looks_like_pdf(file) {
            return Err(Refusal::Kind);
        }
        // Erst jetzt wird es teuer (Prüfsumme, Kopie): Hat der Tab sein Maß
        // für die letzte Zeit schon ausgeschöpft, bleibt es dabei.
        let span = self.limits.span;
        while self.recent.front().is_some_and(|(at, _, _)| now.saturating_duration_since(*at) >= span) {
            self.recent.pop_front();
        }
        let (puts, bytes) = self
            .recent
            .iter()
            .filter(|(_, from, _)| *from == view)
            .fold((0usize, 0usize), |(puts, bytes), (_, _, size)| (puts + 1, bytes + size));
        if puts >= self.limits.span_puts || bytes.saturating_add(file.len()) > self.limits.span_bytes.max(file.len()) {
            return Err(Refusal::Busy);
        }
        self.recent.push_back((now, view, file.len()));
        let digest = digest(file);
        // Dieselbe Datei noch einmal – die Seite hat ihren Rahmen neu gezeichnet:
        // dieselbe Kennung, und die Datei gilt wieder als die jüngste.
        if let Some(position) = self.docs.iter().position(|doc| doc.view == view && doc.digest == digest) {
            if let Some(doc) = self.docs.remove(position) {
                let token = doc.token.clone();
                self.docs.push_back(doc);
                return Ok(token);
            }
        }
        let token = token().filter(|token| is_token(token)).ok_or(Refusal::Random)?;
        if self.docs.iter().any(|doc| doc.token == token) {
            return Err(Refusal::Random);
        }
        self.docs.push_back(Doc { view, token: token.clone(), digest, bytes: Rc::from(file) });
        self.trim(view);
        Ok(token)
    }

    /// Räumt die ältesten Dateien weg, bis alles wieder in die Grenzen passt.
    /// Die eben abgelegte bleibt – sie allein ist nie zu groß.
    fn trim(&mut self, view: usize) {
        loop {
            let (count, bytes) = self
                .docs
                .iter()
                .filter(|doc| doc.view == view)
                .fold((0usize, 0usize), |(count, bytes), doc| (count + 1, bytes + doc.bytes.len()));
            if count <= 1 || (count <= self.limits.view_docs && bytes <= self.limits.view_bytes) {
                break;
            }
            let Some(oldest) = self.docs.iter().position(|doc| doc.view == view) else {
                break;
            };
            self.docs.remove(oldest);
        }
        while self.docs.len() > 1 && self.bytes() > self.limits.bytes {
            self.docs.pop_front();
        }
    }

    /// Die Datei zu einer Kennung – nur für den Tab, der sie geliefert hat.
    pub fn get(&self, view: usize, token: &str) -> Option<Rc<[u8]>> {
        self.docs
            .iter()
            .find(|doc| doc.view == view && same(doc.token.as_bytes(), token.as_bytes()))
            .map(|doc| doc.bytes.clone())
    }

    /// Der Tab lädt eine andere Seite: Seine Kennungen gelten nicht mehr.
    pub fn forget(&mut self, view: usize) {
        self.docs.retain(|doc| doc.view != view);
    }

    /// Räumt weg, was zu Tabs gehört, die es nicht mehr gibt.
    pub fn keep(&mut self, live: &HashSet<usize>) {
        self.docs.retain(|doc| live.contains(&doc.view));
        self.recent.retain(|(_, view, _)| live.contains(view));
    }

    pub fn bytes(&self) -> usize {
        self.docs.iter().map(|doc| doc.bytes.len()).sum()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.docs.len()
    }
}

// ---------------------------------------------------------------------------
// Eingebettetes, aus dem WebKit von sich aus einen Download macht
// ---------------------------------------------------------------------------
//
// Mit der Härtung und im Stealth-Fenster (WebKits Lockdown-Modus) zeigt WebKit
// PDFs nicht an. Aus einer Antwort, die eigentlich angezeigt werden sollte,
// macht es dort selbst einen Download – auch dann, wenn sie für einen Rahmen
// bestimmt war, den eine Seite nur einbettet. Solche Dateien landeten ohne
// jedes Zutun im Download-Ordner, bei jedem Besuch der Seite aufs Neue. Die
// Regeln hier sagen, wann Zunder einen solchen Download absagt
// (`native::nav_response_download`).

/// Verlangt der Server mit dieser `Content-Disposition`-Zeile, dass die Antwort
/// als Datei gespeichert wird? Dieselbe Lesart wie in wry – dort entscheidet
/// sie darüber, ob der Browser aus einer Antwort einen Download macht.
pub fn is_attachment(disposition: &str) -> bool {
    disposition.split(';').next().is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
}

/// Nennt dieser Inhaltstyp ein PDF?
pub fn is_pdf_type(mime: &str) -> bool {
    let kind = mime.split(';').next().unwrap_or_default().trim();
    kind.eq_ignore_ascii_case("application/pdf") || kind.eq_ignore_ascii_case("text/pdf")
}

/// Lädt hier ein eingebetteter Rahmen etwas herunter, wonach niemand gefragt hat?
///
/// Der Browser selbst verlangt einen Download nur, wenn sich der Inhalt nicht
/// anzeigen lässt (`shows` ist falsch) oder der Server ihn als Datei ausweist
/// (`attachment`). Meldet WebKit trotzdem einen Download, hat es etwas, das
/// angezeigt werden sollte, selbst zur Datei erklärt – im Lockdown-Modus macht
/// es das mit PDFs. Geht es dabei um den Tab (`main`), hat jemand diese Adresse
/// geöffnet: Dann ist der Download der einzige Weg zur Datei und bleibt. In
/// einem Rahmen hat niemand danach gefragt – es sei denn, der Mensch hat
/// selbst den Link angeklickt, der den Rahmen dorthin führt (`FrameClicks`).
///
/// Ein PDF (`pdf`) in einem Rahmen zählt auch dann dazu, wenn WebKit es als
/// nicht anzeigbar meldet: Auch dann wollte die Seite es nur zeigen. Alles
/// andere, das sich nicht anzeigen lässt, bleibt ein gewöhnlicher Download –
/// so laden Websites Dateien über einen versteckten Rahmen.
pub fn unasked_download(main: bool, shows: bool, attachment: bool, pdf: bool) -> bool {
    !main && !attachment && (shows || pdf)
}

/// Ist diese Navigation ein echter Mausklick des Menschen auf einen Link oder
/// ein Formular? `kind`: Art der Navigation, wie WebKit sie nennt (0 Link,
/// 1 Formular); `by_user`: WebKit sieht einen Menschen dahinter; `button`: die
/// gedrückte Maustaste. Alle drei müssen stimmen: „Vom Menschen“ meldet WebKit
/// auch für alles, was ein Skript im Gefolge eines Klicks tut, und „Link“ auch
/// dann, wenn ein Skript ihn anklickt – eine Maustaste nennt es dafür nicht.
pub fn frame_click(kind: isize, by_user: bool, button: isize) -> bool {
    matches!(kind, 0 | 1) && by_user && button != 0
}

/// So lange wartet ein Klick des Menschen auf die Antwort des Servers –
/// reichlich auch für einen, der sein PDF erst zusammenstellen muss.
const CLICK_SPAN: Duration = Duration::from_secs(60);
/// So lange deckt eine eingetroffene Antwort den Download, den WebKit aus ihr
/// macht – es meldet ihn gleich danach.
const ANSWER_SPAN: Duration = Duration::from_secs(10);
/// Mehr merkt sich der Browser je Tab nicht auf einmal …
const CLICKS_PER_VIEW: usize = 4;
/// … und mehr nicht insgesamt.
const CLICKS_KEPT: usize = 64;

/// Fingerabdruck einer Adresse ohne Anker. Gemerkt wird nur er: gleich groß
/// für jede Adresse – auch für eine, die eine ganze Datei in sich trägt (`data:`).
fn mark_of(url: &str) -> Option<[u8; 32]> {
    let url = url.split('#').next().unwrap_or(url);
    (!url.is_empty()).then(|| digest(url.as_bytes()))
}

/// Klicks des Menschen, die einen eingebetteten Rahmen woandershin führen: ein
/// mit der Maus angeklickter Link, ein so abgeschicktes Formular. Macht WebKit
/// aus der Antwort auf genau diese Adresse einen Download, war er gewollt –
/// wie bei einem PDF-Link, den man im Tab anklickt. (Welche Navigation als
/// solcher Klick gilt, entscheidet `native::nav_policy`.)
///
/// Ein Klick deckt einen Download, und zwar den aus der ersten Antwort, die
/// danach für diese Adresse eintrifft. Lädt die Seite dieselbe Adresse später
/// noch einmal von sich aus, zählt er nicht mehr.
#[derive(Default)]
pub struct FrameClicks {
    /// Angeklickt, die Antwort steht aus.
    waiting: VecDeque<Push>,
    /// Die Antwort ist da: Wird daraus gleich ein Download, ist er gedeckt
    /// (WebView, Adresse, wann).
    answered: VecDeque<(usize, [u8; 32], Instant)>,
}

/// Ein Klick, dessen Antwort noch aussteht.
struct Push {
    view: usize,
    /// Die Adresse, mit der es begann – daran findet eine Weiterleitung ihn wieder.
    first: [u8; 32],
    /// Die Adresse, auf die jetzt die Antwort erwartet wird.
    url: [u8; 32],
    at: Instant,
}

impl FrameClicks {
    /// Gibt es gerade nichts, worauf eine Antwort passen könnte? (Dann muss
    /// niemand die Adresse einer Antwort erst lesen.)
    pub fn idle(&self) -> bool {
        self.waiting.is_empty() && self.answered.is_empty()
    }

    fn expire(&mut self, now: Instant) {
        self.waiting.retain(|push| now.saturating_duration_since(push.at) < CLICK_SPAN);
        self.answered.retain(|(_, _, at)| now.saturating_duration_since(*at) < ANSWER_SPAN);
    }

    /// Der Mensch hat in dieser WebView einen Rahmen zu dieser Adresse geschickt.
    ///
    /// `redirected_from`: Der Server leitet hierher weiter – genannt ist die
    /// Adresse, mit der dieser Abruf begann (leer, wenn WebKit sie nicht
    /// nennt; dann gilt der jüngste Klick des Tabs). Der Klick wandert mit,
    /// ein zweiter entsteht dadurch nicht; und ohne Klick davor deckt eine
    /// Weiterleitung nichts.
    pub fn note(&mut self, view: usize, url: &str, redirected_from: Option<&str>, now: Instant) {
        self.expire(now);
        let Some(mark) = mark_of(url) else {
            return;
        };
        if let Some(first) = redirected_from {
            let first = mark_of(first);
            let mut mine = self.waiting.iter_mut().rev().filter(|push| push.view == view);
            let found = match first {
                // Je nach WebKit ist das der Anfang der Kette oder die Station
                // davor. Wer schon bei der neuen Adresse steht, ist nicht gemeint
                // (zwei Klicks auf denselben Anfang wandern einzeln).
                Some(first) => mine.find(|push| (push.first == first || push.url == first) && push.url != mark),
                None => mine.next(),
            };
            if let Some(push) = found {
                push.url = mark;
            }
            return;
        }
        // Ein Tab verdrängt nur seine eigenen Klicks, nicht die der anderen.
        if self.waiting.iter().filter(|push| push.view == view).count() >= CLICKS_PER_VIEW {
            if let Some(oldest) = self.waiting.iter().position(|push| push.view == view) {
                self.waiting.remove(oldest);
            }
        }
        while self.waiting.len() >= CLICKS_KEPT {
            self.waiting.pop_front();
        }
        self.waiting.push_back(Push { view, first: mark, url: mark, at: now });
    }

    /// Für einen eingebetteten Rahmen dieser WebView ist die Antwort auf diese
    /// Adresse eingetroffen. War sie angeklickt, deckt der Klick jetzt genau
    /// diese Antwort; eine frühere Antwort auf dieselbe Adresse ist überholt.
    pub fn answer(&mut self, view: usize, url: &str, now: Instant) {
        if self.waiting.is_empty() && self.answered.is_empty() {
            return;
        }
        self.expire(now);
        let Some(mark) = mark_of(url) else {
            return;
        };
        self.answered.retain(|(owner, known, _)| !(*owner == view && *known == mark));
        if let Some(index) = self.waiting.iter().position(|push| push.view == view && push.url == mark) {
            self.waiting.remove(index);
            while self.answered.len() >= CLICKS_KEPT {
                self.answered.pop_front();
            }
            self.answered.push_back((view, mark, now));
        }
    }

    /// Geht dieser Download auf einen Klick des Menschen zurück? Dann ist der
    /// Klick damit verbraucht.
    pub fn take(&mut self, view: usize, url: &str, now: Instant) -> bool {
        self.expire(now);
        let Some(mark) = mark_of(url) else {
            return false;
        };
        match self.answered.iter().position(|(owner, known, _)| *owner == view && *known == mark) {
            Some(index) => {
                self.answered.remove(index);
                true
            }
            None => false,
        }
    }

    /// Der Tab ist neu oder zeigt eine neue Seite: Was vorher angeklickt wurde,
    /// gilt nicht mehr.
    pub fn forget(&mut self, view: usize) {
        self.waiting.retain(|push| push.view != view);
        self.answered.retain(|(owner, _, _)| *owner != view);
    }

    /// Geschlossene Tabs: Ihre Klicks braucht niemand mehr.
    pub fn keep(&mut self, live: &HashSet<usize>) {
        self.waiting.retain(|push| live.contains(&push.view));
        self.answered.retain(|(view, _, _)| live.contains(view));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ein fester Zeitpunkt – wo es auf die Zeit nicht ankommt.
    fn t0() -> Instant {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *START.get_or_init(Instant::now)
    }

    const KEY: &str = "0123456789abcdef0123456789abcdef";

    fn body(file: &[u8]) -> Vec<u8> {
        let mut out = KEY.as_bytes().to_vec();
        out.extend_from_slice(file);
        out
    }

    fn pdf(mark: u8, len: usize) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        out.resize(len.max(9), mark);
        out
    }

    /// Kennungen der Reihe nach: 000…001, 000…002, …
    fn counter() -> impl FnMut() -> Option<String> {
        let mut next = 0u32;
        move || {
            next += 1;
            Some(format!("{next:032x}"))
        }
    }

    #[test]
    fn anfragen_werden_richtig_zugeordnet() {
        let token = "00000000000000000000000000000abc";
        assert_eq!(route("POST", "zunder-pdf://put/"), Ask::Put);
        assert_eq!(route("post", "zunder-pdf://put/"), Ask::Put);
        assert_eq!(route("GET", &format!("zunder-pdf://{token}/Bericht.pdf#view=FitH")), Ask::Doc(token.into()));
        assert_eq!(route("GET", &format!("zunder-pdf://{token}/")), Ask::Doc(token.into()));
        // Abliefern nur per POST, abholen nur per GET.
        assert_eq!(route("GET", "zunder-pdf://put/"), Ask::Unknown);
        assert_eq!(route("POST", &format!("zunder-pdf://{token}/")), Ask::Unknown);
        assert_eq!(route("HEAD", &format!("zunder-pdf://{token}/")), Ask::Unknown);
        // Keine Kennung: zu kurz, Großbuchstaben, fremde Zeichen.
        assert_eq!(route("GET", "zunder-pdf://abc/x.pdf"), Ask::Unknown);
        assert_eq!(route("GET", "zunder-pdf://00000000000000000000000000000ABC/"), Ask::Unknown);
        assert_eq!(route("GET", "zunder-pdf://0000000000000000000000000000.abc/"), Ask::Unknown);
        // Fremdes Schema, Benutzername, Port, Unsinn.
        assert_eq!(route("GET", &format!("zunder://{token}/")), Ask::Unknown);
        assert_eq!(route("GET", &format!("https://{token}/")), Ask::Unknown);
        assert_eq!(route("GET", &format!("zunder-pdf://x@{token}/")), Ask::Unknown);
        assert_eq!(route("GET", &format!("zunder-pdf://{token}:8080/")), Ask::Unknown);
        assert_eq!(route("GET", "zunder-pdf:"), Ask::Unknown);
        assert_eq!(route("GET", ""), Ask::Unknown);
    }

    #[test]
    fn nur_ungefragtes_aus_rahmen_wird_abgesagt() {
        // Rahmen, anzeigbar, kein Anhang: Diesen Download hat WebKit selbst erfunden.
        assert!(unasked_download(false, true, false, true));
        assert!(unasked_download(false, true, false, false));
        // Ein PDF im Rahmen auch dann, wenn WebKit es als nicht anzeigbar meldet.
        assert!(unasked_download(false, false, false, true));
        // Sonst gilt: Was sich nicht anzeigen lässt, hat der Browser selbst als
        // Download verlangt – so laden Websites Dateien über einen versteckten Rahmen.
        assert!(!unasked_download(false, false, false, false));
        // Als Datei ausgewiesen: bleibt, auch als PDF.
        for (shows, pdf) in [(true, true), (true, false), (false, true), (false, false)] {
            assert!(!unasked_download(false, shows, true, pdf));
        }
        // Im Tab bleibt alles: Jemand hat die Adresse geöffnet.
        for shows in [true, false] {
            for attachment in [true, false] {
                for pdf in [true, false] {
                    assert!(!unasked_download(true, shows, attachment, pdf));
                }
            }
        }
    }

    #[test]
    fn ein_klick_deckt_genau_einen_download() {
        let start = t0();
        let later = |seconds: u64| start + Duration::from_secs(seconds);
        let url = "https://beispiel.de/bericht.pdf";
        let mut clicks = FrameClicks::default();

        // Ohne Klick ist nichts gedeckt – auch nicht, wenn eine Antwort eintrifft.
        clicks.answer(1, url, start);
        assert!(!clicks.take(1, url, start));

        // Angeklickt, aber noch keine Antwort: Ein Download ohne sie ist nicht gedeckt.
        clicks.note(1, "https://beispiel.de/bericht.pdf#seite=2", None, start);
        assert!(!clicks.take(1, url, later(1)));
        // Ein anderer Tab und eine andere Adresse haben damit nichts zu tun.
        clicks.answer(2, url, later(1));
        clicks.answer(1, "https://beispiel.de/anderes.pdf", later(1));
        clicks.answer(1, "https://beispiel.de/bericht.pdf?x=1", later(1));
        assert!(!clicks.take(2, url, later(1)));
        assert!(!clicks.take(1, "https://beispiel.de/anderes.pdf", later(1)));
        // Die Antwort trifft ein – der Anker zählt dabei nicht.
        clicks.answer(1, "https://beispiel.de/bericht.pdf#view=FitH", later(2));
        assert!(clicks.take(1, url, later(2)));
        // Verbraucht: Ein zweiter Download derselben Adresse war nicht gewollt.
        assert!(!clicks.take(1, url, later(2)));
        clicks.answer(1, url, later(3));
        assert!(!clicks.take(1, url, later(3)));

        // Zwei Klicks decken zwei Downloads.
        clicks.note(1, url, None, later(10));
        clicks.note(1, url, None, later(10));
        clicks.answer(1, url, later(11));
        assert!(clicks.take(1, url, later(11)));
        clicks.answer(1, url, later(12));
        assert!(clicks.take(1, url, later(12)));
        clicks.answer(1, url, later(13));
        assert!(!clicks.take(1, url, later(13)));
    }

    #[test]
    fn nur_die_erste_antwort_nach_dem_klick_zaehlt() {
        let start = t0();
        let later = |seconds: u64| start + Duration::from_secs(seconds);
        let url = "https://beispiel.de/seite";
        let mut clicks = FrameClicks::default();
        // Der Klick führt zu einer gewöhnlichen Seite – kein Download.
        clicks.note(1, url, None, start);
        clicks.answer(1, url, later(1));
        // Die Seite lädt dieselbe Adresse gleich noch einmal von sich aus, und
        // diesmal antwortet der Server mit einem PDF: nicht gedeckt.
        clicks.answer(1, url, later(2));
        assert!(!clicks.take(1, url, later(2)));

        // Eine Antwort deckt ihren Download nur kurz.
        clicks.note(1, url, None, later(20));
        clicks.answer(1, url, later(21));
        assert!(!clicks.take(1, url, later(21 + 10)));

        // Und ein Klick wartet höchstens eine Minute auf seine Antwort.
        clicks.note(1, url, None, later(100));
        clicks.answer(1, url, later(100 + 60));
        assert!(!clicks.take(1, url, later(100 + 60)));
        clicks.note(1, url, None, later(200));
        clicks.answer(1, url, later(200 + 59));
        assert!(clicks.take(1, url, later(200 + 59)));
    }

    #[test]
    fn eine_weiterleitung_nimmt_den_klick_mit() {
        let start = t0();
        let later = |seconds: u64| start + Duration::from_secs(seconds);
        let (first, hop, last) = ("https://beispiel.de/a", "https://beispiel.de/b", "https://cdn.beispiel.de/c.pdf");
        let mut clicks = FrameClicks::default();
        clicks.note(1, first, None, start);
        clicks.note(1, hop, Some(first), later(1));
        clicks.note(1, last, Some(first), later(1));
        // Gedeckt ist nur das Ziel – die Zwischenstationen nicht mehr.
        clicks.answer(1, first, later(2));
        clicks.answer(1, hop, later(2));
        assert!(!clicks.take(1, first, later(2)));
        assert!(!clicks.take(1, hop, later(2)));
        clicks.answer(1, last, later(3));
        assert!(clicks.take(1, last, later(3)));
        assert!(clicks.idle());

        // Die Frist läuft ab dem Klick, nicht ab der Weiterleitung.
        clicks.note(1, first, None, later(10));
        clicks.note(1, last, Some(first), later(60));
        clicks.answer(1, last, later(10 + 60));
        assert!(!clicks.take(1, last, later(10 + 60)));

        // Zwei Klicks auf einmal: Die Weiterleitung des einen lässt den anderen stehen.
        let other = "https://beispiel.de/zweites.pdf";
        clicks.note(1, first, None, later(100));
        clicks.note(1, other, None, later(100));
        clicks.note(1, last, Some(first), later(101));
        clicks.answer(1, other, later(102));
        assert!(clicks.take(1, other, later(102)));
        clicks.answer(1, last, later(102));
        assert!(clicks.take(1, last, later(102)));
        assert!(clicks.idle());

        // Nennt WebKit den Anfang nicht, wandert der jüngste Klick des Tabs.
        clicks.note(1, first, None, later(200));
        clicks.note(1, last, Some(""), later(201));
        clicks.answer(1, last, later(202));
        assert!(clicks.take(1, last, later(202)));

        // Eine Weiterleitung ohne Klick davor deckt nichts – auch nicht über
        // den Klick eines anderen Tabs oder einen mit anderem Anfang.
        clicks.note(2, first, None, later(300));
        clicks.note(1, last, Some(first), later(301));
        clicks.note(1, last, Some(""), later(301));
        clicks.note(2, last, Some(hop), later(301));
        clicks.answer(1, last, later(302));
        clicks.answer(2, last, later(302));
        assert!(!clicks.take(1, last, later(302)));
        assert!(!clicks.take(2, last, later(302)));
        clicks.answer(2, first, later(302));
        assert!(clicks.take(2, first, later(302)));
    }

    #[test]
    fn klicks_bleiben_begrenzt_und_je_tab_getrennt() {
        let start = t0();
        let later = |seconds: u64| start + Duration::from_secs(seconds);
        let mut clicks = FrameClicks::default();
        // Ein Tab merkt sich vier – die ältesten weichen.
        clicks.note(1, "https://beispiel.de/wichtig.pdf", None, start);
        for index in 0..40 {
            clicks.note(2, &format!("https://beispiel.de/{index}.pdf"), None, start);
        }
        assert_eq!(clicks.waiting.iter().filter(|push| push.view == 2).count(), CLICKS_PER_VIEW);
        clicks.answer(2, "https://beispiel.de/0.pdf", later(1));
        assert!(!clicks.take(2, "https://beispiel.de/0.pdf", later(1)));
        clicks.answer(2, "https://beispiel.de/39.pdf", later(1));
        assert!(clicks.take(2, "https://beispiel.de/39.pdf", later(1)));
        // Der Klick des anderen Tabs ist davon unberührt.
        clicks.answer(1, "https://beispiel.de/wichtig.pdf", later(2));
        assert!(clicks.take(1, "https://beispiel.de/wichtig.pdf", later(2)));

        // Insgesamt nie mehr als vierundsechzig.
        for view in 10..60 {
            for index in 0..4 {
                clicks.note(view, &format!("https://beispiel.de/{view}/{index}.pdf"), None, later(3));
            }
        }
        assert_eq!(clicks.waiting.len(), CLICKS_KEPT);

        // Leere Adressen merkt sich der Browser nicht; lange schon – als
        // Fingerabdruck, gleich groß wie jede andere.
        let mut clicks = FrameClicks::default();
        clicks.note(1, "", None, later(4));
        clicks.note(1, "#nur-anker", None, later(4));
        assert!(clicks.waiting.is_empty());
        let long = format!("data:application/pdf;base64,{}", "JVBERi0x".repeat(40_000));
        clicks.note(1, &long, None, later(4));
        clicks.answer(1, &long, later(5));
        assert!(clicks.take(1, &long, later(5)));

        // Eine neue Seite im Tab und geschlossene Tabs nehmen ihre Klicks mit.
        clicks.note(1, "https://beispiel.de/alt.pdf", None, later(6));
        clicks.note(7, "https://beispiel.de/zu.pdf", None, later(6));
        clicks.note(8, "https://beispiel.de/bleibt.pdf", None, later(6));
        clicks.note(8, "https://beispiel.de/da.pdf", None, later(6));
        clicks.answer(8, "https://beispiel.de/da.pdf", later(7));
        clicks.forget(1);
        clicks.keep(&HashSet::from([1, 8]));
        clicks.answer(1, "https://beispiel.de/alt.pdf", later(7));
        clicks.answer(7, "https://beispiel.de/zu.pdf", later(7));
        clicks.answer(8, "https://beispiel.de/bleibt.pdf", later(7));
        assert!(!clicks.take(1, "https://beispiel.de/alt.pdf", later(7)));
        assert!(!clicks.take(7, "https://beispiel.de/zu.pdf", later(7)));
        assert!(clicks.take(8, "https://beispiel.de/bleibt.pdf", later(7)));
        assert!(clicks.take(8, "https://beispiel.de/da.pdf", later(7)));
    }

    #[test]
    fn nur_der_echte_mausklick_auf_link_oder_formular_zaehlt() {
        // Link und Formular, vom Menschen, mit Maustaste.
        assert!(frame_click(0, true, 1));
        assert!(frame_click(1, true, 1));
        assert!(frame_click(0, true, 4));
        // Ein Skript klickt den Link an oder schickt das Formular ab – auch
        // mitten in einem echten Klick anderswo: keine Maustaste.
        assert!(!frame_click(0, true, 0));
        assert!(!frame_click(1, true, 0));
        // Ein Skript belegt den Rahmen (Art „alles andere“), ob nach einem Klick oder nicht.
        assert!(!frame_click(-1, true, 0));
        assert!(!frame_click(-1, true, 1));
        assert!(!frame_click(-1, false, 0));
        // Blättern im Verlauf und Neuladen sind kein Klick auf einen Link.
        for kind in 2..=4 {
            assert!(!frame_click(kind, true, 1));
        }
        // Ohne WebKits Auskunft, dass ein Mensch dahintersteht, zählt nichts.
        assert!(!frame_click(0, false, 1));
        assert!(!frame_click(1, false, 1));
    }

    #[test]
    fn weiterleitungen_und_aufraeumen_im_einzelnen() {
        let start = t0();
        let later = |seconds: u64| start + Duration::from_secs(seconds);
        let (first, hop, last) = ("https://beispiel.de/a", "https://beispiel.de/b", "https://beispiel.de/c.pdf");

        // WebKit nennt zur Weiterleitung die Station davor statt des Anfangs.
        let mut clicks = FrameClicks::default();
        clicks.note(1, first, None, start);
        clicks.note(1, hop, Some(first), later(1));
        clicks.note(1, last, Some(hop), later(1));
        clicks.answer(1, last, later(2));
        assert!(clicks.take(1, last, later(2)));
        assert!(clicks.idle());

        // Zwei Klicks auf denselben Anfang, beide weitergeleitet: Jeder wandert für sich.
        clicks.note(1, first, None, later(10));
        clicks.note(1, first, None, later(10));
        clicks.note(1, last, Some(first), later(11));
        clicks.note(1, last, Some(first), later(11));
        clicks.answer(1, last, later(12));
        assert!(clicks.take(1, last, later(12)));
        clicks.answer(1, last, later(13));
        assert!(clicks.take(1, last, later(13)));
        assert!(clicks.idle());

        // Eine Antwort deckt ihren Download bis knapp zehn Sekunden.
        clicks.note(1, last, None, later(20));
        clicks.answer(1, last, later(21));
        assert!(clicks.take(1, last, later(21 + 9)));

        // Eine neue Seite im Tab nimmt auch mit, was schon beantwortet war.
        clicks.note(1, last, None, later(40));
        clicks.answer(1, last, later(41));
        clicks.forget(1);
        assert!(!clicks.take(1, last, later(41)));
        assert!(clicks.idle());
        // Ebenso ein geschlossener Tab.
        clicks.note(7, last, None, later(50));
        clicks.answer(7, last, later(51));
        clicks.keep(&HashSet::from([1]));
        assert!(!clicks.take(7, last, later(51)));
        assert!(clicks.idle());

        // Auch die beantworteten bleiben begrenzt.
        for view in 0..200 {
            let url = format!("https://beispiel.de/{view}.pdf");
            clicks.note(view, &url, None, later(60));
            clicks.answer(view, &url, later(60));
        }
        assert_eq!(clicks.answered.len(), CLICKS_KEPT);
        assert!(clicks.waiting.is_empty());
    }

    #[test]
    fn anhang_wird_wie_in_wry_gelesen() {
        assert!(is_attachment("attachment"));
        assert!(is_attachment("attachment; filename=\"bericht.pdf\""));
        assert!(is_attachment("  Attachment ;filename=x"));
        assert!(is_attachment("ATTACHMENT"));
        assert!(!is_attachment("inline"));
        assert!(!is_attachment("inline; filename=\"attachment.pdf\""));
        assert!(!is_attachment("attachments"));
        assert!(!is_attachment("filename=attachment"));
        assert!(!is_attachment(""));
    }

    #[test]
    fn pdf_am_inhaltstyp() {
        assert!(is_pdf_type("application/pdf"));
        assert!(is_pdf_type("Application/PDF; charset=binary"));
        assert!(is_pdf_type(" text/pdf "));
        assert!(!is_pdf_type("application/pdfx"));
        assert!(!is_pdf_type("application/octet-stream"));
        assert!(!is_pdf_type("model/vnd.usdz+zip"));
        assert!(!is_pdf_type(""));
    }

    #[test]
    fn pdf_wird_an_seiner_marke_erkannt() {
        assert!(looks_like_pdf(b"%PDF-1.4\n..."));
        // Etwas davor ist üblich – aber nicht beliebig viel.
        let mut late = vec![b' '; 1000];
        late.extend_from_slice(b"%PDF-1.4");
        assert!(looks_like_pdf(&late));
        let mut too_late = vec![b' '; 1024];
        too_late.extend_from_slice(b"%PDF-1.4");
        assert!(!looks_like_pdf(&too_late));
        assert!(!looks_like_pdf(b"<!doctype html><script>alert(1)</script>"));
        assert!(!looks_like_pdf(b"%PDF"));
        assert!(!looks_like_pdf(b""));
    }

    #[test]
    fn abgelegt_und_wieder_abgeholt() {
        let mut shelf = Shelf::default();
        let mut next = counter();
        let file = pdf(1, 200);
        let token = shelf.put(7, KEY, &body(&file), t0(), &mut next).unwrap();
        assert_eq!(shelf.get(7, &token).as_deref(), Some(&file[..]));
        // Nur der Tab, der geliefert hat – und nur mit der richtigen Kennung.
        assert!(shelf.get(8, &token).is_none());
        assert!(shelf.get(7, "ffffffffffffffffffffffffffffffff").is_none());
        assert!(shelf.get(7, "").is_none());
    }

    #[test]
    fn ohne_schluessel_wird_nichts_angenommen() {
        let mut shelf = Shelf::default();
        let mut next = counter();
        let file = pdf(1, 200);
        // Gar kein Schlüssel davor, ein falscher, ein zu kurzer Text.
        assert_eq!(shelf.put(7, KEY, &file, t0(), &mut next), Err(Refusal::Key));
        let mut wrong = b"0123456789abcdef0123456789abcdee".to_vec();
        wrong.extend_from_slice(&file);
        assert_eq!(shelf.put(7, KEY, &wrong, t0(), &mut next), Err(Refusal::Key));
        assert_eq!(shelf.put(7, KEY, b"kurz", t0(), &mut next), Err(Refusal::Key));
        // Hat der Browser selbst keinen Schlüssel (kein Zufall), nimmt er nichts an.
        assert_eq!(shelf.put(7, "", &file, t0(), &mut next), Err(Refusal::Key));
        assert_eq!(shelf.len(), 0);
    }

    #[test]
    fn nur_pdfs_und_nicht_zu_grosse() {
        let limits = Limits { doc: 1000, ..Limits::default() };
        let mut shelf = Shelf::with(limits);
        let mut next = counter();
        assert_eq!(shelf.put(7, KEY, &body(b"<html><script>1</script></html>"), t0(), &mut next), Err(Refusal::Kind));
        assert_eq!(shelf.put(7, KEY, &body(b""), t0(), &mut next), Err(Refusal::Size));
        assert_eq!(shelf.put(7, KEY, &body(&pdf(1, 1001)), t0(), &mut next), Err(Refusal::Size));
        assert!(shelf.put(7, KEY, &body(&pdf(1, 1000)), t0(), &mut next).is_ok());
        assert_eq!(shelf.len(), 1);
    }

    #[test]
    fn ohne_zufall_keine_kennung() {
        let mut shelf = Shelf::default();
        assert_eq!(shelf.put(7, KEY, &body(&pdf(1, 50)), t0(), || None), Err(Refusal::Random));
        assert_eq!(shelf.put(7, KEY, &body(&pdf(1, 50)), t0(), || Some("zu kurz".into())), Err(Refusal::Random));
        // Zweimal dieselbe Kennung für verschiedene Dateien gibt es nicht.
        let fixed = || Some("0000000000000000000000000000000a".to_string());
        assert!(shelf.put(7, KEY, &body(&pdf(1, 50)), t0(), fixed).is_ok());
        assert_eq!(shelf.put(8, KEY, &body(&pdf(2, 50)), t0(), fixed), Err(Refusal::Random));
        assert_eq!(shelf.len(), 1);
    }

    #[test]
    fn dieselbe_datei_bekommt_dieselbe_kennung() {
        let mut shelf = Shelf::default();
        let mut next = counter();
        let first = shelf.put(7, KEY, &body(&pdf(1, 300)), t0(), &mut next).unwrap();
        let other = shelf.put(7, KEY, &body(&pdf(2, 300)), t0(), &mut next).unwrap();
        let again = shelf.put(7, KEY, &body(&pdf(1, 300)), t0(), &mut next).unwrap();
        assert_eq!(first, again);
        assert_ne!(first, other);
        assert_eq!(shelf.len(), 2);
        // In einem anderen Tab ist es eine andere Ablage.
        let elsewhere = shelf.put(8, KEY, &body(&pdf(1, 300)), t0(), &mut next).unwrap();
        assert_ne!(first, elsewhere);
        assert_eq!(shelf.len(), 3);
    }

    #[test]
    fn die_aeltesten_weichen() {
        let limits = Limits { doc: 1000, view_docs: 3, view_bytes: 2500, bytes: 4000, ..Limits::default() };
        let mut shelf = Shelf::with(limits);
        let mut next = counter();
        let tokens: Vec<String> =
            (1..=4u8).map(|mark| shelf.put(7, KEY, &body(&pdf(mark, 100)), t0(), &mut next).unwrap()).collect();
        // Vier Dateien, drei Plätze: die erste ist weg.
        assert!(shelf.get(7, &tokens[0]).is_none());
        assert!(shelf.get(7, &tokens[1]).is_some());
        assert!(shelf.get(7, &tokens[3]).is_some());
        // Zu viele Bytes im Tab: wieder weichen die ältesten.
        let big_a = shelf.put(7, KEY, &body(&pdf(10, 1000)), t0(), &mut next).unwrap();
        let big_b = shelf.put(7, KEY, &body(&pdf(11, 1000)), t0(), &mut next).unwrap();
        let big_c = shelf.put(7, KEY, &body(&pdf(12, 1000)), t0(), &mut next).unwrap();
        assert!(shelf.get(7, &tokens[3]).is_none());
        assert!(shelf.get(7, &big_a).is_none());
        assert!(shelf.get(7, &big_b).is_some());
        assert!(shelf.get(7, &big_c).is_some());
        assert_eq!(shelf.bytes(), 2000);
        // Ein anderer Tab verdrängt erst, wenn es insgesamt zu viel wird –
        // dann die ältesten überhaupt.
        let far_a = shelf.put(8, KEY, &body(&pdf(20, 1000)), t0(), &mut next).unwrap();
        let far_b = shelf.put(8, KEY, &body(&pdf(21, 1000)), t0(), &mut next).unwrap();
        assert!(shelf.get(7, &big_b).is_some());
        let far_c = shelf.put(9, KEY, &body(&pdf(22, 1000)), t0(), &mut next).unwrap();
        assert!(shelf.get(7, &big_b).is_none());
        assert!(shelf.get(7, &big_c).is_some());
        assert!(shelf.get(8, &far_a).is_some());
        assert!(shelf.get(8, &far_b).is_some());
        assert!(shelf.get(9, &far_c).is_some());
        assert_eq!(shelf.bytes(), 4000);
    }

    #[test]
    fn neue_seite_und_geschlossene_tabs_raeumen_auf() {
        let mut shelf = Shelf::default();
        let mut next = counter();
        let mine = shelf.put(7, KEY, &body(&pdf(1, 100)), t0(), &mut next).unwrap();
        let theirs = shelf.put(8, KEY, &body(&pdf(2, 100)), t0(), &mut next).unwrap();
        shelf.forget(7);
        assert!(shelf.get(7, &mine).is_none());
        assert!(shelf.get(8, &theirs).is_some());
        // Dieselbe Datei nach dem Seitenwechsel: eine neue Kennung, die alte bleibt ungültig.
        let fresh = shelf.put(7, KEY, &body(&pdf(1, 100)), t0(), &mut next).unwrap();
        assert_ne!(mine, fresh);
        assert!(shelf.get(7, &mine).is_none());
        shelf.keep(&HashSet::from([7]));
        assert!(shelf.get(8, &theirs).is_none());
        assert!(shelf.get(7, &fresh).is_some());
    }

    #[test]
    fn ein_tab_liefert_nicht_endlos() {
        let limits = Limits {
            doc: 1000,
            span: Duration::from_secs(30),
            span_puts: 3,
            span_bytes: 2500,
            ..Limits::default()
        };
        let mut shelf = Shelf::with(limits);
        let mut next = counter();
        let start = t0();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        // Drei Lieferungen in kurzer Zeit gehen, die vierte nicht – auch wenn
        // es dieselbe Datei wäre: Schon das Prüfen kostet.
        for mark in 1..=3u8 {
            assert!(shelf.put(7, KEY, &body(&pdf(mark, 100)), at(u64::from(mark)), &mut next).is_ok());
        }
        assert_eq!(shelf.put(7, KEY, &body(&pdf(4, 100)), at(4), &mut next), Err(Refusal::Busy));
        assert_eq!(shelf.put(7, KEY, &body(&pdf(1, 100)), at(5), &mut next), Err(Refusal::Busy));
        // Ein anderer Tab hat sein eigenes Maß.
        assert!(shelf.put(8, KEY, &body(&pdf(4, 100)), at(5), &mut next).is_ok());
        // Was ohne Schlüssel, zu groß oder kein PDF ist, zählt gar nicht erst mit.
        assert_eq!(shelf.put(9, KEY, b"kein Schluessel", at(5), &mut next), Err(Refusal::Key));
        assert_eq!(shelf.put(9, KEY, &body(b"<html>"), at(5), &mut next), Err(Refusal::Kind));
        for mark in 1..=3u8 {
            assert!(shelf.put(9, KEY, &body(&pdf(mark, 100)), at(5), &mut next).is_ok());
        }
        // Nach der Frist geht es weiter: Die Lieferungen von Sekunde 1 und 2
        // sind dann verjährt, die von Sekunde 3 zählt noch.
        assert!(shelf.put(7, KEY, &body(&pdf(4, 100)), at(32), &mut next).is_ok());
        assert!(shelf.put(7, KEY, &body(&pdf(5, 100)), at(32), &mut next).is_ok());
        assert_eq!(shelf.put(7, KEY, &body(&pdf(6, 100)), at(32), &mut next), Err(Refusal::Busy));
        // Auch die Menge zählt: 2500 Bytes in der Frist, nicht mehr.
        let mut shelf = Shelf::with(limits);
        assert!(shelf.put(7, KEY, &body(&pdf(1, 1000)), at(0), &mut next).is_ok());
        assert!(shelf.put(7, KEY, &body(&pdf(2, 1000)), at(1), &mut next).is_ok());
        assert_eq!(shelf.put(7, KEY, &body(&pdf(3, 1000)), at(2), &mut next), Err(Refusal::Busy));
        assert!(shelf.put(7, KEY, &body(&pdf(3, 400)), at(2), &mut next).is_ok());
        assert!(shelf.put(7, KEY, &body(&pdf(3, 1000)), at(31), &mut next).is_ok());
        // Ein geschlossener Tab hinterlässt auch hier nichts.
        shelf.keep(&HashSet::new());
        assert!(shelf.recent.is_empty());
    }

    #[test]
    fn zufall_liefert_brauchbare_kennungen() {
        let first = random_hex().unwrap();
        let second = random_hex().unwrap();
        assert!(is_token(&first) && is_token(&second));
        assert_ne!(first, second);
        assert_eq!(first.len(), KEY_LEN);
    }
}
