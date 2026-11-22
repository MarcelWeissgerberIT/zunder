//! Prüfhilfe und Entwicklerschalter.
//!
//! In der ausgelieferten App gibt es sie nicht. Nur ein Bau mit dem Merkmal
//! `harness` (`ZUNDER_HARNESS=1 scripts/bundle.sh` bzw. `cargo build --features
//! harness`) wertet `ZUNDER_DEBUG` und die übrigen Variablen aus. Sonst könnte
//! jedes Programm, das Zunder starten darf, den Browser über Umgebung und
//! Standardeingabe fernsteuern – und so zum Beispiel die Abfrage per Touch ID
//! vor dem Passwort-Tresor umgehen.

use std::ffi::OsString;

/// Wurde dieses Programm mit der Prüfhilfe gebaut?
pub const BUILT_IN: bool = cfg!(feature = "harness");

/// Ist die Prüfhilfe eingeschaltet (mit ihr gebaut und `ZUNDER_DEBUG` gesetzt)?
pub fn enabled() -> bool {
    BUILT_IN && std::env::var_os("ZUNDER_DEBUG").is_some()
}

/// Eine Variable, die nur bei eingeschalteter Prüfhilfe gilt.
pub fn var(name: &str) -> Option<String> {
    if enabled() {
        std::env::var(name).ok()
    } else {
        None
    }
}

pub fn var_os(name: &str) -> Option<OsString> {
    if enabled() {
        std::env::var_os(name)
    } else {
        None
    }
}

/// Eine Variable für die Entwicklung: gilt in jedem Bau mit Prüfhilfe, auch
/// ohne `ZUNDER_DEBUG` (eigenes Profil, eigener Download-Ordner, Oberfläche von der Platte).
pub fn build_var_os(name: &str) -> Option<OsString> {
    if BUILT_IN {
        std::env::var_os(name)
    } else {
        None
    }
}

/// Name für Einträge im Schlüsselbund. Ein Bau mit Prüfhilfe benutzt immer
/// eigene Einträge – die der echten App rührt er nie an.
pub fn keychain_service() -> &'static str {
    if BUILT_IN {
        "app.zunder.browser.test"
    } else {
        "app.zunder.browser"
    }
}
