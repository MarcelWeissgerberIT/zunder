# Zunder

Ein schneller, schlichter Webbrowser für den Mac – mit eingebautem
KI-Assistenten, der auch ganz ohne Netz arbeitet.

Die gesamte Hülle ist in Rust geschrieben: Fenster, Tabs, Adressleiste,
Verlauf, Lesezeichen, Downloads, Privatmodus, Werbeblocker, Passwort-Tresor,
Assistent. Die Seiten selbst stellt das in macOS eingebaute WebKit dar – nicht
Chromium.
Damit läuft das heutige Web vollständig, und die App startet sofort.

![Zunder mit Vorschlägen in der Adressleiste](docs/zunder-vorschlaege.png)

## Was Zunder kann

- **Tabs**: öffnen, schließen, per Ziehen umsortieren, duplizieren,
  geschlossene wieder öffnen, ⌘1–⌘9, ⌃Tab. Links mit ⌘-Klick oder mittlerer
  Maustaste öffnen einen Tab im Hintergrund. **Tabs durchsuchen** (⇧⌘A) findet
  unter vielen offenen den richtigen; die Leiste rollt mit, sodass der aktive
  Tab immer zu sehen ist. **Alle Tabs zeigen** (⇧⌘\) legt sie als Karten mit
  Vorschaubild nebeneinander – durchsuchbar, mit einem Klick gewechselt oder
  geschlossen.
