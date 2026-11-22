//! Stealth-Fenster: so wenig Spuren wie möglich – auf diesem Mac und im Netz.
//!
//! Ein Stealth-Fenster ist ein privates Fenster mit zusätzlichen Riegeln:
//!
//! * Jeder Tab ist eine eigene Insel (eigener, flüchtiger Datenspeicher): Was
//!   in einem Tab an Cookies und Website-Daten entsteht, sieht kein anderer.
//!   Nur Tabs, die eine Seite selbst öffnet, teilen sich die Insel ihrer
//!   Herkunft – sonst funktionierten Anmelde-Fenster nicht.
//! * WebKits Lockdown-Modus, kein WebRTC (es verriete die echte Adresse am
//!   Tunnel vorbei), WebKits eigener Schutz gegen Fingerabdrücke (verrauschte
//!   Zeichenfläche, Bildschirmgröße = Fenstergröße).
//! * Nur verschlüsselt, ohne Tracking-Anhängsel, mit Werbe- und
//!   Tracker-Blocker – unabhängig davon, was sonst eingestellt ist.
//! * Läuft auf diesem Mac Tor, geht der gesamte Verkehr des Fensters dort
//!   hindurch – jeder Tab über einen eigenen Weg. Fällt Tor aus, lädt nichts
//!   mehr: Am Tunnel vorbei wird nie geladen.
//!
//! Was es NICHT kann, sagt die Startseite des Fensters ausdrücklich: Ohne Tor
//! bleibt die IP-Adresse sichtbar, und auch mit Tor ist Zunder nicht der Tor
//! Browser – dessen Besucher sehen alle gleich aus, die von Zunder nicht.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Ein SOCKS5-Zugang auf diesem Mac (Tor).
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub host: String,
    pub port: u16,
}

impl Route {
    pub fn label(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Wo Tor üblicherweise lauscht: der Dienst (9050) und der Tor Browser (9150).
const PORTS: [u16; 2] = [9050, 9150];

/// Sucht einen laufenden Tor-Zugang. Gefragt wird einmal je neuem Fenster –
/// alle Tabs eines Fensters gehen denselben Weg.
pub fn find_route() -> Option<Route> {
    if !crate::native::can_route() {
        return None;
    }
    // Nur für Prüfungen: ein anderer Zugang (ebenfalls nur auf diesem Mac).
    if let Some(custom) = crate::dev::var("ZUNDER_STEALTH_PROXY") {
        let (host, port) = custom.rsplit_once(':')?;
        let port = port.parse().ok()?;
        return speaks_socks5(host, port).then(|| Route { host: host.to_string(), port });
    }
    PORTS
        .iter()
        .find(|port| speaks_socks5("127.0.0.1", **port))
        .map(|port| Route { host: "127.0.0.1".to_string(), port: *port })
}

/// Antwortet dort jemand auf den Gruß nach SOCKS5?
fn speaks_socks5(host: &str, port: u16) -> bool {
    let address: Option<SocketAddr> = (host, port).to_socket_addrs().ok().and_then(|mut found| found.next());
    let Some(address) = address.filter(|address| address.ip().is_loopback()) else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(300)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(600)));
    // Fassung 5, zwei Verfahren im Angebot: ohne Anmeldung (0) und mit Name/Passwort (2).
    if stream.write_all(&[5, 2, 0, 2]).is_err() {
        return false;
    }
    let mut answer = [0u8; 2];
    stream.read_exact(&mut answer).is_ok() && answer[0] == 5 && matches!(answer[1], 0 | 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn recognises_a_socks5_greeting_and_nothing_else() {
        // Ein Dienst, der wie SOCKS5 antwortet.
        let socks = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_port = socks.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in socks.incoming().take(1) {
                let mut stream = stream.unwrap();
                let mut hello = [0u8; 4];
                let _ = stream.read_exact(&mut hello);
                let _ = stream.write_all(&[5, 2]);
            }
        });
        // Ein Dienst, der etwas anderes spricht.
        let other = TcpListener::bind("127.0.0.1:0").unwrap();
        let other_port = other.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in other.incoming().take(1) {
                let mut stream = stream.unwrap();
                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
            }
        });
        // Ein Port, an dem niemand lauscht.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();

        assert!(speaks_socks5("127.0.0.1", socks_port));
        assert!(!speaks_socks5("127.0.0.1", other_port));
        assert!(!speaks_socks5("127.0.0.1", closed));
        // Nie ein Zugang außerhalb dieses Macs.
        assert!(!speaks_socks5("192.0.2.1", 9050));
    }
}
