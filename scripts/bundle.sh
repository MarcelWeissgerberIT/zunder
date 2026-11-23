#!/bin/sh
# Baut Zunder und packt daraus die Mac-App dist/Zunder.app.
#
#   scripts/bundle.sh            bauen und bündeln
#   scripts/bundle.sh --install  zusätzlich nach /Applications kopieren
#
# Für einen Probelauf mit Prüfhilfe (Befehle über die Standardeingabe, siehe
# README, Abschnitt Entwicklung):
#   ZUNDER_HARNESS=1 ZUNDER_BUNDLE_ID=app.zunder.browser.test ZUNDER_DIST=<ordner> scripts/bundle.sh
#
# Die App wird nur lokal signiert (ad hoc). Das genügt, um sie auf dem
# eigenen Mac zu starten; zum Weitergeben bräuchte es eine Apple-Entwickler-ID.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
# Mit ZUNDER_DIST=<ordner> entsteht die App woanders – etwa für einen Probelauf,
# während die App aus dist/ noch läuft.
APP="${ZUNDER_DIST:-$ROOT/dist}/Zunder.app"
# Eine Probe-App braucht eine eigene Kennung (ZUNDER_BUNDLE_ID): macOS legt
# Cookies und Website-Daten je Kennung ab – sonst teilte sie sich diese mit der
# echten App.
BUNDLE_ID="${ZUNDER_BUNDLE_ID:-app.zunder.browser}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"

# Die Prüfhilfe gehört nie in die echte App: Mit ihr ließe sich der Browser von
# außen fernsteuern. Deshalb gibt es sie nur zusammen mit einer eigenen Kennung.
FEATURES=""
if [ -n "${ZUNDER_HARNESS:-}" ]; then
    if [ "$BUNDLE_ID" = "app.zunder.browser" ]; then
        echo "Die Prüfhilfe gibt es nur mit eigener Kennung (ZUNDER_BUNDLE_ID=…)." >&2
        exit 1
    fi
    FEATURES="--features harness"
fi

cd "$ROOT"
# shellcheck disable=SC2086
cargo build --release $FEATURES

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$TARGET_DIR/release/zunder" "$APP/Contents/MacOS/Zunder"
cp "$ROOT/assets/Zunder.icns" "$APP/Contents/Resources/Zunder.icns"

# Live-Hintergründe der Startseite (erzeugt mit scripts/build_wallpapers.sh).
# Fehlen sie, bietet die Startseite einfach keine an.
if ls "$ROOT/assets/wallpapers/"*.mp4 >/dev/null 2>&1; then
    mkdir -p "$APP/Contents/Resources/wallpapers"
    cp "$ROOT/assets/wallpapers/"*.mp4 "$ROOT/assets/wallpapers/"*.jpg "$APP/Contents/Resources/wallpapers/"
fi

# Filterlisten des Werbeblockers, unverändert wie von easylist.to geholt
# (Stand zum Zeitpunkt des Baus – die App frischt sie danach selbst auf).
if ls "$ROOT/assets/filters/"*.txt >/dev/null 2>&1; then
    mkdir -p "$APP/Contents/Resources/filters"
    cp "$ROOT/assets/filters/"*.txt "$APP/Contents/Resources/filters/"
fi

# Helfer für den Assistenten: spricht Apples Sprachmodell auf dem Gerät an
# (ab macOS 26). Lässt er sich nicht bauen, läuft Zunder trotzdem – der
# Assistent bietet dann nur Ollama und Claude an.
HELPER="$APP/Contents/MacOS/zunder-ai"
if command -v swiftc >/dev/null 2>&1; then
    if swiftc -O -parse-as-library -target arm64-apple-macos26.0 -module-name ZunderAI \
        "$ROOT/helper/zunder-ai.swift" -o "$HELPER" 2>"$TARGET_DIR/zunder-ai.log"; then
        codesign --force --options runtime --sign - "$HELPER" >/dev/null 2>&1
    else
        rm -f "$HELPER"
        echo "Hinweis: Der Helfer für Apples Gerätemodell ließ sich nicht bauen (siehe $TARGET_DIR/zunder-ai.log)."
    fi
else
    echo "Hinweis: Ohne Swift-Compiler entsteht kein Helfer für Apples Gerätemodell."
fi

# Sprachordner: Erst damit zeigt macOS seine eigenen Texte (Kontextmenüs,
# Dialoge, Fehlermeldungen) in der Sprache des Nutzers statt auf Englisch.
for LANGUAGE in de en; do
    mkdir -p "$APP/Contents/Resources/$LANGUAGE.lproj"
    printf '"CFBundleName" = "Zunder";\n' > "$APP/Contents/Resources/$LANGUAGE.lproj/InfoPlist.strings"