- **Angeheftete Tabs und Tab-Gruppen**: Ein angehefteter Tab steht schmal
  ganz links und ist nach jedem Start wieder da. Gruppen fassen Tabs unter
  einem farbigen Namen zusammen und lassen sich zusammenklappen – siehe
  [Tabs ordnen](#tabs-ordnen).
- **Adressleiste**: erkennt Adresse oder Suche, vervollständigt aus dem
  Verlauf, schlägt offene Tabs, Lesezeichen, Verlauf und Suchbegriffe vor.
  Sechs Suchmaschinen zur Wahl.
- **Verlauf, Lesezeichen, Downloads** als Seitenleisten, durchsuchbar.
- **Lesezeichen mit Ordnern**: ⌘D setzt eins; in der Seitenleiste lassen sie
  sich umbenennen, in Ordner legen und durch Ziehen sortieren. Die
  **Lesezeichenleiste** (⇧⌘B) zeigt sie unter der Adresszeile, Ordner als
  Aufklappmenü.
- **Startseite**: zeigt oft Besuchtes und auf Wunsch einen Live-Hintergrund –
  sonst nichts. Wer mag, legt die Lesezeichen dazu (Einstellungen →
  Startseite → „Lesezeichen auf der Startseite“, von Haus aus aus): Dann wird
  sie zum **Schreibtisch**, auf dem die Lesezeichen als Kacheln liegen, die
  sich durch Ziehen anordnen und in Ordner legen lassen; ein Ordner öffnet
  sich als kleines Fenster.
- **Passwörter**: sichert Anmeldungen verschlüsselt, füllt sie auf einen Klick
  und nach Touch ID aus und schlägt sichere Passwörter vor – siehe unten.
- **Lesemodus** (⌥⌘R): zeigt nur den Text eines Artikels, ohne Menüs, Werbung
  und Randspalten. Schriftgröße wählbar, hell und dunkel nach System.
- **Seite übersetzen** (⌥⌘T): übersetzt die Seite an Ort und Stelle – mit dem
  Modell des Assistenten, von Haus aus also auf dem Mac. Siehe
  [Seiten übersetzen](#seiten-übersetzen-t).
- **Security-Ampel mit Seitencheck** (⌥⌘S): Eine Ampel in der Adresszeile
  sagt, wie die gezeigte Seite dasteht, und ein Klick zeigt, wo sie
  angreifbar wäre – siehe
  [Security-Ampel und Seitencheck](#security-ampel-und-seitencheck-s).
- **Website-Daten**: zeigt je Website, was sie auf dem Mac abgelegt hat
  (Cookies, gespeicherte Daten, Zwischenspeicher), und löscht es einzeln oder
  für alle (Einstellungen → Website-Daten).
- **Umziehen**: Lesezeichen kommen direkt aus Chrome, Brave, Edge oder Vivaldi
  herüber oder aus der HTML-Datei, die Safari und Firefox exportieren;
  Passwörter aus der CSV-Datei jedes Browsers oder Passwortmanagers. Beides
  lässt sich auch wieder als Datei sichern.
- **Private Fenster**: kein Verlauf, keine Sitzung, keine Suchvorschläge;
  Cookies und Website-Daten leben nur, solange das Fenster offen ist.
- **Suche in der Seite** mit Trefferzählung, **Zoom** je Tab, **Drucken**.
- **Für Entwickler**: die vollständigen Entwicklerwerkzeuge von WebKit
  (Elemente, Konsole, Netzwerk, Quellen, Speicher; ⌥⌘I – von Haus aus
  abgeschaltet, der Menüpunkt schaltet sie ein), **Seitenquelltext** (⌥⌘U) so,
  wie der Server ihn liefert, und die **mobile Ansicht** (⇧⌘M): Der Tab gibt
  sich als iPhone aus, Seiten liefern ihre mobile Fassung.
- **Sitzung**: Beim Start sind die Tabs vom letzten Mal wieder da und laden
  erst, wenn man sie öffnet.
- **Speicher sparen**: Tabs, die drei Stunden unbenutzt im Hintergrund liegen,
  geben ihren Speicher frei – außer es läuft dort Ton oder Video.
- **Pop-ups und Anmeldefenster** werden zu Tabs und behalten die Verbindung
  zur öffnenden Seite.
- Dialoge der Seite, Anmeldung an geschützten Seiten, Fehlerseite mit
  „Erneut versuchen“, Rückfrage vor dem Öffnen anderer Programme.
- Hell und dunkel nach Systemeinstellung, Oberfläche deutsch oder englisch.
- **Eingebaute Apps**: eigene Seiten des Browsers mit genau benannten Rechten.
  Es gibt vier: Wetter und Regenradar, eine Zeiterfassung (⌥⌘Z), ein
  Sicherheitscheck, der die Schutzschichten einer Website prüft, und ein
  Request-Inspektor für HTTP-Anfragen – beides, um die eigenen Seiten und
  Schnittstellen zu härten (siehe [Eingebaute Apps](#eingebaute-apps)).
- **Ladebild beim Start**: fünf Sekunden Animation – das App-Symbol „zündet“
  vor der Glut-Szene der Live-Hintergründe, der Schriftzug setzt sich, ein
  Balken füllt sich. Die Seiten laden währenddessen schon; ein Klick oder ein
  Tastendruck gibt den Browser sofort frei (wer lostippt, landet in der
  Adressleiste). Abschaltbar in den Einstellungen.

![Das Ladebild beim Start](docs/zunder-ladebild.png)

![Lesezeichen mit Ordnern und Leiste](docs/zunder-lesezeichen.png)

![Die Startseite als Schreibtisch mit geöffnetem Ordner – mit eingeschalteten Lesezeichen](docs/zunder-schreibtisch.png)

![Alle Tabs als Karten mit Vorschaubild](docs/zunder-tab-uebersicht.png)

Dazu vier Dinge, die Zunder von anderen Browsern abheben:

### Assistent (⌘J)

![Der Assistent fasst eine Seite zusammen](docs/zunder-assistent.png)

Eine Seitenleiste, die die angezeigte Seite liest: zusammenfassen (⇧⌘J),
Kernpunkte, einfach erklären, übersetzen – oder eine eigene Frage stellen. Ist
auf der Seite etwas markiert, geht es nur um diese Stelle. Jeder Tab hat seine
eigene Unterhaltung; sie liegt nur im Arbeitsspeicher.

Vier Wege stehen zur Wahl („Modell wählen“ unten in der Leiste):

| Modell | Wo es läuft | Eigenheiten |
| --- | --- | --- |
| **Auf diesem Mac** | Apples Sprachmodell (ab macOS 26, Apple Intelligence) | Ohne Konto, ohne Netz, antwortet in 2–5 Sekunden. Liest höchstens gut 6000 Zeichen auf einmal: Lange Seiten liest es in bis zu acht Teilen, bei Fragen sucht es die passenden Abschnitte heraus. Die Antwort sagt jeweils dazu, wie viel gelesen wurde. |
| **Ollama** | ein Modell der eigenen Ollama-Installation | Bleibt auf dem Mac. Liest etwa 28 000 Zeichen auf einmal. Zunder nimmt es nur, wenn es hier ausdrücklich gewählt ist – nie von selbst. |
| **Claude** | Claude-API von Anthropic (Opus 5.5, Sonnet 5.5 oder Haiku 5.5) | Am gründlichsten, liest bis 400 000 Zeichen. Der Seitentext wird dafür an Anthropic gesendet. Braucht einen eigenen API-Schlüssel; er liegt im Schlüsselbund von macOS. |

| **Anderer Dienst** | alles, was die verbreitete Chat-Schnittstelle spricht („OpenAI-kompatibel“): LM Studio oder llama.cpp auf dem Mac, OpenAI, OpenRouter, Mistral, eine eigene Adresse | Auf dem Mac ohne Schlüssel. Im Netz mit eigenem Schlüssel (Schlüsselbund); der Seitentext geht dann an diesen Dienst. Das Modell wählt man aus der Liste des Dienstes. |

**Anderer Dienst im Einzelnen:** Die Adresse muss verschlüsselt sein
(`https://`) – unverschlüsselt geht nur `localhost`. Der Schlüssel gehört zu
genau dem Dienst, für den er eingetragen wurde: Wechselt die Adresse, wird er
entfernt statt mitgeschickt, und Umleitungen folgt Zunder nicht. Bei
**OpenRouter** geht es auch ohne Abtippen: „Bei OpenRouter anmelden“ öffnet
die Zustimmungsseite in einem Tab (OAuth mit PKCE); danach bekommt Zunder
einen eigenen Schlüssel für das Konto. Über OpenRouter sind auch die
Claude-Modelle erreichbar. Anthropic selbst bietet fremden Programmen keine
solche Anmeldung an – für Claude direkt bleibt es beim API-Schlüssel.

Ohne ausdrückliche Wahl arbeitet der Assistent nur mit Modellen auf dem Mac –
von selbst geht nie etwas ins Netz. In privaten Fenstern gilt das immer, auch
wenn sonst Claude oder ein Dienst im Netz gewählt ist. Die Zeile unter dem Eingabefeld zeigt jederzeit,
welches Modell antwortet und ob der Text den Mac verlässt.

Bei Claude ist für Opus und Sonnet der serverseitige Ersatz eingeschaltet:
Lehnt ein Schutzfilter eine Anfrage ab, übernimmt in derselben Anfrage ein
anderes Claude-Modell (`fallbacks: "default"`). Haiku kennt diesen Weg nicht.

### Compared Browsing (⌥⌘J)

![Zwei Angebote nebeneinander, rechts die Vergleichstabelle](docs/zunder-vergleich.png)

Im Assistenten „Vergleichen“ wählen, bis zu drei weitere Tabs ankreuzen – die
KI stellt die Seiten in einer Tabelle gegenüber. Mit genau einem weiteren Tab
stehen beide Seiten dabei nebeneinander im Fenster. Solange der Vergleich
läuft, gehen auch freie Fragen an alle Seiten zugleich („Welches ist
günstiger?“). Zwei Tabs lassen sich auch ohne KI nebeneinanderstellen:
Rechtsklick auf einen Tab → „Neben aktuellem Tab zeigen“.

Apples kleines Modell verwechselt Seiten, sobald es mehrere zugleich vor sich
hat. Deshalb sieht es beim Vergleichen immer nur eine: Es nennt je Seite
Stichworte, füllt dann je Seite denselben Steckbrief aus, und die Tabelle setzt
Zunder selbst zusammen. Darunter steht nur, was sich nachprüfen lässt – welche
Merkmale sich unterscheiden und welche gleich sind. Ein Fazit in Worten
liefern Ollama und Claude.

### Live-Hintergründe

![Auswahl der Hintergründe auf der Startseite](docs/zunder-hintergrund.png)

Sechs bewegte 3D-Szenen für die Startseite: Glut, Nordlicht, Tiefsee,
Kristall, Dünen, Wolken. Gewählt wird über den Knopf unten rechts auf der
Startseite oder in den Einstellungen. Jede Szene ist eine 7,4 Sekunden lange,
nahtlos wiederholte Schleife; das Standbild ist sofort da, das Video blendet
darüber ein. Es läuft nur, solange die Startseite wirklich zu sehen ist – nicht
in einem Tab im Hintergrund und nicht in einem verdeckten Fenster. Wer am Mac
„Bewegung reduzieren“ eingeschaltet hat, bekommt nur das Standbild. Private
Fenster zeigen keinen Hintergrund.

### Sicherheit

![Der Abschnitt Sicherheit in den Einstellungen](docs/zunder-sicherheit.png)

Wenige Schalter, die wirklich etwas bewirken (Einstellungen → Sicherheit):

- **Nur verschlüsselte Verbindungen** (an): Eine `http://`-Adresse wird zuerst
  verschlüsselt versucht. Geht das nicht – Zertifikatsfehler, keine Antwort
  binnen sechs Sekunden, Rückleitung ins Unverschlüsselte –, fragt Zunder nach,
  statt still unverschlüsselt zu laden. Die Zustimmung gilt je Rechner, bis
  Zunder beendet wird. Geräte im eigenen Netz (localhost, 192.168.x.x,
  `.local` …) sind ausgenommen.
- **Werbung und Tracker blockieren** (an): sperrt Werbung, Tracker und
  ungefragte Werbefenster nach den Filterlisten, die auch die bekannten
  Werbeblocker nutzen – siehe [Werbeblocker](#werbeblocker).
- **Tracking-Anhängsel entfernen** (an): `utm_…`, `fbclid`, `gclid` und rund
  30 weitere Kennungen fliegen aus jeder Adresse, bevor sie geladen wird.
- **Vor täuschend ähnlichen Adressen warnen** (an): Wer zum ersten Mal auf
  einer Adresse landet, die einer eigenen Seite zum Verwechseln ähnelt, wird
  vor dem Laden gefragt – `paypa1.com` statt `paypal.com`, kyrillische statt
  lateinischer Buchstaben, ein Tippfehler neben der Bank, oder die bekannte
  Adresse als Vorsatz einer fremden (`paypal.com.konto.example`). Verglichen
  wird nur mit dem, was der Browser selbst weiß: Lesezeichen, gesicherte
  Passwörter, eingetippte und besuchte Seiten. Nichts davon verlässt den Mac.
- **Vor Betrugs- und Schadseiten warnen** (an): Adressen, die als Betrugsseite
  (Phishing) oder als Quelle von Schadprogrammen gemeldet sind, öffnet Zunder
  nicht, sondern zeigt eine Warnseite – mit dem Weg zurück und, für den Fall
  eines Irrtums, „trotzdem öffnen“ (gilt bis zum Beenden, und nur für das
  Engste: die eine gemeldete Adresse oder den einen Rechner, nicht seine
  Nachbarn unter demselben gemeldeten Namen). Gesperrt sind auch
  Abrufe im Hintergrund und Fenster, die eine Seite dorthin öffnen will; der
  Schutz gilt unabhängig vom Werbeblocker, also auch auf Websites, für die er
  abgeschaltet ist. Die beiden Listen holt Zunder zweimal täglich direkt bei
  ihren Herausgebern und prüft dann auf dem Mac – welche Seiten man besucht,
  erfährt dabei niemand. Näheres unter
  [Listen gemeldeter Betrugsseiten](#listen-gemeldeter-betrugsseiten).
- **Eigenes Netz abschirmen** (an): Eine Website aus dem Internet darf weder
  den Router noch den Drucker noch einen Dienst auf diesem Mac ansprechen –
  nicht im Hintergrund (Bilder, Skripte, `fetch`, WebSockets, eingebettete
  Rahmen) und nicht, indem sie den Tab dorthin schickt. Versucht es eine
  Seite, sagt Zunder einmal Bescheid und bietet an, es ihr zu erlauben; die
  Erlaubnis steht in den Einstellungen und lässt sich dort zurücknehmen.
  Seiten, die selbst im eigenen Netz liegen, und Adressen, die man selbst
  eintippt, sind nicht betroffen.
- **Härtung** (aus): schaltet WebKits Lockdown-Modus für alle Tabs ein – ohne
  JIT, WebAssembly, WebGL und Webfonts. Der stärkste Schutz vor präparierten
  Seiten; manche Seiten werden langsamer oder sehen anders aus, und PDFs
  zeigt WebKit dann nicht an (ein geöffnetes wird heruntergeladen, ein
  eingebettetes bleibt weg).
- **Entwicklerwerkzeuge** (aus): „Element untersuchen“ gibt es erst nach dem
  Einschalten. Solange es aus ist, kann auch kein anderes Programm über Safaris
  Entwicklerwerkzeuge in die Seiten hineinsehen.
- **Beim Beenden löschen** (aus): Verlauf, Cookies, Website-Daten,
  Zwischenspeicher und die Sitzung sind nach dem Beenden weg.

![Rückfrage bei einer täuschend ähnlichen Adresse](docs/zunder-aehnliche-adresse.png)

Immer aktiv und ohne Schalter:

- **Die Adressleiste zeigt, wem eine Seite gehört.** Der Teil der Adresse, der
  zählt (`example.com`), steht kräftig da und rutscht nie aus dem Bild – auch
  nicht hinter `konto.bank.de.irgendwas.example.com`. Zugangsdaten in der
  Adresse (`bank.de@fremd.example`) werden nicht angezeigt, Namen in fremden
  Schriften stehen in ihrer kodierten Form da.
- **Das Passwort einer Website auf einer anderen:** Tippt jemand ein
  gesichertes Passwort auf einer Seite ein, zu der es nicht gehört, warnt
  Zunder sofort – noch vor dem Abschicken, wenn man kurz innehält.
- **Andere Programme:** Eine Seite öffnet nie etwas ohne Rückfrage, und nur der
  Tab, den man vor sich hat. Netzlaufwerke, Fernzugriffe und Systemfunktionen
  (`smb:`, `ssh:`, `vnc:`, Kurzbefehle, Systemeinstellungen …) reicht Zunder
  aus einer Webseite heraus gar nicht weiter.
- **Dialoge** einer Seite nennen den Rahmen, der sie ausgelöst hat – nicht die
  Adresse, zu der die Seite gerade unterwegs ist. Tabs im Hintergrund zeigen
  keine; eine Seite, die damit nervt, lässt sich stummschalten, und das
  übersteht auch ihr Neuladen. Nach Benutzername und Passwort fragt nur die
  Seite selbst, nicht ein eingebundenes Bild von einem fremden Rechner.
- **Downloads** landen immer direkt im Download-Ordner, nie als versteckte
  Datei und nicht in Serie; macOS prüft sie vor dem ersten Öffnen. Private
  Fenster hinterlassen keine Zeile in der Download-Liste.
- **Vervollständigt** wird in der Adressleiste nur zu Seiten, die man schon
  einmal eingetippt oder als Lesezeichen gemerkt hat. Eine Seite, auf die man
  bloß geraten ist, kann sich nicht vordrängen.
- **Zunders Oberfläche lädt nichts aus dem Netz** – mit einer Ausnahme, die
  man selbst öffnet: eingebaute Apps holen ihre Daten bei den Diensten, die
  für sie eingetragen sind. Seiten-Icons zeichnet ein
  Skript in der Seite selbst auf 32 × 32 Punkte; die Oberfläche bekommt nur
  dieses Bild. Keine Website erfährt so, wann jemand den Verlauf öffnet oder
  ihre Adresse tippt.
- Seiten laufen in den abgeschotteten Prozessen von WebKit, Pop-ups gehen nur
  nach einem Klick auf, Kamera und Mikrofon gibt es nur auf Nachfrage (Zunder
  fragt selbst und merkt sich die Antwort nur, wenn man es will – siehe
  [Rechte je Website](#rechte-je-website)), das Profil ist nur für den eigenen
  Nutzer lesbar, und die App selbst ist mit „Hardened Runtime“ signiert.

**Der Browser selbst läuft in der Sandbox von macOS.** Nicht nur die Seiten
sind abgeschottet (das macht WebKit), auch das Programm darum herum – Fenster,
Verlauf, Passwort-Tresor, Assistent – kommt nur an sein eigenes Profil, an den
Download-Ordner und an Dateien, die man ihm im Dateidialog ausdrücklich gibt.
Der Rest des Benutzerordners ist für Zunder unerreichbar. Das Profil liegt
seitdem unter `~/Library/Containers/app.zunder.browser/`; ein Profil aus der
Zeit davor holt Zunder beim ersten Start einmal herüber (Cookies und
Anmeldungen bei Websites nicht – dort meldet man sich einmal neu an).

![Zwei Seiten nebeneinander, jede mit eigener Adresszeile](docs/zunder-nebeneinander.png)

Stehen zwei Tabs nebeneinander, hat jede Hälfte ihre eigene Adresszeile – so
ist immer zu sehen, welche Seite wo steht, gleich, in welcher gerade
gearbeitet wird.

Grenzen, ehrlich benannt: Der Schutz des eigenen Netzes prüft die Adresse,
nicht, wohin ein Name am Ende zeigt – ein öffentlicher Name, der auf ein Gerät
im Heimnetz umgebogen wurde, fällt nicht auf. Die Warnung vor ähnlichen
Adressen kennt nur die eigenen Seiten; die Listen gemeldeter Betrugsseiten
kennen nur, was schon jemand gemeldet hat – eine Betrugsseite von heute Morgen
steht noch nicht darin.
Und alles, was WebKit selbst betrifft (Lücken in der Darstellung von Seiten),
schließt Apple mit den Updates von macOS – Zunder kann dort nur die
Angriffsfläche verkleinern (Härtung).

## Werbeblocker

![Filterlisten in den Einstellungen](docs/zunder-filterlisten.png)

Der Blocker ist von Haus aus an und arbeitet in zwei Schichten:

1. Eine **eingebaute Liste** der rund 180 bekanntesten Werbe- und
   Tracking-Dienste. Sie steckt im Programm und gilt vom ersten Abruf an.
2. Die großen Filterlisten **EasyList, EasyList Germany und EasyPrivacy** –
   zusammen rund 140.000 Regeln. Sie liegen der App bei (unverändert, siehe
   `assets/filters/`), werden alle vier Tage direkt von easylist.to
   aufgefrischt und von Zunder in das Regelformat von WebKit übersetzt
   (`src/filters.rs`). WebKit prüft die Abrufe dann selbst: Der Blocker liest
   keine Seite mit und kostet beim Surfen keine Zeit. Übersetzt wird nur, wenn
   sich die Listen ändern (rund sechs Sekunden im Hintergrund); beim Start
   stehen die Listen in weniger als einer Sekunde.

Gesperrt werden Abrufe (Skripte, Bilder, eingebettete Rahmen, Zählpixel …),
Werbefenster und einzelne Seiten, die sich ungefragt öffnen; ausgeblendet
werden die leeren Flächen, die Werbung sonst hinterlässt. Was sich in WebKits
Regelsprache nicht genau ausdrücken lässt (Skripte, Umleitungen, erweiterte
Auswahl-Sprachen), fällt weg – und zwar immer zur vorsichtigen Seite: Eine
Sperre wird höchstens enger, eine Ausnahme höchstens weiter als in der Liste.

- Das **Schild in der Adresszeile** zeigt, wie viele Abrufe auf der Seite
  gesperrt wurden. Ein Klick darauf schaltet den Blocker **nur für diese
  Website** ab oder an – für Seiten, die mit ihm nicht zurechtkommen.
- Manche Websites sperren Besucher mit Werbeblocker aus. Zunder umgeht solche
  Sperren nicht. Es sagt aber, woran es liegt, und bietet mit einem Klick an,
  den Blocker für genau diese Website abzuschalten.
- **Grenzen:** Werbung, die eine Website selbst und untrennbar vom Inhalt
  ausliefert (etwa Werbepausen in Videos), fängt ein Blocker dieser Bauart
  nicht ab. Dazu bräuchte es eingeschleuste Skripte – darauf verzichtet Zunder.

Die Listen stammen von den EasyList-Autoren (<https://easylist.to/>) und stehen
unter GPL v3 oder CC BY-SA 3.0; Näheres in `assets/filters/README.md`.

### Listen gemeldeter Betrugsseiten

Der Schutz vor Betrugs- und Schadseiten (Einstellungen → Sicherheit) nutzt
denselben Übersetzer und dieselbe Technik wie der Werbeblocker, aber eigene
Listen – rund 70.000 Einträge:

- **Phishing URL Blocklist** von malware-filter (gespeist aus OpenPhish,
  PhishTank und IPThreat), <https://gitlab.com/malware-filter/phishing-filter>,
  CC BY-SA 4.0.
- **uBlock filters – Badware risks** der uBlock-Origin-Autoren,
  <https://github.com/uBlockOrigin/uAssets>, GPL v3.

Diese Listen liegen der App nicht bei: Zunder holt sie kurz nach dem ersten
Start und danach alle zwölf Stunden. Bis sie da sind, gibt es diesen Schutz
noch nicht; die
Einstellungen zeigen, wie viele Einträge gerade gelten. Steht eine ganze
Website auf der Liste, ist sie als Ganzes gesperrt; ist nur eine einzelne
Adresse gemeldet (etwa eine Seite auf einem Dienst für Formulare), dann nur
diese.

## Stealth-Fenster (⌥⌘N)

![Startseite eines Stealth-Fensters über Tor](docs/zunder-stealth.png)

So wenig Spuren wie möglich – auf diesem Mac und im Netz. Ein Stealth-Fenster
ist ein privates Fenster mit zusätzlichen Riegeln:

- **Nichts bleibt auf dem Mac:** kein Verlauf, keine Cookies, kein
  Zwischenspeicher. Nur Downloads und ausdrücklich gesetzte Lesezeichen bleiben.
- **Jeder Tab ist eine Insel:** Jeder Tab hat seinen eigenen, flüchtigen
  Datenspeicher. Was eine Website in einem Tab speichert, sieht kein anderer –
  Websites erkennen niemanden von Tab zu Tab wieder. (Tabs, die eine Seite
  selbst öffnet, teilen sich die Insel ihrer Herkunft, sonst funktionierten
  Anmelde-Fenster nicht.)
- **Die Schutzschalter gelten immer:** Blocker, nur verschlüsselt, ohne
  Tracking-Anhängsel, Schutz des eigenen Netzes – gleich, was sonst
  eingestellt ist.
- **Der Fingerabdruck wird verwischt:** WebKits Lockdown-Modus (kein JIT,
  WebAssembly, WebGL), kein WebRTC (es spräche am Tunnel vorbei), und WebKits
  eigener Schutz gegen Fingerabdrücke: Was Seiten von einer Zeichenfläche
  zurücklesen, ist verrauscht – in jedem Tab anders –, und als Bildschirmgröße
  sehen sie die Größe des Fensters.
- **Über Tor, wenn Tor läuft:** Findet Zunder auf dem Mac einen Tor-Zugang
  (Port 9050 des Tor-Dienstes oder 9150 des Tor Browsers), geht der gesamte
  Verkehr des Fensters dort hindurch – jeder Tab über einen eigenen Weg, Namen
  werden erst im Tor-Netz aufgelöst. Fällt Tor aus, lädt nichts mehr: Am
  Tunnel vorbei geht nichts. Einrichten: `brew install tor && brew services
  start tor`, danach ein neues Stealth-Fenster öffnen.

Was es **nicht** kann, sagt die Startseite des Fensters offen: Ohne Tor sehen
Websites und Netzbetreiber weiterhin die IP-Adresse. Und auch mit Tor ist
Zunder nicht der Tor Browser – dessen Besucher sehen für Websites alle gleich
aus, die von Zunder nicht. Wer sich irgendwo anmeldet, ist dort erkennbar.

### Tarn-Ampel

![Die Tarn-Ampel mit den Abfragen einer Seite](docs/zunder-tarn-ampel.png)

In der Adresszeile eines Stealth-Fensters zeigt eine Ampel, wie gut man auf
der Seite getarnt ist, die gerade offen ist:

- **Grün:** über Tor, verschlüsselt, Blocker an, kein Passwort eingegeben.
- **Gelb:** Die Website sieht die IP-Adresse (Tor läuft nicht) – oder der
  Blocker ist für diese Website abgeschaltet.
- **Rot:** Die Verbindung ist unverschlüsselt – oder man hat auf dieser
  Website ein Passwort eingegeben und ist ihr damit bekannt.

Ein Klick zeigt die Gründe und dazu, **was die Seite selbst per JavaScript
abgefragt hat**: Bildschirmgröße, Zeichenfläche (typisch für Fingerabdrücke),
Grafikkarte, Geräte-Eigenschaften, Browser-Kennung, Sprache, Zeitzone,
Schriften, Cookies, Speicher, Standort, Kamera und Mikrofon, Zwischenablage –
jeweils mit Anzahl, auch für eingebettete Rahmen. Gezählt wird, was über die
üblichen Wege läuft (`ui/watch.js`); das ist eine Auskunft, kein Beweis: Eine
Seite, die es darauf anlegt, kann daran vorbei fragen.

## Security-Ampel und Seitencheck (⌥⌘S)

![Der Seitencheck einer Seite ohne Schutzschichten](docs/zunder-seitencheck.png)

Links in der Adresszeile, wo sonst das Schloss sitzt, steht eine kleine Ampel.
Sie sagt, wie die **gezeigte Seite** dasteht – nicht Zunder, die Seite:

- **Rot**: Hier ist jetzt etwas gefährdet – die Verbindung ist unverschlüsselt,
  ein Formular sendet offen, ein Passwortfeld steht auf einer
  unverschlüsselten Seite, die Verschlüsselung ist veraltet (TLS 1.0/1.1),
  oder die Seite lädt Skripte von einem Dienst, der nachweislich Schadcode
  ausgeliefert hat (polyfill.io).
- **Gelb**: Der Seite fehlen Schutzschichten.
- **Grün**: Die geprüften Schutzschichten sind da.

Die Zahl neben den Lichtern zählt die Stellen. Ein Klick auf die Ampel (oder
⌥⌘S) öffnet den **Seitencheck**: jede Stelle mit einem Satz dazu, wie die
Seite dort angreifbar wäre, darunter Hinweise und was in Ordnung ist.

| Wo | Was der Check ansieht |
| --- | --- |
| Verbindung | unverschlüsselt · veraltete Verschlüsselung · gemischte Inhalte · ob die Website Verschlüsselung erzwingt (HSTS) |
| Eingeschleuster Code | Content-Security-Policy: fehlt, nur im Beobachtungsmodus, oder mit Lücken (`'unsafe-inline'`, Skripte von jeder Adresse, `data:`) – mehrere Policies werden zusammen gelesen, Kennwort/Prüfsumme und `'strict-dynamic'` berücksichtigt |
| Einbetten | weder `X-Frame-Options` noch `frame-ancestors`: Die Seite lässt sich unsichtbar über fremde Knöpfe legen (Clickjacking) |
| Skripte | von fremden Rechnern (wie viele, woher) · von öffentlichen Sammelstellen ohne Prüfsumme · veraltete Bibliotheken mit bekannten Lücken: jQuery, jQuery UI, Bootstrap, Lodash, Moment.js, Handlebars sowie die abgekündigten Reihen AngularJS 1 und Vue 2 |
| Formulare | senden an eine `http`-Adresse · Passwortfeld auf unverschlüsselter Seite |
| Cookies | Sitzungs-Cookies ohne `Secure`, ohne `HttpOnly`, ohne `SameSite` – gezeigt werden Namen, nie Werte |
| Auskunft | die Website verrät ihre Software samt Version · meldet eine PHP-Reihe, die der Hersteller nicht mehr pflegt · stellt das Dateityp-Raten nicht ab · gibt volle Adressen an fremde Seiten weiter |

**Wie geprüft wird:** nur mit dem, was beim Laden ohnehin anfällt. Den
Antwortkopf liest Zunder mit, wenn WebKit die Seite lädt; den Aufbau liest ein
Skript in Zunders abgeschotteter Welt (`ui/check.js`), das die Seite nicht
bemerkt; die Merkmale der Cookies kommen aus WebKits Speicher. Es gibt keinen
zusätzlichen Abruf und keinen Angriff, und nichts davon verlässt den Mac. Die
Auswertung steht in `src/sitecheck.rs` und ist dort mit Tests belegt.

**Grenzen:** Das ist kein Sicherheitstest. Lücken im Programm einer Website
sieht man von außen nicht – „Solide“ heißt nur: Die geprüften Schutzschichten
sind da. Bibliotheken erkennt der Check nur, wenn ihre Version in der Adresse
des Skripts steht; was in ein großes Bündel verpackt ist, bleibt unerkannt.
Steht eine Website auf der Vorab-Liste der Browser für erzwungene
Verschlüsselung, schickt aber den Kopf dazu nicht, meldet der Check
„nicht erzwungen“. Und was eine Website über sich sagt, zeigt Zunder nur in
fester Form („Name/Version“, Namen von Cookies ohne Sonderzeichen): Freier
Text aus einer Website stünde sonst in Zunders eigenem Fenster, als hätte der
Browser ihn geschrieben.

## Rechte je Website

![Die Übersicht „Rechte je Website“ mit fünf Websites](docs/zunder-rechte.png)

Will eine Seite die Kamera oder das Mikrofon benutzen, fragt Zunder – mit dem
Namen der Website, die fragt. Ohne ein Ja bekommt keine Seite etwas; wer
keinen Namen hat (eine Datei, ein abgeschottetes Dokument), bekommt gar
nichts.

- **Die Antwort gilt für dieses eine Mal.** Mit dem Häkchen „Für … merken“
  gilt sie für die Website: Ein gemerktes Ja lässt die Seite künftig ohne
  Frage aufnehmen, ein gemerktes Nein lässt sie gar nicht mehr fragen.
  Gemerkt wird nur für verschlüsselte Websites (`https`) und nie in privaten
  oder Stealth-Fenstern.
- **Ein Nein ohne Merken** gilt für die Website in diesem Tab, bis man die
  Seite selbst neu lädt oder selbst eine Adresse aufruft. Die Frage hält den
  ganzen Browser an, und keine Seite soll ihn damit festhalten können: Lädt
  eine Seite sich selbst neu oder schickt sie den Tab zu einer zweiten und
  zurück, kommt keine neue Frage. Und nach drei abgelehnten Fragen in Folge
  stellt in dem Tab niemand mehr eine, bis man selbst weiterklickt oder
  blättert.
- **Ein gemerktes Ja gilt nur für die Seite selbst.** Was sie einbettet (ein
  Videofenster, eine Anzeige), erbt es nicht: Für einen eingebetteten Rahmen
  fragt Zunder jedes Mal und nennt beide – den Rahmen und die Seite, in der er
  steckt.
- **Ein gemerktes Nein gilt überall:** für alles, was die Website einbettet,
  dort, wo sie selbst nur als Rahmen in einer anderen Website steckt, und in
  privaten Fenstern.
- **„Erlauben“ lässt sich erst drücken, wenn die Frage einen Augenblick lang
  zuoberst stand**, und die Eingabetaste steht auf „Nicht erlauben“. Eine
  Seite, die zu schnellen Klicks oder Tastendrücken verleitet und die Frage
  genau dann aufgehen lässt, bekommt so kein Ja untergeschoben. Solange die
  Frage offen ist, kann keine Seite einen eigenen Dialog darüberlegen – und
  die Frage legt sich über keinen anderen.
- Ein Tab im Hintergrund bekommt keine Frage – und damit auch nichts. Eine
  Website mit gemerktem Ja darf dagegen auch im Hintergrund aufnehmen.
- macOS fragt zusätzlich einmal, ob Zunder als Programm Kamera und Mikrofon
  benutzen darf.

**Die Übersicht** (Einstellungen → Sicherheit → „Rechte je Website“) zeigt für
jede Website, was man ihr erlaubt oder verboten hat: Kamera, Mikrofon, den
Zugriff ins eigene Netz und einen abgeschalteten Werbeblocker. Jede
Entscheidung lässt sich einzeln zurücknehmen; dann gilt wieder das Übliche,
und Zunder fragt beim nächsten Mal. Wer ein Ja zurücknimmt, beendet damit
auch, was die Website in einem offenen Tab gerade aufnimmt. Die Liste fasst
500 Websites; ist sie voll, bietet die Frage das Merken nicht mehr an.

Der Teil wurde nach dem Bau von unabhängiger Seite durchgesehen; was dabei
herauskam, ist behoben und nachgeprüft: Ein Dialog der Seite konnte sich über
die Frage legen und die Wartezeit von „Erlauben“ verdeckt ablaufen lassen, ein
gemerktes Nein galt nicht, wenn die Website als Rahmen woanders steckte, und
Zunders eigene Oberfläche verließ sich bei Kamera und Mikrofon allein auf die
Vorgabe der WebView-Bibliothek – die Ja lautet.

Geprüft ist die Abfrage mit WebKits gestellten Testgeräten, nicht mit einer
echten Kamera. Und: Welche Seite gerade aufnimmt, zeigt bisher nur macOS in
der Menüleiste – Zunder selbst markiert den Tab noch nicht.

## Eingebettete PDFs

Web-Apps zeigen ein PDF oft in einem Rahmen, dessen Adresse mit `blob:`
beginnt – die Datei liegt dann nur im Speicher der Seite. Verbietet eine
Website mit `frame-ancestors 'none'`, dass *andere* sie einbetten, wendet
WebKit das Verbot auch auf diesen eigenen Rahmen an: Er bleibt leer. Zunder
hilft ihm auf die Sprünge. Ein eigenes Skript (`ui/pdfs.js`, in Zunders
abgeschotteter Welt) erkennt den geleerten Rahmen, holt die Datei aus der
Seite und gibt sie dem Browser; der Rahmen lädt sie dann unter einer eigenen
Adresse (`zunder-pdf://…`) – an derselben Stelle der Seite, mit dem Betrachter
von WebKit.

- **Nur Zunders Skript kann abliefern.** Jede Lieferung beginnt mit einem
  Schlüssel, der bei jedem Start neu ausgewürfelt wird und nur in der
  abgeschotteten Welt steht. Die Skripte einer Seite kennen ihn nicht.
- **Nur PDFs, und nur als PDF.** Angenommen wird, was die Marke eines PDFs
  trägt und höchstens 64 MB groß ist. Heraus geht es stets als PDF, das WebKit
  weder als Seite noch als Skript deuten darf.
- **Eine Kennung gilt in ihrem Tab und für die Seite, die gerade steht.** Ein
  anderer Tab bekommt unter ihr nichts. Steht im Tab eine neue Seite (auch
  nach dem Neuladen), ist sie ungültig – ein Klick, aus dem nur ein Download
  wird, ändert dagegen nichts. Als Seite oder als neues Fenster lädt Zunder
  die Adresse nie, nur in einem Rahmen.
- **Der Rahmen bleibt der Seite – und dem, der darin klickt.** Führt ein Link
  im PDF den Rahmen woandershin, bleibt er dort. Belegt die Seite ihn neu
  (dieselbe Adresse noch einmal, oder sie hängt ihn neu ein), kommt das PDF
  wieder.
- **Einer nach dem anderen, und wenig auf Vorrat:** Der Browser behält
  höchstens zwölf Dateien oder 128 MB je Tab und 320 MB insgesamt, die
  ältesten weichen. Geholfen wird deshalb einem Rahmen nach dem anderen – was
  weicht, ist dann schon angezeigt.
- **Der Browser lässt sich nicht beschäftigen.** Ein Tab liefert in 30
  Sekunden höchstens 40-mal und 256 MB ab; eine Seite kann beliebig viele
  Rahmen anlegen, und jede Lieferung kostet den Browser selbst Zeit. Ist das
  Maß voll, versucht es Zunders Skript später noch einmal. (Dass eine Seite
  überhaupt etwas an die Adresse schickt, kann Zunder nicht verhindern –
  WebKit reicht es durch, bevor Zunder ablehnen kann.)
- **Eine Absage ist ein gescheiterter Abruf** – ohne Antwort, deren Nummer
  oder Zeilen ein Skript der Seite lesen könnte. Dass hier Zunder am Werk ist,
  kann eine Seite trotzdem herausfinden: an der Ausnahme von ihren
  Schutzregeln, siehe unten.
- **Zwei Zusagen an WebKit** braucht der Weg: Die Schutzregeln einer Seite
  (`frame-src`) lassen `zunder-pdf:` zu, und in einer verschlüsselt geladenen
  Seite gilt die Adresse nicht als unsicherer Inhalt. Ohne die erste sperrt
  die Regel der Seite den Rahmen, ohne die zweite kommt die Datei gar nicht
  beim Browser an (beides ausprobiert). Sie gelten nur für diese eine Adresse,
  und was unter ihr zu holen ist, bestimmt allein der Browser.

Der Teil wurde nach dem Bau von unabhängiger Seite durchgesehen. Einen Weg, an
den Schlüssel zu kommen, eine Lieferung zu fälschen, an die Datei eines
anderen Tabs zu gelangen oder Abgelegtes als Skript ausführen zu lassen, fand
die Durchsicht nicht. Was sie fand, ist behoben und nachgeprüft: Viele Rahmen
auf einmal verdrängten einander, bevor sie abgeholt waren (von 46 kamen 13
an – jetzt alle); ein Link in einem PDF führte nicht weiter, weil die Hilfe
den Rahmen zurückholte; die Kennungen verfielen schon beim Aufbruch zu einer
anderen Seite statt bei deren Ankunft; eine Website konnte das Maß des Tabs
für die nächste aufbrauchen; und eine Absage war als Antwort mit Nummer zu
lesen.

Geprüft ist das an einer nachgebauten Seite und an einer echten Web-App mit
solchen Kopfzeilen, mit und ohne Sandbox, im privaten Fenster und in einem
Tab, den eine Seite öffnet; 30 MB brauchen etwa eine Sechstelsekunde. In der
eingespielten App ist es außerdem an einem echten Dokument hinter einer
Anmeldung bestätigt. Nicht geholfen wird Dateien über 64 MB, abgeschotteten
Rahmen (`sandbox`), Rahmen, die selbst in einem eingebetteten Rahmen oder in
einem Schattenbaum stecken, und allem, was kein PDF ist.

### Mit Härtung und im Stealth-Fenster

Dort zeigt WebKit PDFs überhaupt nicht an – die Hilfe bleibt deshalb ganz
weg. Stattdessen macht WebKit aus jedem PDF, das angezeigt werden sollte, von
sich aus einen Download. Öffnet man ein PDF, ist das der einzige Weg zur
Datei. Es galt aber auch für PDFs, die eine Seite nur einbettet: Sie landeten
ohne jedes Zutun im Download-Ordner, bei jedem Besuch der Seite aufs Neue.
Solche Downloads sagt Zunder ab, bevor es für sie ein Ziel gibt, und sagt
einmal je Website Bescheid, warum der Rahmen leer bleibt. (Leert schon eine
Regel der Seite selbst ihren Rahmen – der Fall oben –, kommt es gar nicht zu
einem Download: Dann bleibt er ohne Hinweis leer.)

- **Was bleibt:** ein PDF, das man als Seite öffnet; ein Link oder ein
  Formular, das man selbst mit der Maus anklickt und das einen Rahmen zu
  einem PDF führt; Links mit `download`; und alles, was der Server
  ausdrücklich als Datei ausweist (`Content-Disposition: attachment`).
- **Was nicht zählt:** alles, was ein Skript der Seite in einen Rahmen lädt –
  beim Aufbau, verzögert oder auf einen Klick hin. So betten Web-Apps ihre
  PDFs ein: Man klickt auf einen Eintrag, die Seite ruft die Datei ab und
  hängt den Rahmen ein. Ebenso ein Link, den ein Skript „anklickt“, und ein
  Formular, das ein Skript abschickt – auch mitten in einem echten Klick
  anderswo auf der Seite.
- **Ein Klick deckt einen Download:** den aus der ersten Antwort, die danach
  für die angeklickte Adresse eintrifft (Weiterleitungen eingeschlossen),
  höchstens eine Minute lang. Lädt die Seite dieselbe Adresse später noch
  einmal von sich aus, zählt er nicht mehr.

Woran Zunder den echten Klick erkennt: WebKit nennt zu einer Navigation eine
Maustaste nur, wenn wirklich eine gedrückt wurde. Dass „ein Mensch
dahintersteht“, meldet es dagegen auch für alles, was ein Skript im Gefolge
eines Klicks tut – selbst wenn die Seite erst Daten abruft und den Rahmen
später einhängt (beides gemessen auf macOS 26). Die Kehrseite: Ein Link in
einem Rahmen, den man mit der Tastatur auslöst, gilt nicht als angeklickt.
Und ein Knopf, auf den hin die Seite selbst ein PDF in einen Rahmen lädt
(„Rechnung anzeigen“), bringt mit Härtung weder Anzeige noch Download –
ansehen lässt sich ein solches PDF dann nur über einen Link der Seite oder
ohne Härtung.

Auch dieser Teil wurde von unabhängiger Seite durchgesehen, zweimal.
Schwerwiegendes fand sich nicht; geändert wurde danach die Ausnahme für
Klicks: Ein Klick
deckt nur noch die erste Antwort auf seine Adresse (vorher galt er eine
Minute lang für jede), Weiterleitungen hinterlassen keine Zwischenstationen,
ein Tab kann die Klicks eines anderen nicht verdrängen, eine neue Seite im
Tab lässt die alten verfallen, und als echter Klick gilt nur noch einer mit
Maustaste (vorher zählte auch ein Link, den ein Skript während eines echten
Klicks „anklickte“). Vermerkt wird ein Klick erst, wenn feststeht, dass die
Navigation stattfindet und eine Antwort folgt – nicht bei einem Link mit
`download` und nicht bei einem Sprung im selben Dokument. Der Hinweis zählt
außerdem nicht mehr als gezeigt, wenn ihn sofort ein anderer verdrängt. Der
Verdacht, ein Skript komme über ein angeklicktes Etikett oder vorgetäuschte
Tastendrücke doch an eine Maustaste, hat sich im Probelauf nicht bestätigt.

Geprüft mit Härtung und im Stealth-Fenster, mit und ohne Sandbox und in einem
Tab, den eine Seite öffnet: Rahmen mit Adresse und aus `blob:` (wie bei der
Web-App oben) und 46 Rahmen auf einmal – vorher fünf Downloads, jetzt
keiner. Mit der Maus angeklickt bleiben Link (auch über zwei
Weiterleitungen), Formular und Link mit Ziel-Rahmen. Abgesagt werden ein
Knopf, auf den hin ein Skript den Rahmen belegt, ein Link, den ein Skript
anklickt, ein Formular, das ein Skript abschickt, Abruf mit anschließendem
Rahmen, zehn Rahmen auf einen Klick und das erneute Laden einer schon
angeklickten Adresse. Ohne Härtung ändert sich nichts – dort macht WebKit
aus solchen Antworten keinen Download. (Ausnahme: der Blockierungsmodus von
macOS. Er gilt für jede WebView, auch ohne Zunders Härtung; Zunder verhält
sich dann wie mit ihr.)

## Eingebaute Apps

Eine App ist eine eigene Seite des Browsers in einem Tab – `zunder://app-…/` –,
geladen aus dem Programm selbst. Man öffnet sie über ihre Kachel auf der
Startseite, über „Ablage“ im Menü oder über ihre Adresse. Es gibt vier:
**Wetter und Regenradar**, die **Zeiterfassung**, den **Sicherheitscheck** und
den **Request-Inspektor**.

### Was eine App darf

Mehr als eine Webseite, aber nur, was für sie eingetragen ist
(`src/apps.rs`):

- **Netz:** nur zu den Diensten, die bei ihr stehen – getrennt nach Daten und
  Bildern. Das setzt WebKit selbst durch: über die Schutzregel, mit der der
  Browser ihre Seite ausliefert. Fremde Skripte, Rahmen und alles andere im
  Netz erreicht sie nicht.
- **Standort:** nur nach Zustimmung je App. Gefragt wird in der Leiste des
  Browsers, nicht in der Seite; macOS fragt beim ersten Mal zusätzlich selbst.
  Die App bekommt den Standort auf zwei Nachkommastellen gerundet (gut ein
  Kilometer). Im Stealth-Fenster gibt es keinen, und Webseiten bietet Zunder
  ihn weiterhin gar nicht an.
- **Speicher:** ein kleiner Zettel beim Browser (höchstens 3000 Bytes) und –
  wo es für die App eingetragen ist – Karteikarten: viele kleine Einträge,
  jeder unter seiner Kennung (je höchstens 1500 Bytes; die Zeiterfassung darf
  20 000 haben). Beides liest keine andere App und keine Webseite. Private
  Fenster lesen und schreiben nichts davon. Es liegt in der Datenbank des
  Browsers wie die Lesezeichen, nicht bei den Website-Daten: „Verlauf und
  Website-Daten löschen“ lässt es stehen.
- **Dateien sichern:** nur Dateien, die die App selbst erzeugt, nur mit einer
  Endung, die für sie eingetragen ist (die Zeiterfassung: `csv`, der
  Sicherheitscheck: `md`), und nur über den gewöhnlichen Weg der Downloads – in
  den Download-Ordner, ohne eine vorhandene Datei zu überschreiben.
- **Abrufen:** Der Sicherheitscheck darf den Browser bitten, eine Adresse
  abzurufen und zu prüfen, der Request-Inspektor, eine frei zusammengestellte
  Anfrage zu schicken. Den Abruf macht der Browser selbst (`src/security.rs`,
  `src/inspect.rs`), nie die Seite der App; im Stealth-Fenster gar nicht.

Laden kann eine App allein der Browser – nie eine Webseite, weder im Tab noch
in einem Rahmen oder einem neuen Fenster. Die Anmeldung, mit der der Browser
eine eigene Seite lädt, gilt dabei genau der einen WebView: Eine Webseite in
einem anderen Tab, die es im selben Augenblick mit derselben Adresse versucht,
kann sie nicht für sich verbrauchen. Um etwas bitten kann den Browser nur die
Seite der App selbst: Den Absender jeder Nachricht nennt WebKit. Und die
Antwort prüft an Ort und Stelle, in welchem Dokument sie läuft – zeigt der Tab
inzwischen etwas anderes, verpufft sie. In einem Stealth-Tab, dem der Tunnel
fehlt, lädt keine App. Im Verlauf zu einer App zurückblättern kann ein Tab wie
zu jeder früheren Seite; ihre eigenen Links nach draußen öffnet die App
deshalb in einem neuen Tab, ohne Verbindung zurück.

### Wetter und Regenradar

- **Jetzt und demnächst:** Temperatur, gefühlte Temperatur, Wind, Böen,
  Luftfeuchte, Luftdruck, Sonnenauf- und -untergang; der Niederschlag der
  nächsten zwei Stunden in Viertelstunden; 24 Stunden und sieben Tage. Der
  Himmel hinter der Seite folgt dem Wetter.
- **Regenradar:** die letzte Stunde gemessen, bis zu zwei Stunden
  vorhergesagt, alle fünf Minuten ein Bild – als Film, oder am Schieber auf
  eine Zeit gestellt. Ausschnitt 50, 100 oder 150 km um den Ort, mit
  Entfernungsringen, Städten und einem Satellitenbild dahinter. Das Radar
  reicht über Deutschland und einen Streifen seiner Nachbarn.
- **Ort:** der eigene Standort – oder ein Ort nach Name oder Postleitzahl.
  Postleitzahlen sucht die App zuerst im Land des Macs; so lässt sich auch
  nachbessern, wenn die Ortung nicht genau trifft. Bis zu acht Orte merkt sie
  sich.

Woher die Daten kommen – und was dabei wohin geht:

| Was | Dienst | Dorthin geht |
|---|---|---|
| Vorhersage, Ortssuche nach Namen | [Open-Meteo](https://open-meteo.com/) (CC BY 4.0; Ortsnamen: GeoNames) | der gerundete Ort bzw. der eingetippte Name |
| Regenradar | Deutscher Wetterdienst, ausgeliefert von [Bright Sky](https://brightsky.dev/) (CC BY 4.0) | der gerundete Ort |
| Satellitenbild | NASA Blue Marble, ausgeliefert von NASA GIBS | der gezeigte Ausschnitt |
| Postleitzahlen | Nominatim (© OpenStreetMap-Mitwirkende, ODbL) | die eingetippte Postleitzahl |

Jeder dieser Dienste sieht dabei wie bei jedem Abruf die Netzadresse des
Rechners. Die App fragt nur, solange ihr Tab zu sehen ist: die Vorhersage alle
zehn Minuten, das Radar alle fünf. Das Radar zeichnet sie selbst aus den
Messwerten – Kartenkacheln eines fremden Dienstes braucht es dafür nicht.

Geprüft im Probelauf: Kachel, Menü und eingetippte Adresse öffnen die App;
eine Webseite kann sie weder öffnen noch ihren Kanal benutzen; fremde
Rechner, Skripte, Bilder und Rahmen sind ihr gesperrt; die Rückfrage zum
Standort, das Merken von Orten, privates und Stealth-Fenster. Der Standort
selbst ist nur mit einem gestellten Ort geprüft – die echte Abfrage bei macOS
löst erst die eingespielte App aus.

### Zeiterfassung

Zeiten buchen in einer Zeile, ohne Konto und ohne Netz (⌥⌘Z, Kachel auf der
Startseite oder `zunder://app-time/`). Die Zeile liest, was man tippt, und
zeigt vor dem Buchen, was daraus wird:

| Eingabe | Ergebnis |
|---|---|
| `9-11:30 #Kunde Besprechung` | Eintrag von 9:00 bis 11:30 auf das Projekt „Kunde“ |
| `45m Mails` · `1,5h` · `1h30` · `1:30` | Eine Dauer: Sie endet jetzt. Ist dort schon gebucht, schließt sie an den letzten Eintrag an oder rückt in die späteste freie Lücke davor – so lässt sich abends ein ganzer Tag nacheinander buchen |
| `gestern 2h Angebot` · `fr 9-12` · `23.11. 8-10` | Dasselbe für einen anderen Tag (das Wort für den Tag steht vorn; ein Datum ohne Jahr meint das, das heute am nächsten liegt) |
| `#Kunde Angebot` | Die Uhr läuft ab jetzt; `ab 9 #Kunde` lässt sie seit 9 Uhr laufen. Sie steht im Titel des Tabs und läuft weiter, wenn der Tab oder der Browser zu ist |

Die Zeit steht vorn oder hinten in der Zeile (`Mails 45m` geht auch) – was
mitten in der Notiz steht, bleibt Notiz: „Fehler seit 2 Wochen“ ist keine
Uhrzeit.

- **Projekte** entstehen beim Buchen: `#Name`; für ein bekanntes genügt der
  Anfang (`#ku`), `#"Name mit Leerzeichen"` gilt wörtlich, `#-` bucht ohne
  Projekt. Ohne Angabe gilt das zuletzt benutzte – oder das, dessen Knopf
  unter der Zeile vorgemerkt ist. Die Knöpfe „+15 min“, „+30 min“ und „+1 h“
  buchen mit einem Klick.
- **Der Tag** als Liste mit Summe und Soll; Lücken zwischen zwei Einträgen
  lassen sich mit einem Klick in die Zeile übernehmen, Überschneidungen sind
  markiert. Ein Klick auf einen Eintrag ändert ihn; Buchen, Löschen, Stoppen
  und Verwerfen lassen sich zurücknehmen.
- **Pause und Weiterführen:** Die laufende Uhr hat „Pause“ und „Weiter“ – sie
  läuft danach im selben Eintrag weiter, und die Pause steht darin. Play an
  einem Eintrag von heute fragt: diesen Eintrag weiterführen (die Zeit seit
  seinem Ende steht dann als Pause darin) oder einen neuen beginnen; bei
  älteren Einträgen beginnt gleich ein neuer. Pausen zählen nicht zur
  gearbeiteten Zeit, stehen als Marke am Eintrag und lassen sich beim
  Bearbeiten entfernen. In die Pause eines Eintrags passt ein anderer, ohne
  dass sich etwas überschneidet.
- **Die Woche** als Balken je Tag, nach Projekten gefärbt, mit Soll und der
  Summe je Projekt; dazu die Summe des Monats.
- **Kalender:** Ein Klick auf das Datum klappt einen Monat auf – mit einem
  Punkt an den Tagen, an denen gebucht ist. Ein Klick auf einen Tag zeigt ihn
  und schließt den Kalender.
- **Einstellungen:** Sollstunden je Wochentag, Runden auf 1, 5 oder 15
  Minuten, Projekte umbenennen, färben und ablegen.
- **Als Tabelle sichern:** der Monat oder alles als CSV-Datei bei den
  Downloads (Semikolon als Trenner; Pause in Minuten, die gearbeitete Zeit
  als gerundete Dezimalzahl in Stunden und daneben genau in Minuten). Felder, die eine Tabellenkalkulation als Formel
  läse, bekommen ein Hochkomma davor.

Die Daten liegen nur auf diesem Mac, in der Datenbank des Browsers: jeder
Eintrag und jedes Projekt als eigene Karteikarte, Einstellungen und die
laufende Uhr auf dem Zettel der App. Ins Netz geht diese App nicht – ihre
Schutzregel erlaubt ihr keinen einzigen Rechner. Sind zwei Tabs oder Fenster
mit ihr offen, liest jeder den Stand neu, sobald er wieder nach vorn kommt
oder die Eingabe bekommt – und jede Änderung wartet darauf, damit ein
veralteter Tab nicht etwa die Uhr überschreibt, die im anderen läuft. Lässt
sich der Stand nicht lesen, zeigt und ändert sie nichts. Im privaten Fenster
bucht sie nur für die Dauer des Fensters.

Geprüft im Probelauf, auch in der abgeschotteten Fassung: Buchen in allen
Schreibweisen, Uhr starten, stoppen und über einen Neustart des Browsers
hinweg, Pause und Weiterführen (auch pausiert über einen Neustart), Ändern,
Löschen und Zurücknehmen, zwei Tabs (auch ein veralteter, der
sofort bucht oder stoppt), privates Fenster (nichts gelesen, nichts abgelegt,
nichts verloren beim Tabwechsel), die Tabelle samt Namen der Datei – und dass eine
Webseite weder an die Karteikarten kommt noch die App lädt, auch nicht, wenn
sie es im selben Augenblick versucht, in dem der Browser sie öffnet.

### Sicherheitscheck

Gedacht, damit man die **eigenen** Seiten härten kann: Man gibt die Adresse
einer Website ein, und Zunder prüft, welche Schutzschichten ihr fehlen. Anders
als der Seitencheck (⌥⌘S), der nur auswertet, was beim gewöhnlichen Laden
ohnehin anfällt, ruft diese App eine Adresse **eigens** ab – so liest Zunder den
ganzen Antwortkopf, die Cookies und den Quelltext, mehr als eine Seite im Tab
über sich preisgibt.

Abgerufen wird vom Browser selbst (`src/security.rs`), nicht von der Seite der
App: Die geht gar nicht ins Netz. Die Bewertung ist dieselbe wie hinter der
Security-Ampel (`src/sitecheck.rs`), um Befunde ergänzt, die erst der eigene
Abruf zeigt. Zu jedem Fund steht, wie man ihn behebt.

- **Was geprüft wird:** Verschlüsselung und HSTS, Schutz vor eingeschleustem
  Code (CSP), Einbetten in fremde Seiten (`X-Frame-Options`/`frame-ancestors`),
  Merkmale der Sitzungs-Cookies (`Secure`, `HttpOnly`, `SameSite`), unverschlüsselt
  geladene Teile, Skripte von fremden Rechnern und Sammelstellen ohne Prüfsumme,
  veraltete Bibliotheken, was die Seite über ihre Software verrät. Dazu, was erst
  der eigene Abruf sichtbar macht: ob `http` von selbst auf `https` führt, ob
  CORS fremde Herkünfte wahllos (und mit Anmeldung) zulässt, ob bekannte Pfade
  wie `/.git/config` oder `/.env` offen liegen, ob der Server Ordnerinhalte
  auflistet, und ob `Cross-Origin-Opener-Policy` und `Permissions-Policy` fehlen.
- **Zum Weiterarbeiten:** Die Befunde lassen sich als Markdown sichern – nach
  Stufen geordnet, das Ernste und die Schwächen als Checkliste zum Abhaken, zu
  jedem Punkt die Behebung. So kann man die eigenen Seiten (oder Apps) Stelle
  für Stelle überarbeiten.
- **Gefahrlos:** Es werden nur **Abrufe** gemacht (GET) – nichts eingeschleust,
  nichts erraten-durchprobiert, keine Last erzeugt. Die „aktiven“ Prüfungen
  rufen eine Handvoll bekannter Pfade ab und sehen nach, ob eine Adresse
  umleitet und wie sie beim Teilen antwortet. Das tut jeder Sicherheits-Scanner;
  einen Angriff stellt es nicht dar. Einschleusen (XSS, SQL), das Durchprobieren
  von Passwörtern und Lastangriffe gehören bewusst **nicht** dazu: Solche
  Nutzlasten wirken gegen jede fremde Seite gleich, und die App kann nicht
  prüfen, wem eine Adresse gehört. Fürs Härten braucht es sie nicht.
- **Grenzen:** Im Stealth-Fenster prüft die App nichts – der eigene Abruf ginge
  am Tunnel vorbei und verriete die echte Adresse. Im eigenen Netz
  (`localhost` …) entfallen die Prüfungen, die nur im offenen Netz Sinn ergeben
  (offene Dateien, `http`→`https`, die Trennschichten). Geprüfte Adressen merkt
  sich die App (bis zu zwölf), im privaten Fenster nicht. Was im Programm einer
  Website steckt, sieht von außen niemand – die App prüft die Schutzschichten,
  nicht den Code dahinter.

Geprüft im Probelauf gegen einen eigens gebauten Server: eine schwache Seite
(rot, mit CORS-Freigabe an jede Herkunft, fehlender CSP, Einbettbarkeit,
veralteter Bibliothek, Cookie ohne Schutz) und eine solide (grün); dazu die
Fehlerfälle (keine Web-Adresse, nicht erreichbar), die Merkliste, privates und
Stealth-Fenster, und einmal eine echte Adresse (`example.com`), um den Abruf im
offenen Netz zu bestätigen.

### Request-Inspektor

Wie `curl`, nur im Browser: Man stellt eine HTTP-Anfrage zusammen – Methode,
Adresse, Kopfzeilen, Rumpf – und sieht die rohe Antwort: Status, Dauer,
Antwortkopf und Rumpf. Gedacht, um die **eigenen** Schnittstellen zu entwickeln
und zu prüfen. Weil der Browser selbst abruft (`src/inspect.rs`) und nicht die
Seite der App, gelten die Grenzen fremder Herkünfte (CORS) nicht – man sieht
jede Antwort unverändert.

- **Methoden:** GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS. Kopfzeilen frei,
  bis auf die, die der Browser selbst setzt (`Host`, `Content-Length`,
  `Connection` …) – so lässt sich nichts einschleusen oder verfälschen. Ein
  JSON-Rumpf lässt sich mit einem Klick einrücken.
- **Weitergeleitet wird nicht:** Eine 3xx-Antwort zeigt die App mit ihrem
  `Location`, statt ihr zu folgen – beim Prüfen will man genau das sehen, was
  der Server geantwortet hat.
- **Grenzen:** Die Anfrage geht über den Kanal der Seiten (höchstens ~4 KB je
  Nachricht), darum ist der Rumpf klein gehalten; die Antwort darf größer sein
  (bis 512 KB werden gezeigt). Im Stealth-Fenster schickt die App nichts (die
  Anfrage ginge am Tunnel vorbei). Zuletzt benutzte Anfragen merkt sie sich –
  nur Methode und Adresse, keine Kopfzeilen und keinen Rumpf, damit keine
  Zugangsdaten liegen bleiben; im privaten Fenster gar nichts.

Geprüft im Probelauf gegen einen eigenen Server: POST mit eigener Kopfzeile und
Rumpf (vom Server als JSON zurückgespiegelt), eine Weiterleitung (nicht
gefolgt, `Location` im Kopf), ein 404, die Fehlerfälle (leere/unerreichbare
Adresse, zu große Anfrage), die Merkliste, privates und Stealth-Fenster.

## Passwörter

![Vorschlag am Anmeldefeld](docs/zunder-passwoerter.png)

Nach einer Anmeldung fragt Zunder, ob es das Passwort sichern soll. Kommt man
wieder, zeigt sich am Anmeldefeld, was für diese Website gesichert ist: ein
Klick, Touch ID, ausgefüllt. In Feldern für ein neues Passwort schlägt Zunder
eines vor (18 zufällige Zeichen in drei Gruppen) und sichert es gleich mit.
Der Schlüssel in der Adressleiste zeigt dasselbe, ⇧⌘K füllt die zuletzt
benutzte Anmeldung aus, und „Passwörter“ im Menü öffnet die Verwaltung:
suchen, anzeigen, kopieren, ändern, löschen, importieren, exportieren.

![Die Verwaltung der Passwörter](docs/zunder-passwortliste.png)

Wie das abgesichert ist:

- **Verschlüsselt abgelegt.** Website und Benutzername stehen lesbar in der
  Datenbank, das Passwort nur verschlüsselt (ChaCha20-Poly1305) – gebunden an
  genau diese Website und diesen Namen. Der Schlüssel liegt im Schlüsselbund
  von macOS und wird erst geholt, wenn zum ersten Mal ein Passwort gebraucht
  wird.
- **Nur auf einen Klick.** Von selbst füllt Zunder nie etwas aus. Ein Skript
  einer Seite kommt so nicht an ein Passwort, solange niemand klickt. Auch die
  Vorschläge erscheinen erst, wenn jemand selbst in ein Feld geklickt oder
  eine Taste gedrückt hat – eine Seite kann sie nicht heraufbeschwören.
- **Nur auf derselben Website.** Ein Passwort geht nur dorthin, wofür es
  gesichert wurde (derselbe Rechnername, mit oder ohne „www.“, derselbe Port),
  und ein verschlüsselt gesichertes nie auf eine unverschlüsselte Seite. Die
  Entscheidung fällt im Dokument selbst, unmittelbar vor dem Einsetzen, an
  seiner wirklichen Herkunft; der Browser prüft vorher zusätzlich gegen die
  Adresse des Tabs. Das Passwort reist dabei als Argument in die Seite, nie
  als Text eines Skripts – so kann es auch in keiner Fehlermeldung landen.
- **Eingebettete Rahmen bekommen nichts.** Es zählt nur die Seite selbst, nicht
  was fremde Anbieter in ihr einblenden.
- **Die Seite redet nicht mit.** Erkennen und Ausfüllen erledigt ein Skript in
  einer eigenen, von der Seite abgeschotteten Welt. Die Skripte der Seite sehen
  weder seine Funktionen noch seinen Kanal zum Browser – sie können ihm keine
  Anmeldungen unterschieben und nicht abfragen, welche Konten gesichert sind.
- **Touch ID** (oder das Passwort des Macs) vor dem Ausfüllen. Das lässt sich
  abschalten – aber nur mit Touch ID, und vermerkt wird es beim Schlüssel im
  Schlüsselbund, nicht in einer Datei, die sich umschreiben ließe. In der
  Verwaltung braucht es die Abfrage vor dem Anzeigen, Kopieren und Ändern,
  vor jedem Exportieren eigens. Eine Freigabe zum Ausfüllen öffnet die
  Verwaltung nicht und umgekehrt.
- Ein kopiertes Passwort ist für Zwischenablage-Verwalter als geheim markiert
  und nach 90 Sekunden wieder aus der Zwischenablage verschwunden.
- **Private Fenster** füllen aus, sichern aber nichts.

## Lesemodus (⌥⌘R)

![Ein Artikel im Lesemodus](docs/zunder-lesemodus.png)

Wo eine Seite genug Fließtext hat, zeigt die Adressleiste den Knopf dafür. Der
Lesemodus sucht den Artikel, lässt Menüs, Werbung, Randspalten und Kommentare
weg und setzt den Text in ruhiger Schrift neu – mit Überschrift, Bildern,
Listen, Tabellen und Links. Schriftgröße über − und +, hell oder dunkel nach
System; Esc oder ein zweiter Klick auf den Knopf führt zurück zur Seite.

## Seiten übersetzen (⌥⌘T)

![Eine übersetzte Seite mit dem Hinweis „Original zeigen“](docs/zunder-uebersetzen.png)

„Seite übersetzen“ (Menü, ⌥⌘T oder der Knopf in der Adresszeile, der
erscheint, wenn eine Seite eine andere Sprache angibt) übersetzt die Seite, wie
sie dasteht: Überschriften, Absätze, Menüs und Knöpfe wechseln die Sprache,
Aufbau, Bilder und Links bleiben. Derselbe Befehl – oder „Original zeigen“ im
Hinweis – holt das Original zurück.

- **Womit:** mit dem Modell, das im Assistenten gewählt ist – Apples Modell
  oder Ollama auf dem Mac, oder Claude bzw. ein anderer Dienst. Der Hinweis
  nennt das Modell jedes Mal. Aus privaten und Stealth-Fenstern übersetzt nur
  ein Modell auf dem Mac; der Text einer Seite geht von dort nie ins Netz.
- **Wohin:** in die Sprache der Oberfläche (Deutsch oder Englisch). Steht die
  Seite schon in ihr, wird in die jeweils andere übersetzt.
- **Wie:** Zunders Skript sammelt die sichtbaren Textstellen. Was nur durch
  einen Link oder Fettdruck getrennt ist, geht als ein Satz an das Modell und
  kommt in denselben Stücken zurück – so stimmt der Satzbau, und jeder Link
  behält seinen Text. Eingesetzt wird ausschließlich Text in vorhandene
  Textstellen, nie HTML: Der Aufbau der Seite bleibt, wie er war, und was das
  Modell auch antwortet, kann in der Seite nichts auslösen. Der Seitentext
  gilt dem Modell als Material – „ignoriere alle Anweisungen“ wird übersetzt,
  nicht befolgt.
- **Was bleibt:** Code, Eingabefelder, Stellen, die die Seite selbst als
  „nicht übersetzen“ markiert, und alles Unsichtbare.
- **Wie lange:** Die Seite füllt sich Teil für Teil – der eigentliche Inhalt
  zuerst, Menüs und Fußzeile danach. Auf dem Mac läuft eine Anfrage nach der
  anderen (bei Apples Modell rund vier Sekunden je Teil; ein langer
  Wikipedia-Artikel brauchte zwei Minuten), bei einem Dienst im Netz drei
  nebeneinander. Sehr lange Seiten werden nur am Anfang übersetzt – der
  Hinweis sagt es dann.

Grenzen: Die Güte ist die des Modells – Apples kleines Modell übersetzt
verständlich, aber holprig, und lässt Einzelnes stehen („A Leuchtturm ist ein
Turm“); ein größeres Modell über Ollama übersetzte im Vergleich merklich
besser. Gegen Claude und andere Dienste im Netz ist das Übersetzen nur mit
einer Attrappe auf dem Mac geprüft, nicht gegen die echten Dienste. Übersetzt wird
nur die Seite selbst, nicht eingebettete Rahmen anderer Websites; Text, den
eine Seite nach dem Übersetzen nachlädt, bleibt im Original, und der Titel des
Tabs auch.

## Tabs ordnen

![Angeheftete Tabs und zwei Tab-Gruppen](docs/zunder-tab-gruppen.png)

Ein Rechtsklick auf einen Tab bietet beides an:

- **Tab anheften**: Der Tab rückt ganz nach links und zeigt nur noch sein
  Icon. Angeheftete Tabs kommen nach jedem Start wieder – auch wenn „Beim
  Start letzte Sitzung wiederherstellen“ abgeschaltet ist –, und „Andere Tabs
  schließen“ lässt sie stehen.
- **In neue Gruppe** bzw. **Zur Gruppe …**: Die Tabs einer Gruppe stehen
  nebeneinander hinter einem farbigen Schild mit ihrem Namen. Ein Klick auf
  das Schild klappt die Gruppe zu oder auf; ein Rechtsklick benennt sie um,
  gibt ihr eine von acht Farben, löst sie auf oder schließt sie samt Tabs. Ein
  Tab, der aus einer Gruppe heraus geöffnet wird, gehört zur selben Gruppe.
  Gruppen überstehen den Neustart.

## Gemessen

Auf einem MacBook Pro (Apple Silicon, macOS 26) mit lokalen Testseiten:

| | |
| --- | --- |
| Größe der App | 15 MB – davon 10 MB die sechs Hintergründe |
| Fenster steht | nach etwa 0,15 s |
| erste Seite geladen | nach etwa 0,4 s |
| Last im Leerlauf | 0 % CPU, kein einziges Aufwachen |
| Zusammenfassung einer Seite, Apples Modell | 2–4 s |
| Vergleich zweier Seiten, Apples Modell | 4–5 s |
| lange Seite (64 000 Zeichen), Apples Modell | etwa 20 s für acht Teile |

## Was fehlt

- **Erweiterungen.** WebKit bringt seit macOS 15.4 eine Schnittstelle für
  Web-Erweiterungen mit; sie anzubinden (Verwaltung, Rechte je Erweiterung,
  Symbolleiste, Speicher) ist ein eigenes großes Vorhaben.
- **Synchronisierung zwischen Geräten** – dafür bräuchte es Konten und einen
  Dienst, der Daten Ende-zu-Ende verschlüsselt aufbewahrt. **Profile**
  (getrennte Sätze von Verlauf, Lesezeichen und Anmeldungen) gibt es ebenfalls
  nicht; private und Stealth-Fenster trennen nur für die Dauer des Fensters.
- **Automatische Updates.** Eine neue Fassung entsteht durch Bauen aus dem
  Quelltext (`scripts/bundle.sh --install`). Für Updates von selbst bräuchte
  es eine bei Apple beglaubigte Signatur und eine Stelle, die Fassungen
  ausliefert.
- **Passkeys**: WebKit gibt sie eingebetteten Browsern nur mit einer
  Berechtigung frei, die Apple auf Antrag an Browser-Hersteller vergibt. In
  Zunder meldet WebKit deshalb „nicht verfügbar“; Seiten bieten dann den Weg
  über das Passwort an.
- Automatisches Ausfüllen von Adressen und Zahlungsdaten.
- Der Passwort-Tresor füllt keine Anmeldeformulare aus, die in einem
  eingebetteten Rahmen einer anderen Website stecken, warnt nicht vor
  Passwörtern aus bekannten Datenlecks und erkennt nicht jede Anmeldung: Nach
  dem Sichern fragt er, wenn die Seite nach dem Abschicken wechselt oder das
  Passwortfeld verschwindet – auch dann, wenn die Anmeldung abgelehnt wurde.
- Der Lesemodus findet den Artikel über einfache Regeln (wo steht der meiste
  Fließtext?). Auf Seiten, die ihren Text ungewöhnlich verpacken, fehlt
  mitunter etwas oder es bleibt Beiwerk stehen.
- Mitteilungen von Webseiten und Standortabfrage (bietet WebKit eingebetteten
  Browsern nicht bzw. nur mit Zusatzaufwand an). Den Standort bekommen nur
  eingebaute Apps, nach Zustimmung – Webseiten nicht.
- **Wetter-App:** Das Regenradar gibt es nur für Deutschland und einen
  Streifen der Nachbarländer. Das Satellitenbild dahinter löst etwa 500 Meter
  auf und wirkt im kleinsten Ausschnitt weich. Städte stehen nur die größeren
  da, Grenzen und Flüsse keine. Die echte Standortabfrage bei macOS ist nicht
  im Probelauf geprüft. Vor einer Veröffentlichung sind die Bedingungen der
  Dienste zu klären: Open-Meteo ist ohne Vertrag nur für nicht kommerzielle
  Nutzung frei, und Nominatim verträgt nur wenige Anfragen aller Nutzer
  zusammen.
- **Zeiterfassung:** Die Zeiten liegen nur auf diesem einen Mac – kein
  Abgleich mit anderen Geräten, keine Sicherung außer der Tabelle. Ein Eintrag
  lässt sich nicht auf einen anderen Tag verschieben (löschen und neu buchen),
  und Stundensätze oder Rechnungen kennt sie nicht. Pausen entstehen über
  „Pause“ an der Uhr oder beim Weiterführen – von Hand nachtragen lassen sie
  sich nicht, ein Eintrag kennt höchstens zwölf, und weiterführen lässt sich
  nur ein Eintrag von heute. Am Anfang und am
  Ende der Zeile liest sie Zahlen als Zeit: Aus „3-4 Leute anrufen“ wird eine
  Buchung von 3 bis 4 Uhr – die Vorschau zeigt es vor dem Buchen.
- Es gibt vier eingebaute Apps; eine Schnittstelle für Apps von außen gibt
  es nicht.
- **Request-Inspektor:** Der Rumpf einer Anfrage ist klein (der Kanal der
  Seiten fasst höchstens ~4 KB je Nachricht); für große Hochladungen ist die
  App nicht gedacht. Weitergeleitet wird nicht (bewusst). Prüfe nur
  Schnittstellen, die dir gehören oder für die du die Erlaubnis hast.
- **Sicherheitscheck:** Er prüft die Schutzschichten von außen, nicht das
  Programm hinter der Website – eingeschleust, durchprobiert oder belastet wird
  nichts. Die Fassung der TLS-Verbindung liest er nicht (ob also noch TLS 1.0/1.1
  erlaubt ist, sagt er nicht). Im eigenen Netz entfallen die Prüfungen fürs
  offene Netz. Prüfe nur Seiten, die dir gehören oder für die du die Erlaubnis
  hast.
- **Dateinamen aus dem Link:** Erzeugt eine Website eine Datei selbst und
  gibt ihr über den Link einen Namen (`download="bericht.csv"` an einer
  `blob:`-Adresse), heißt sie in der abgeschotteten App trotzdem „Unknown“
  mit passender Endung: Dort stößt Zunder den Download selbst an, und WebKit
  nennt den Namen aus dem Link dabei nicht. Für die eingebauten Apps ist das
  gelöst (sie kündigen den Namen an), für Websites nicht.
- Der Blocker setzt die Filterlisten um, schleust aber keine Skripte in
  Seiten ein. Werbepausen in Videos und Websites, die Besucher mit Blocker
  aussperren, bleiben deshalb, wie sie sind.
- Stealth-Fenster: Tor bringt Zunder nicht selbst mit – es nutzt einen
  Tor-Zugang, der auf dem Mac läuft. Ohne ihn bleibt die IP-Adresse sichtbar.
- Der Assistent kann die Anmeldungen von Claude Code oder der Claude-App
  nicht mitbenutzen (die Sandbox lässt ihn nicht an deren Daten), und er
  spricht mit anderen Diensten nur über die verbreitete Chat-Schnittstelle.
- **Betrugsseiten**: Die Listen kennen nur Gemeldetes und gelten erst, wenn
  sie nach dem ersten Start geholt sind. WebKits eigener Schalter für
  „betrügerische Websites“ ist zusätzlich an, ließ sich mit der offiziellen
  Testseite in Zunder aber nicht auslösen – zugesagt wird nur, was Zunder
  selbst prüft.
- **Ollama**: Ist Ollama als Modell gewählt, spricht Zunder mit dem, was auf
  diesem Mac an Ollamas Anschluss antwortet. Läuft Ollama dann nicht, könnte
  ein anderes Programm des Macs dort lauschen und bekäme den Seitentext einer
  Anfrage. Ohne diese Wahl spricht Zunder den Anschluss nur an, um zu sehen,
  ob Ollama läuft und welche Modelle es anbietet.
- **Rechte je Website** kennt Kamera, Mikrofon, das eigene Netz und den
  abgeschalteten Werbeblocker – andere Rechte (etwa die Bildschirmfreigabe)
  verwaltet die Übersicht nicht. Den Tab, der gerade aufnimmt, markiert Zunder
  noch nicht; macOS zeigt es in der Menüleiste.
- **Folgen der Sandbox**: Eine eingetippte Dateiadresse (`file://…`) öffnet
  Zunder nicht mehr – Dateien kommen über „Öffnen mit → Zunder“ herein, und
  dann nur die eine Datei, nicht ihre Nachbarn. Bei Links mit vorgegebenem
  Dateinamen (`download="…"`) heißt die Datei so, wie der Server sie nennt.
  „Aus Chrome übernehmen“ liest nur das Standardprofil.
- **Eingebettete PDFs** zeigt Zunder mit dem Betrachter von WebKit – als
  Rahmen, als `embed` oder `object`, auch aus `blob:`-Adressen; dem Fall, in
  dem WebKit den eigenen Rahmen einer Website leer lässt, hilft Zunder ab
  (siehe [Eingebettete PDFs](#eingebettete-pdfs)). Leer bleibt eine Vorschau
  weiterhin, wenn die Datei größer als 64 MB ist und wenn der Rahmen
  abgeschottet ist (`sandbox`); liefert der Server das PDF als Anhang aus
  (`Content-Disposition: attachment`), lädt Zunder es herunter.
- **PDFs mit Härtung und im Stealth-Fenster:** WebKit zeigt sie dort nicht
  an. Ein PDF, das man öffnet, wird heruntergeladen. Eines, das eine Seite
  einbettet, bleibt ein leerer Rahmen – auch wenn sie es auf einen Klick hin
  einbettet. Ansehen lässt es sich dann nur über einen Link der Seite oder
  ohne Härtung
  (siehe [Mit Härtung und im Stealth-Fenster](#mit-härtung-und-im-stealth-fenster)).
- **Downloads ohne Klick:** Weist ein Server eine Datei ausdrücklich als
  Download aus, lädt Zunder sie auch dann, wenn niemand geklickt hat – in
  jedem Modus, auch aus einem eingebetteten Rahmen. Begrenzt ist das auf fünf
  je Tab in zehn Sekunden; eine Rückfrage je Website gibt es nicht. Ebenso
  kann eine Seite mit Härtung den Tab selbst zu einem PDF schicken, das dann
  heruntergeladen wird.
- Lesezeichen kennen eine Ebene von Ordnern, keine Ordner in Ordnern.
- Die Vorschaubilder der Tab-Übersicht entstehen, während ein Tab vorn ist; ein
  Tab, der nie vorn war, zeigt sein Icon.
- Der Assistent liest nur Text: keine PDFs, keine Bilder, nichts aus
  eingebetteten Rahmen fremder Herkunft. Er kann im Browser nichts auslösen
  und nichts im Netz nachschlagen.
- Der Weg zu Claude ist gegen einen lokalen Nachbau des Dienstes geprüft
  (Anfrage, Antwortstrom, Fehlerfälle), nicht gegen den echten Dienst.
  Dasselbe gilt für „Anderer Dienst“ und die Anmeldung bei OpenRouter: geprüft
  gegen Nachbauten auf dem Mac, nicht gegen OpenAI, OpenRouter oder LM Studio.
- Einträge im Schlüsselbund (der API-Schlüssel, der Schlüssel des
  Passwort-Tresors, und von WebKit angelegte Schlüssel wie der WebCrypto-Schlüssel
  einer Seite) hängen an der Signatur der App. Wird mit einem **gleichbleibenden**
  Zertifikat signiert (`bundle.sh` nimmt automatisch „Zunder Local“ oder eine
  „Apple Development“-Identität, sonst `ZUNDER_SIGN_ID=<Name>`), behält macOS
  „Immer erlauben“ über Neubauten hinweg. Nur **ad hoc** signiert (kein
  Zertifikat vorhanden) ändert sich die Signatur bei jedem Neubau, und macOS
  fragt dann nach jedem Einspielen erneut.
- Wer am entsperrten Mac sitzt, kann ohne Touch ID Passwörter löschen und
  neue hinzufügen, aber keine lesen. Nach dem Ausfüllen steht ein Passwort im
  Feld der Seite – dort lässt es sich, wie in jedem Browser, sichtbar machen.
  Ein anderes Programm desselben Benutzers kann den Schlüssel im Schlüsselbund
  löschen (dann sind die Passwörter verloren), ohne Rückfrage von macOS aber
  nicht lesen.
- Tastenkürzel des Browsers haben Vorrang vor denen einer Seite.
- Gebaut für Apple Silicon. Für Intel-Macs müsste zusätzlich für
  `x86_64-apple-darwin` gebaut werden.

## Bauen und starten

Voraussetzung: Rust (stable) und die Xcode-Kommandozeilenwerkzeuge (für den
Swift-Helfer des Assistenten braucht es das SDK von macOS 26).

```bash
scripts/bundle.sh
```

Das Ergebnis liegt in `dist/Zunder.app`. Mit `scripts/bundle.sh --install`
landet die App zusätzlich in `/Applications` (Programme).

Die App ist nur lokal signiert. Das genügt für den eigenen Mac; zum
Weitergeben braucht es eine Apple-Entwickler-ID und Notarisierung.

Signiert wird mit „Hardened Runtime“ und für die Sandbox von macOS
(`assets/Zunder.entitlements`). `ZUNDER_SANDBOX=0 scripts/bundle.sh` baut ohne
Sandbox – dann liegt das Profil wieder unter
`~/Library/Application Support/Zunder`.

Lässt sich der Helfer nicht bauen (älteres Xcode), entsteht die App trotzdem;
der Assistent bietet dann Ollama und Claude an.

## Aufbau

| Datei | Aufgabe |
| --- | --- |
| `src/app.rs` | Zustand, Ereignisschleife, Fenster, Tabs, Nebeneinander, Sitzung |
| `src/commands.rs` | Befehle aus Oberfläche und Menüleiste, Einstellungen |
| `src/pages.rs` | Ereignisse aus den Seiten: Laden, Titel, Pop-ups, Downloads, Fehler |
| `src/webviews.rs` | Erzeugt die WebViews (Rahmen, Assistent, eine je Tab) |
| `src/native.rs` | Direkter Draht zu AppKit/WebKit für alles, was wry nicht anbietet |
| `src/omnibox.rs` | Adressleiste: Adresse oder Suche, Suchmaschinen, Vorschläge |
| `src/shield.rs` | Schutz beim Navigieren: nur verschlüsselt, Tracking-Anhängsel |
| `src/blocker.rs` | Werbeblocker, Schutz vor gemeldeten Betrugsseiten und des eigenen Netzes: Regellisten für WebKit, Auffrischen, Schalter je Website |
| `src/filters.rs` | Übersetzer für Filterlisten (EasyList-Schreibweise → WebKits Regelformat) |
| `src/stealth.rs` | Stealth-Fenster: Tor-Zugang auf dem Mac finden |
| `src/vault.rs` | Passwort-Tresor: Verschlüsselung, Schlüssel, Zuordnung zu Websites, CSV |
| `src/logins.rs` | Passwörter im Browser: Sichern, Ausfüllen, Sprechblase, Verwaltung |
| `src/reader.rs` | Lesemodus |
| `src/translate.rs` | Seiten übersetzen: Sätze bündeln, Portionen ans Modell, Antworten einsetzen |
| `src/sitecheck.rs` | Seitencheck hinter der Security-Ampel: Auswertung von Antwortkopf, Aufbau und Cookies |
| `src/media.rs` | Kamera und Mikrofon je Website: wann gefragt, erlaubt oder abgelehnt wird (die Frage selbst stellt `src/native.rs`) |
| `src/apps.rs` | Eingebaute Apps: welche es gibt, was jede darf (Netz, Standort, Speicher, Dateien sichern, Abrufen), ihre Adressen und Anfragen |
| `src/security.rs` | Sicherheitscheck: eine Adresse abrufen, den Quelltext lesen und – mit `sitecheck` – bewerten; dazu ein paar gefahrlose aktive Prüfungen |
| `src/inspect.rs` | Request-Inspektor: eine frei zusammengestellte HTTP-Anfrage schicken und die rohe Antwort zurückgeben |
| `src/geo.rs` | Standort von macOS für eingebaute Apps – nie für Webseiten |
| `src/pdfs.rs` | Eingebettete PDFs, die WebKit leer lässt: was der Browser annimmt, aufbewahrt und wieder herausgibt – und wann er einen Download absagt, den WebKit für ein eingebettetes PDF von sich aus beginnt (die Stellen selbst stehen in `src/native.rs`) |
| `src/bookmarks_io.rs` | Lesezeichen aus anderen Browsern übernehmen und sichern |
| `src/ai.rs` | Die drei Modelle hinter einer Schnittstelle; Zerlegen langer Seiten, Vergleich |
| `src/assistant.rs` | Seitenleiste des Assistenten: Seitentext holen, Modellwahl, Schlüssel |
| `src/wallpapers.rs` | Liefert die Live-Hintergründe an die Startseite |
| `src/store.rs` | Verlauf, Lesezeichen, Downloads, Einstellungen (SQLite) |
| `src/menu.rs` | Menüleiste |
| `src/assets.rs` | Liefert Oberfläche und interne Seiten über `zunder://` aus |
| `src/i18n.rs` | Texte, deutsch und englisch |
| `src/dev.rs` | Prüfhilfe – nur in Probe-Apps mit dem Bau-Merkmal `harness` |
| `helper/zunder-ai.swift` | Brücke zu Apples Sprachmodell (eigenes kleines Programm in der App) |
| `ui/` | Oberfläche (HTML, CSS, JavaScript), Assistent, Sprechblase, Startseite, Fehlerseite |
| `ui/logins.js`, `ui/reader.js`, `ui/translate.js`, `ui/check.js`, `ui/pdfs.js` | Zunders eigene Skripte in den Seiten (abgeschottete Welt) |
| `ui/apps.js`, `ui/apps.css` | Gemeinsamer Teil der eingebauten Apps: der Draht zum Browser |
| `ui/app-weather.*` | Die App „Wetter und Regenradar“ samt der Städteliste für das Radar |
| `ui/app-time.*` | Die App „Zeiterfassung“ |
| `ui/app-security.*` | Die App „Sicherheitscheck“ |
| `ui/app-request.*` | Die App „Request-Inspektor“ |

Ein Fenster besteht aus einer WebView für den Rahmen (Tab-Leiste,
Adressleiste, Seitenleisten), einer WebView je Tab und – sobald sie zum ersten
Mal gebraucht werden – je einer für den Assistenten und für die Sprechblase
des Passwort-Tresors. Jede Seite läuft damit in eigenen WebKit-Prozessen,
getrennt vom Browser selbst.

Für Menüs, Vorschläge und Seitenleisten legt sich der durchsichtige Rahmen
kurz über das ganze Fenster; Suchleiste und Hinweise schieben die Seite
stattdessen nach unten, damit sie bedienbar bleibt. Der Assistent sitzt neben
der Seite und macht sie schmaler, statt sie zu verdecken. Die Sprechblase des
Passwort-Tresors ist eine eigene kleine Ansicht über der Seite: Sie verdeckt
nur ihre eigene Fläche, daneben bleibt die Seite bedienbar.

`wry` kennt nur „Seite übernommen“ und „Seite fertig“. Was ein Browser darüber
hinaus braucht – Ladebeginn, Weiterleitung, Ladefehler, Dialoge der Seite,
Anmeldeabfrage, ⌘-Klick, den Schutz beim Navigieren – rüstet `native.rs` zur
Laufzeit an den Delegate-Klassen von wry nach. Das ist die Stelle, die bei
einem Update von wry zuerst zu prüfen ist; in einer Probe-App zeigt der Befehl
`hooks`, was nachgerüstet wurde.

## Wie Zunder sich selbst schützt

Der Code wird in Runden von unabhängiger Seite durchgesehen – zuletzt am
22.11.2026 in drei getrennten Durchsichten (Listen und Stealth, KI und
Übersetzen, Oberfläche und Befehle). Was dabei herauskam, ist behoben und,
wo es sich nachstellen ließ, mit einem Versuch vorher und nachher belegt:

- **„Im Verlauf“ zählt nur für echte Verlaufseinträge.** WebKit meldet auch
  die Weiterleitung, mit der ein Server auf ein Neuladen antwortet, als
  „Neuladen“. Eine Seite, die sich selbst neu lädt, kam so an der Abschirmung
  des eigenen Netzes vorbei und konnte Zunders Warnseite mit erfundenem Inhalt
  laden. Jetzt prüft Zunder, ob das Ziel wirklich der Eintrag im Verlauf ist;
  die eigenen Warnseiten tragen zusätzlich ein Siegel, das je Start neu
  ausgewürfelt wird.
- **`beispiel.de.` ist `beispiel.de`.** Der Punkt, der hinter jedem
  Rechnernamen stehen darf, führte an den Listen vorbei – auch an der für das
  eigene Netz (`localhost.`). Alle Muster nehmen ihn jetzt hin.
- **Listen folgen der Seite, die ein Tab zeigt** – nicht einer Adresse, zu
  der er nur aufgebrochen ist. Sonst würde eine Seite den Werbeblocker los,
  indem sie zum Schein zu einer Website aufbricht, für die er abgeschaltet
  ist. Auf Zunders eigenen Seiten gelten die Listen gar nicht: Keine Regel
  kann dort den Text einer Warnung ausblenden.
- **Hinweise gehören zu der Seite, die sie ausgelöst hat.** Sie nennen die
  Website, erscheinen nur über ihrem Tab und verschwinden, wenn er die Website
  wechselt. Eine Warnung räumt ihren Platz nicht für einen gewöhnlichen
  Hinweis.
- **Ein Schlüssel gehört genau einem Dienst.** Er liegt im Schlüsselbund unter
  dessen Adresse; eine Anfrage an einen Dienst kann nur dessen eigenen
  Schlüssel holen. Wechselt Modell, Dienst oder Schlüssel, endet jede laufende
  Übersetzung.
- **Eine Übersetzung schreibt nur in die Seite, die sie gelesen hat** – nie in
  eine andere, die inzwischen im Tab steht. Verlässt eine Seite den Tab, kommt
  ihr Original zurück.
- **Keine Link-Vorschau** beim kräftigen Klick: Sie lüde die Adresse an den
  Prüfungen, den Listen und – im Stealth-Fenster – am Tunnel vorbei.
- **Manipulierte Filterlisten** können sich nicht mehr mit einem Fantasie-Datum
  für immer als „die neueste“ festsetzen; nimmt WebKit einen Teil eines
  Listensatzes nicht an, bleibt es beim vorigen.
- **Privat bleibt privat:** Eine Erlaubnis fürs eigene Netz aus einem privaten
  oder Stealth-Fenster gilt nur bis zum Beenden. „Neu laden“ auf der Warnung
  vor dem eigenen Netz lädt die Warnung neu, nicht das Ziel.

- Die ausgelieferte App lässt sich nicht von außen steuern: Die Prüfhilfe
  (Befehle über die Standardeingabe, Umgebungsvariablen zum Prüfen) steckt nur
  in eigens gebauten Probe-Apps – siehe „Entwicklung“.
- Befehle nimmt der Browser nur von seiner eigenen Oberfläche an
  (`zunder://ui/`: Rahmen, Assistent und Sprechblase) – und von jeder Fläche
  nur, was zu ihrer Aufgabe gehört: Die Seitenleiste des Assistenten kommt
  nicht an Einstellungen oder Passwörter, die Sprechblase nur an ihre eigenen
  Antworten. Webseiten und auch die internen Seiten in einem Tab haben keinen
  Befehlskanal.
- Interne Seiten (`zunder://…`) lädt ein Tab nur, wenn der Browser selbst das
  will: Jede solche Navigation ist vorher angemeldet und gilt einmal. Eine
  Website kann deshalb weder die Fehlerseite mit eigenem Text und fremder
  Adresse aufrufen noch die Startseite oder die Oberfläche einbetten. Jede
  WebView bekommt außerdem nur ausgeliefert, was zu ihr gehört – ein Tab
  nichts von der Oberfläche, die Oberfläche keine Seiten für Tabs.
- Hält der Browser eine Navigation an, um sie zu verbessern (verschlüsselt
  statt unverschlüsselt, ohne Tracking-Anhängsel), geht die Seite danach
  selbst noch einmal los. So bleibt es ihre Navigation: Die Adressleiste zeigt
  bis zur Übernahme die alte Adresse, und Cookies, die nur bei eigenen Aufrufen
  mitgehen sollen, bleiben zu Hause.
- Was eine Seite dem Browser schickt, nimmt Zunder selbst entgegen und liest es
  Zeichen für Zeichen – ein kaputter Text (ein halbes Zeichenpaar) brachte die
  dafür übliche Bibliothek zum Absturz. Auch die Suche in der Seite und das
  Auslesen für den Assistenten laufen in Zunders abgeschotteter Welt und
  liefern nur Text zurück.
- Zunders eigene Skripte in einer Seite (Passwort-Tresor, Lesemodus) laufen in
  einer abgeschotteten Welt: Sie sehen dasselbe Dokument wie die Seite, aber
  die Skripte der Seite sehen weder ihre Funktionen noch ihren Kanal zum
  Browser. Zu jeder Meldung nennt WebKit selbst Rahmen und Herkunft; gewertet
  wird nur, was aus dem Hauptrahmen der Seite kommt, die der Tab gerade zeigt.
- Die Adressleiste zeigt eine Adresse, die noch lädt, nur dann, wenn der
  Browser selbst dorthin geschickt hat (Eingabe, Lesezeichen, neuer Tab).
  Schickt eine Seite sich woandershin, bleibt bis zur Übernahme die bisherige
  Adresse stehen – sonst stünde über ihrem Inhalt schon die fremde.
- Der Lesemodus baut den Artikel aus neu erzeugten Elementen im Dokument der
  Seite selbst nach – Text, Bilder und Links, nie HTML-Text der Seite, keine
  Skripte, keine Formulare. Eine eigene Seite unter Zunders Namen, die fremde
  Inhalte zeigt, entsteht dabei nicht.
- Die Sprechblase am Anmeldefeld erscheint dort, wo die Seite das Feld meldet.
  Das ist nur ein Hinweis: Die Blase nimmt Klicks erst einen Augenblick nach
  dem Erscheinen an, und ausgefüllt wird ohnehin nur auf der Website, für die
  ein Passwort gesichert wurde.
- Wenige eng begrenzte Meldungen dürfen Seiten schicken, und der Browser prüft
  jede gegen WebKits Angabe, woher sie kommt (nur der Hauptrahmen zählt): Die
  Startseite darf einen der eingebauten Hintergründe wählen, die
  Rückfrage-Seiten dürfen die Zustimmung melden („unverschlüsselt öffnen“,
  „trotz ähnlicher Adresse öffnen“, „dieser Website das eigene Netz erlauben“),
  und jede Seite darf sagen, dass sie die Tastatur bekommen hat (wirkt nur
  zwischen zwei nebeneinander gezeigten Tabs).
- Was von Webseiten kommt (Titel, Adressen), wird nur als Text eingesetzt,
  gekürzt und ohne Steuerzeichen. Ein Besuch zählt je Website höchstens alle
  30 Sekunden – eine Seite, die sich in Schleife neu lädt oder nur ihre Adresse
  ändert, schiebt sich weder in Vorschlägen noch auf der Startseite nach vorn.
- Der Assistent behandelt Seitentext als Material, nicht als Auftrag: Er steht
  in einer eigenen Klammer, die der Text selbst nicht schließen kann, und die
  Anweisung an das Modell sagt das ausdrücklich. Antworten werden als Text
  dargestellt – ohne Links, Bilder oder HTML – und der Assistent hat keine
  Werkzeuge. Was eine Seite dem Modell unterschiebt, kann deshalb nichts
  auslösen und nichts nachladen. Titel und Adresse einer Seite nimmt der
  Browser aus dem Tab, nicht aus der Seite.
- Andere Programme (`mailto:`, `tel:` …) öffnet eine Seite nie ohne Rückfrage;
  eingebettete Rahmen und Tabs im Hintergrund dürfen gar nicht erst fragen.
- Der Schlüssel für Claude geht an genau eine Adresse – keiner Umleitung nach,
  nie unverschlüsselt. Über Ollama nutzt der Assistent nur Modelle, die auf
  diesem Mac laufen; solche, die Ollama in der Cloud rechnen lässt, bietet er
  nicht an. Eine Frage gilt genau der Seite, zu der sie gestellt wurde: Hat der
  Tab inzwischen gewechselt, geht nichts an ein Modell.
- Private Fenster nutzen einen flüchtigen Datenspeicher; Verlauf, Sitzung,
  Suchvorschläge und Modelle im Netz bleiben dort aus.
- Suchvorschläge senden die Eingabe an die gewählte Suchmaschine (abschaltbar
  in den Einstellungen). Adressen werden dabei nie gesendet.

## Icons und Hintergründe

Alle Icons wurden über Higgsfield generiert (Modell `gpt_image_2_6`): das
App-Icon als Bild, die 71 Icons der Oberfläche und der eingebauten Apps als SVG. Die unveränderten
Ergebnisse liegen in `assets/higgsfield-raw/`, die Auftragsnummern in
`assets/icon-jobs.json`.

![Die Icons der Oberfläche](docs/icons-uebersicht.png)

```bash
python3 -I scripts/build_icons.py /tmp/zunder-icons
```

bereitet sie auf: Die SVGs werden auf eine feste Liste von Formen reduziert,
auf einheitliche Größe und Strichstärke gebracht und zu `ui/icons.svg`
zusammengefasst; aus dem Motiv entsteht `assets/Zunder.icns`.

Auch die Live-Hintergründe stammen von Higgsfield: das Standbild von
`gpt_image_2_6`, die Bewegung von `wan3_0` – mit dem Standbild als erstem und
letztem Bild, damit sich die Szene wiederholen lässt.

![Die sechs Hintergründe auf der Startseite](docs/zunder-hintergruende.jpg)

```bash
scripts/build_wallpapers.sh
```

macht aus den Rohfassungen (je 12–23 MB) nahtlose Schleifen von 1–2,5 MB samt
Standbild und Vorschau in `assets/wallpapers/`. Das braucht `ffmpeg`; die App
selbst braucht es nicht.

## Entwicklung

Geprüft wird an einer **Probe-App mit Prüfhilfe**. Sie entsteht nur auf
ausdrücklichen Wunsch und nur mit eigener Kennung:

```bash
ZUNDER_HARNESS=1 ZUNDER_BUNDLE_ID=app.zunder.browser.test ZUNDER_DIST=/tmp/zunder-probe scripts/bundle.sh
```

Eine Probe-App entsteht ohne Sandbox, weil die Prüfhilfe mit Ordnern außerhalb
arbeitet; `ZUNDER_SANDBOX=1` baut sie mit – dann ohne `ZUNDER_PROFILE_DIR` und
`ZUNDER_DOWNLOAD_DIR` starten.

Die Prüfhilfe ist ein eigenes Bau-Merkmal (`harness`, siehe `src/dev.rs`). Die
gewöhnliche App enthält sie nicht und wertet keine der folgenden Variablen aus
– sonst könnte jedes Programm, das Zunder starten darf, den Browser von außen
fernsteuern und etwa die Abfrage per Touch ID umgehen. Die eigene Kennung
braucht es, weil macOS Cookies und Website-Daten je Kennung ablegt: Ohne sie
teilte sich die Probe-App diese mit der echten App, und „Browserdaten löschen“
im Test träfe die echten Anmeldungen. Eine Probe-App benutzt außerdem immer
eigene Einträge im Schlüsselbund (`app.zunder.browser.test`) und, wenn nichts
anderes gesagt ist, ein eigenes Profil („Zunder-Probe“).

In der Probe-App gilt:

- `ZUNDER_PROFILE_DIR=<ordner>` nutzt ein eigenes Profil,
  `ZUNDER_DOWNLOAD_DIR=<ordner>` einen eigenen Download-Ordner,
  `ZUNDER_UI_DIR=ui` lädt die Oberfläche frisch von der Platte.
- `ZUNDER_DEBUG=1` nimmt Befehle über die Standardeingabe an: `dump` (Zustand
  als JSON), `menu <id>`, `ui <json>`, `js <code>`, `jsin <tab-nr> <code>`,
  `uijs <code>`, `aijs <code>` (im Assistenten), `bubjs <code>` (in der
  Sprechblase), `isojs <code>` (in Zunders abgeschotteter Welt der Seite),
  `snap <ordner>` (Bilder von Rahmen, Seiten, Assistent und Sprechblase),
  `filechosen <zweck> <pfad>` (als wäre im Dateidialog gewählt worden),
  `hooks`, `lights`, `appearance dark|light`, `sleep <ms>`. Für Dialoge:
  `abortmodal <s>` bricht den nächsten nach so vielen Sekunden ab,
  `modalclick <s> <aufschrift>` drückt dann den Knopf mit dieser Aufschrift,
  `modalclear` verwirft, was davon noch aussteht. Damit lässt sich der Browser
  ohne Maus und Tastatur durchprüfen.
- Mit `ZUNDER_DEBUG`, für Kamera und Mikrofon: `ZUNDER_MOCK_MEDIA=1` stellt
  WebKits Testgeräte bereit (macOS fragt dann nicht nach den echten, und es
  wird nichts aufgenommen), `ZUNDER_MEDIA_HTTP=1` lässt die unverschlüsselten
  Probe-Seiten beim Merken wie verschlüsselte zählen,
  `ZUNDER_ALWAYS_VISIBLE=1` lässt Seiten auch in einem verdeckten Probefenster
  als sichtbar gelten, und `ZUNDER_MODAL_FRONT=1` lässt die Wartezeit von
  „Erlauben“ auch laufen, wenn die Probe-App nicht vorn ist.
- Mit `ZUNDER_DEBUG` richten `ZUNDER_CLAUDE_URL` und `ZUNDER_CLAUDE_KEY` den
  Claude-Weg auf einen Nachbau, `ZUNDER_OLLAMA_URL` auf eine andere
  Ollama-Adresse, `ZUNDER_AI_HELPER` auf einen anderen Helfer.
- Mit `ZUNDER_DEBUG`, für den Passwort-Tresor: `ZUNDER_AUTH=ok` bzw. `deny`
  beantwortet die Abfrage per Touch ID, `ZUNDER_VAULT_KEY=<64 Hex-Zeichen>`
  ersetzt den Schlüssel aus dem Schlüsselbund, `ZUNDER_PASTEBOARD=<name>` nimmt
  statt der echten eine eigene Zwischenablage, und
  `ZUNDER_IMPORT_ROOT=<ordner>` steht für den Ort, an dem andere Browser ihre
  Lesezeichen ablegen. So fasst ein Probelauf weder die Zwischenablage noch
  fremde Browserdaten an.

`cargo test` prüft die Deutung von Eingaben in der Adressleiste, die
Datenhaltung, die Sperrliste, den Schutz beim Navigieren, das Zerlegen und
Verpacken von Seitentext, die Form der Anfragen an die Modelle, das Ausliefern
der Hintergründe, den Passwort-Tresor (Zuordnung zu Websites, Verschlüsselung,
Vorschläge, CSV), das Einlesen von Lesezeichen, den Übersetzer für
Filterlisten – auch mit den mitgelieferten Listen als Ganzes –, das Erkennen
eines Tor-Zugangs, das Übersetzen von Seiten (Sätze bündeln, Antworten des
Modells zuordnen, Sprache erkennen), den Seitencheck (Schutzregeln lesen,
Bibliotheken an ihrer Adresse erkennen, was eine Website über sich sagt, nur
in fester Form zeigen), die Regeln zu Kamera und Mikrofon (wann gefragt,
erlaubt oder abgelehnt wird, wie lange ein Nein hält, unter welchem Namen
sich Zunder eine Website merkt) und die Regeln für eingebettete PDFs (wer
abliefern darf, was als PDF gilt, wer es wieder bekommt, was wann weicht –
und welcher Download als ungefragt gilt – sowie die eingebauten Apps (Adressen, Schutzregel, Anfragen, Karteikarten, Dateinamen, Rundung des Standorts, Dateinamen, Abruf-Freigabe, Übersetzung aller Texte; dazu der Sicherheitscheck (eine Seite abrufen, Quelltext lesen, bewerten) und der Request-Inspektor (Methode, Adresse, Kopfzeilen und Rumpf prüfen und senden); 141 Tests).

Der Name „Zunder“ ist ein Arbeitstitel. Er steht in `Cargo.toml`,
`scripts/bundle.sh`, `src/menu.rs`, `src/i18n.rs` und im Schema `zunder://`.
