// zunder-ai – Brücke von Zunder zu Apples Sprachmodell auf dem Gerät
// (Foundation Models, ab macOS 26). Alles läuft lokal, nichts geht ins Netz.
//
//   zunder-ai --check
//       eine Zeile: "available context=<n> german=<true|false>"
//       oder        "unavailable <grund>"
//
//   zunder-ai            (Anfrage als JSON über die Standardeingabe)
//       {"system": "...", "prompt": "...", "max_tokens": 900,
//        "history": [["frühere Frage", "frühere Antwort"], ...],
//        "exact": true}          (history und exact sind freiwillig)
//       Die Antwort wird fortlaufend als UTF-8 auf die Standardausgabe
//       geschrieben. Bei einem Fehler steht "ERROR <art> <text>" auf der
//       Fehlerausgabe und der Rückgabewert ist ungleich 0.

import Foundation
import FoundationModels

struct Request: Decodable {
    let system: String
    let prompt: String
    let history: [[String]]?
    let max_tokens: Int?
    /// Immer das wahrscheinlichste Wort nehmen – für Schritte, bei denen es
    /// auf eine feste Form ankommt und nicht auf abwechslungsreichen Text.
    let exact: Bool?
}

func text(_ content: String) -> [Transcript.Segment] {
    [.text(Transcript.TextSegment(content: content))]
}

func fail(_ kind: String, _ message: String, code: Int32) -> Never {
    FileHandle.standardError.write(Data("ERROR \(kind) \(message)\n".utf8))
    exit(code)
}

@main
struct ZunderAI {
    static func main() async {
        // Zusammenfassen und Umformen fremder Texte ist der Zweck – dafür gibt
        // es die nachsichtigere Schutzstufe für Inhalts-Umwandlungen.
        let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)

        if CommandLine.arguments.contains("--check") {
            switch model.availability {
            case .available:
                let german = model.supportsLocale(Locale(identifier: "de_DE"))
                print("available context=\(model.contextSize) german=\(german)")
            case .unavailable(let reason):
                switch reason {
                case .deviceNotEligible: print("unavailable device_not_eligible")
                case .appleIntelligenceNotEnabled: print("unavailable not_enabled")
                case .modelNotReady: print("unavailable model_not_ready")
                @unknown default: print("unavailable unknown")
                }
            }
            return
        }

        let input = FileHandle.standardInput.readDataToEndOfFile()
        guard let request = try? JSONDecoder().decode(Request.self, from: input) else {
            fail("request", "Anfrage nicht lesbar", code: 2)
        }
        guard case .available = model.availability else {
            fail("unavailable", "Modell nicht verfügbar", code: 3)
        }

        let turns = (request.history ?? []).filter { $0.count == 2 }
        let session: LanguageModelSession
        if turns.isEmpty {
            session = LanguageModelSession(model: model, instructions: request.system)
        } else {
            // Frühere Fragen und Antworten gehen als echter Gesprächsverlauf mit –
            // so hält das Modell sie nicht für Text, den es fortsetzen soll.
            var entries: [Transcript.Entry] = [
                .instructions(Transcript.Instructions(segments: text(request.system), toolDefinitions: []))
            ]
            for turn in turns {
                entries.append(.prompt(Transcript.Prompt(segments: text(turn[0]))))
                entries.append(.response(Transcript.Response(assetIDs: [], segments: text(turn[1]))))
            }
            session = LanguageModelSession(model: model, transcript: Transcript(entries: entries))
        }
        var options = GenerationOptions()
        options.maximumResponseTokens = request.max_tokens ?? 1200
        if request.exact == true {
            options.sampling = .greedy
        }

        do {
            // Der Strom liefert jeweils den gesamten bisherigen Text; weiter
            // gereicht wird nur der neue Teil.
            var written = 0
            for try await snapshot in session.streamResponse(to: request.prompt, options: options) {
                let text = snapshot.content
                if text.count > written {
                    let delta = String(text.dropFirst(written))
                    written = text.count
                    FileHandle.standardOutput.write(Data(delta.utf8))
                }
            }
        } catch let error as LanguageModelSession.GenerationError {
            switch error {
            case .exceededContextWindowSize:
                fail("context", "Text zu lang für das Modell", code: 4)
            case .guardrailViolation, .refusal:
                fail("guardrail", "Vom Modell abgelehnt", code: 5)
            case .unsupportedLanguageOrLocale:
                fail("language", "Sprache wird nicht unterstützt", code: 6)
            case .rateLimited, .concurrentRequests:
                fail("busy", "Modell ist gerade beschäftigt", code: 7)
            case .assetsUnavailable:
                fail("unavailable", "Modell wird noch geladen", code: 3)
            default:
                fail("other", error.localizedDescription, code: 8)
            }
        } catch {
            fail("other", error.localizedDescription, code: 8)
        }
    }
}
