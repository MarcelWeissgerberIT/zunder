# Filterlisten des Werbeblockers

Diese Dateien sind **unveränderte Kopien** der öffentlichen Filterlisten von
[easylist.to](https://easylist.to/) – der Stand zum Zeitpunkt des Baus. Zunder
liefert sie mit, damit der Blocker vom ersten Start an vollständig arbeitet,
und holt danach alle vier Tage selbst die neueste Fassung.

| Datei | Liste | Quelle |
| --- | --- | --- |
| `easylist.txt` | EasyList | <https://easylist.to/easylist/easylist.txt> |
| `easylistgermany.txt` | EasyList Germany | <https://easylist.to/easylistgermany/easylistgermany.txt> |
| `easyprivacy.txt` | EasyPrivacy | <https://easylist.to/easylist/easyprivacy.txt> |

**Urheber und Lizenz:** © The EasyList authors (<https://easylist.to/>). Die
Listen stehen wahlweise unter der GNU General Public License, Version 3 oder
neuer, oder unter Creative Commons Attribution-ShareAlike 3.0 Unported oder
neuer – siehe <https://easylist.to/pages/licence.html>. Der Lizenzhinweis steht
auch im Kopf jeder Datei.

Zunder verändert die Listen nicht. Es übersetzt sie zur Laufzeit in das
Regelformat von WebKit (`src/filters.rs`); was sich dort nicht genau ausdrücken
lässt, bleibt weg. Auffrischen lässt sich der mitgelieferte Stand mit:

```bash
for n in easylist easylistgermany easyprivacy; do curl -fsSL -o "assets/filters/$n.txt" "https://easylist.to/$( [ "$n" = easylistgermany ] && echo easylistgermany || echo easylist )/$n.txt"; done
```
