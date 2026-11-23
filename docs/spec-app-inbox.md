# Spec: `app-inbox` — Zunder als Client für One & Atlas

Status: Entwurf · 23.11.2026 · eingebaute App Nr. 6

## 1. Ziel und Abgrenzung

Zunder bleibt **Client**. Eine eingebaute App bündelt die „Was braucht meine
Aufmerksamkeit"-Sicht aus **Atlas** und **One** an einer Stelle: offene / mir
zugewiesene / auf meine Bestätigung wartende Work-Items, Erwähnungen und
Kommentare — mit Klick direkt zum Record/zur Seite im Tab und (später) einer
Inline-Antwort. Die volle Bearbeitung bleibt in der jeweiligen Web-App.

**Nicht-Ziele:** Records/Seiten editierbar nachbauen; ein eigener Server;
ein eigenes Chat-Backend; Rendering der One/Atlas-Oberfläche.

## 2. Architektur — wie Zunder spricht

- Eingebaute App `zunder://app-inbox/`, Menü `app_inbox`, Dateien
  `ui/app-inbox.{html,css,js}`, Icon (z. B. `inbox`, neu über Higgsfield).
- **Transport: nativer Abruf wie der Request-Inspektor** (`ureq`), NICHT
  `fetch()` aus der Seite — eine Seite unter `zunder://app-inbox/` scheitert an
  CORS und läse keine fremden Antwortköpfe (dasselbe Argument wie bei
  Sicherheitscheck und Request-Inspektor).
  - Neuer `Op::Api { service, method, path, query, body }` + Flag `App.api`
    (nur diese App true), analog zu `Op::Scan`/`App.scans` und
    `Op::Request`/`App.requests`.
  - Nativer Handler in z. B. `src/inbox.rs`: Hintergrund-Thread, Token aus dem
    Tresor eingesetzt, Ergebnis per `UserEvent::Api { window, tab, app, n, result }`
    → `app_reply`. `begin/finish`-Deckel (max. 3) wie Scan/Request.
- **Auth:** `Authorization: Bearer <token>` je Dienst. Der Atlas-Token ist
  tenant-scoped (der Tenant steckt im Token). Tokens liegen im **Tresor**
  (Keychain, `vault.rs`), je (Dienst[, Tenant]) — nie in der SQLite-Datei, nie
  in Logs, **nie an die Seite**. Die Seite stellt nur Anfrage-Deskriptoren,
  der native Teil hängt den Token an.
- **Server-Adressen als Einstellung, nichts fest verdrahtet.** Im öffentlichen
  Repo stehen keine konkreten Hosts (`<ATLAS_BASE>`, `<ONE_BASE>` sind
  Platzhalter; der echte Wert kommt aus den Einstellungen). Nur die
  konfigurierten Dienst-Basen dürfen angesprochen werden (Allowlist wie
  `connect`), kein beliebiges Ziel.
- **Cache:** Inbox-Items lokal in `Z.records` (sofort da, offline lesbar),
  Abgleich bei Fokus/`visibilitychange` und optional Intervall.
- **Kanal-Limit 4096 B** (`native.rs`): der Anfrage-Deskriptor bleibt klein;
  die Antwort darf groß sein (nativ, wie `BODY_MAX` beim Inspektor).

## 3. Atlas-API (bestätigt aus dem Ingest-Weg)

Base: `<ATLAS_BASE>` (Einstellung). Auth: `Authorization: Bearer <token>`.
Manche Endpunkte liegen hinter einem Cookie-Tor (`atlas_token`-Cookie) —
falls nötig zusätzlich mitsenden.

| Zweck | Aufruf |
|---|---|
| **Inbox in EINEM Call** | `GET /api/context?project=<p>&shadowOwner=<Name>` → `openItems` (in-progress / awaiting-confirmation), `recentRecords`, `sessionNotes`, `activeConstraints`, `memories`. Läuft auch, wenn der MCP aus ist. |
| Offene Work-Items | `GET /api/ingest?kind=feature,ticket,bug` |
| Ein Record | `GET /api/ingest?ref=<ref>` |
| Record anlegen / ändern | `POST` / `PATCH /api/ingest` (`appendBody` = nicht destruktiv) |
| Record-Arten | `GET /api/record-kinds` |
| Anhang | `POST /api/ingest/attachments` (JSON `{ref, filename, mime, contentBase64|text}`) |

**Work-Item-Felder** (aus einer echten Abfrage): `ref`, `displayRef`, `title`,
`kind`, `status`, `project`, `slug`, `href` (= `/docs/<project>/<slug>`),
`assignee`, `priority`, `due`, `milestone`, `createdBy`, `tags`, `updated`.

**Filter „mein Posteingang"** über AQL (Abfragesprache, z. B. per MCP
`atlas_query`; exakten HTTP-Query-Pfad vor Bau bestätigen):
- mir zugewiesen: `assignee = "Marcel Weissgerber" AND status != done`
- wartet auf meine Bestätigung: `tags IN ("awaiting-confirmation") AND createdBy = "Marcel Weissgerber"`
- Mehrtenant: 5 Tenants; je Item `project`/Tenant → in der Liste nach
  Tenant/Projekt gruppieren.