done

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Zunder</string>
    <key>CFBundleDisplayName</key><string>Zunder</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key><string>Zunder</string>
    <key>CFBundleIconFile</key><string>Zunder</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleDevelopmentRegion</key><string>de</string>
    <key>CFBundleLocalizations</key><array><string>de</string><string>en</string></array>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>

    <!-- Heruntergeladene Dateien als „aus dem Internet“ kennzeichnen, damit
         macOS sie vor dem ersten Öffnen prüft. -->
    <key>LSFileQuarantineEnabled</key><true/>

    <!-- Ein Browser muss auch unverschlüsselte Seiten anzeigen können. -->
    <key>NSAppTransportSecurity</key>
    <dict>
        <key>NSAllowsArbitraryLoadsInWebContent</key><true/>
        <key>NSAllowsLocalNetworking</key><true/>
    </dict>

    <!-- Kamera und Mikrofon: macOS fragt einmal für die App, WebKit zusätzlich je Seite. -->
    <key>NSCameraUsageDescription</key>
    <string>Webseiten können die Kamera nur nutzen, wenn du es ihnen erlaubst.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>Webseiten können das Mikrofon nur nutzen, wenn du es ihnen erlaubst.</string>

    <!-- Standort: nur für eingebaute Apps (Wetter), nie für Webseiten – und
         erst, wenn du es der App erlaubt hast. -->
    <key>NSLocationUsageDescription</key>
    <string>Die eingebaute Wetter-App zeigt damit das Wetter an deinem Ort. Webseiten bekommen deinen Standort nicht.</string>
    <key>NSLocationWhenInUseUsageDescription</key>
    <string>Die eingebaute Wetter-App zeigt damit das Wetter an deinem Ort. Webseiten bekommen deinen Standort nicht.</string>

    <!-- Als Browser anmelden: Links und HTML-Dateien öffnen. -->
    <key>CFBundleURLTypes</key>
    <array>
        <dict>
            <key>CFBundleURLName</key><string>Web site URL</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>CFBundleURLSchemes</key>
            <array><string>http</string><string>https</string></array>
        </dict>
    </array>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>HTML document</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key>
            <array><string>public.html</string><string>public.xhtml</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST

# Mit „Hardened Runtime“: Fremder Code lässt sich dann nicht in die laufende
# App einschleusen (keine untergeschobenen Bibliotheken, kein Anhängen von außen).
# Und in der Sandbox von macOS: Der Browser kommt nur an sein eigenes Profil, an
# den Download-Ordner und an Dateien, die man ihm ausdrücklich gibt (siehe
# assets/Zunder.entitlements). Für einen Probelauf mit Prüfhilfe bleibt die
# Sandbox aus, solange nicht ZUNDER_SANDBOX=1 gesetzt ist – die Prüfhilfe
# arbeitet mit Ordnern außerhalb. ZUNDER_SANDBOX=0 schaltet sie auch sonst ab.
SANDBOX="${ZUNDER_SANDBOX:-}"
if [ -z "$SANDBOX" ]; then
    if [ -n "${ZUNDER_HARNESS:-}" ]; then SANDBOX=0; else SANDBOX=1; fi
fi
if [ "$SANDBOX" = "1" ]; then
    codesign --force --options runtime --entitlements "$ROOT/assets/Zunder.entitlements" --sign - "$APP" >/dev/null 2>&1
    echo "Signiert mit Sandbox."
else
    # Ohne Sandbox bleiben Kamera und Mikrofon trotzdem nutzbar.
    ENT="$TARGET_DIR/zunder-ohne-sandbox.entitlements"
    cat > "$ENT" <<ENTITLEMENTS
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>com.apple.security.device.camera</key><true/>
    <key>com.apple.security.device.audio-input</key><true/>
    <key>com.apple.security.personal-information.location</key><true/>
</dict></plist>
ENTITLEMENTS
    codesign --force --options runtime --entitlements "$ENT" --sign - "$APP" >/dev/null 2>&1
    echo "Signiert ohne Sandbox."
fi
echo "Fertig: $APP ($(du -sh "$APP" | cut -f1), Version $VERSION)"

if [ "${1:-}" = "--install" ]; then
    rm -rf "/Applications/Zunder.app"
    cp -R "$APP" "/Applications/Zunder.app"
    echo "Installiert: /Applications/Zunder.app"
fi