**Deep-Link zum Tab:** `<ATLAS_BASE>/r/<ref>` oder `<ATLAS_BASE>/docs/<project>/<slug>`.

**Antwort/Kommentar:** MCP `atlas_comment` ist der klare Weg; der HTTP-Pfad
(über die `/api/ingest`-Familie) ist vor dem Bau zu bestätigen.

## 4. One-API (Modell; Transport vor Bau bestätigen)

One ist **local-first über eine Brücke** (localhost-Port, MCP/JSON-RPC) **oder
gehostet** (`<ONE_BASE>`, an einen Tenant gebunden). Während dieser Recherche
war die lokale Brücke durch einen anderen Client belegt (bekannter
Port-Konflikt) → die exakten HTTP-Endpunkte sind **vor dem Bau zu verifizieren**.

Datenmodell (aus One): Workspaces, Pages, Databases, **Work-Items**
(`> [!TODO] Titel {#wi_…}`, Liste über `list_work_items`), Kommentare, Suche,
read-only One-Script-Query. Deep-Link zu einer Seite: `#/p/<id>`.

Inbox-Beitrag von One: mir zugewiesene / offene Work-Items und Erwähnungen je
Workspace. Zwei Transport-Optionen abwägen:
- **(a) lokale Brücke** — schon vorhanden, aber MCP-Protokoll statt simpler REST;
- **(b) gehostete HTTP-API mit Token** — einheitlich mit Atlas. **Empfehlung (b)**,
  falls One eine Token-HTTP-API bietet; sonst (a) über einen schlanken nativen
  JSON-RPC-Client.

## 5. UI — was man sieht

- **Posteingang** (eine Liste): Items aus Atlas **und** One zusammengeführt,
  gruppiert nach Quelle/Tenant/Projekt, sortiert nach Fälligkeit/Neuheit. Je
  Item: Titel, Quelle-Badge (One/Atlas), Art (`kind`), Status, Projekt, fällig,
  Priorität.
- **Klick** → Record/Seite im Tab (Deep-Link, neuer Tab mit `noopener`).
- **Filter** im Kopf: nur offen · nur mir zugewiesen · wartet auf mich · Tenant.
- **Inline-Antwort** je Item (Kommentar) — zuerst Atlas (klarer Weg), One später.
- **Benachrichtigung:** Zunders Hinweis-/Badge-System meldet neue Items
  (Polling-Intervall; echtes Push später).
- Saubere Zustände Leer / Lädt / Fehler / kein Token (wie die anderen Apps).

## 6. Sicherheit

- Token **nur nativ** (Tresor), nie an die Seite, nie in DB/Logs.
- Server-URLs als Einstellung, **nicht im Repo** (keine konkreten
  Infra-Hosts im öffentlichen Code).
- Host-Allowlist: nur die konfigurierten Dienst-Basen, kein beliebiges Ziel.
- **Im Stealth-Fenster verweigert** (eigener Abruf umginge den Tunnel — wie
  Scan/Request). Im privaten Fenster: kein Token speichern, Inbox nur flüchtig.
- Schreiben (Kommentar, Statuswechsel) nur auf echten Klick; Statuswechsel
  bestätigen (nichts still ändern).

## 7. MVP — erster Baustein

1. `app-inbox` zeigt die Atlas-`openItems` aus `GET /api/context` für Marcels
   Projekte, **read-only**.
2. Token einmal einfügen (Tresor), `<ATLAS_BASE>` als Einstellung.
3. Klick öffnet `/r/<ref>` im Tab. Items lokal in `Z.records` zwischengespeichert.

Danach schrittweise: One-Items dazu → Inline-Antwort (Atlas) → Benachrichtigungen
→ „Von überall erfassen" (Seitentext markieren → Record/Notiz/Kommentar).

## 8. Vor dem Bau zu klären

- **One:** exakte HTTP-API + Transport (Brücke vs. gehostet) — Brücke war belegt.
- **Atlas-Kommentar/Reply über HTTP:** exakten Pfad bestätigen (MCP `atlas_comment`
  ist klar).
- **AQL über HTTP:** Query-Endpunkt für die präzisen „mein Posteingang"-Filter.
- **„@Erwähnung / mir zugewiesen":** welche Felder/Tags genau (`assignee`,
  `awaiting-confirmation` + `createdBy`, Erwähnungen).
- **Token-Beschaffung:** Paste (MVP) vs. OAuth (Atlas hat einen OAuth-Proxy) —
  später OAuth.
- **Rust-seitig:** `Op::Api` + `App.api`, Host-Allowlist aus Einstellungen,
  Timeout/Nebenläufigkeit wie Scan/Request, `UserEvent::Api`.
