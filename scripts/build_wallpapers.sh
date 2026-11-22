#!/bin/sh
# Bereitet die Live-Hintergründe der Startseite auf.
#
# Die Rohfassungen stammen von Higgsfield (Standbild: gpt_image_2_6, Bewegung:
# wan3_0 mit dem Standbild als erstem und letztem Bild) und liegen unter
# assets/higgsfield-raw/wallpaper-<name>/. Daraus entstehen je Hintergrund:
#
#   assets/wallpapers/<name>.mp4        nahtlose Schleife, ohne Ton, klein
#   assets/wallpapers/<name>.jpg        Standbild – sofort da, bevor das Video läuft
#   assets/wallpapers/<name>-thumb.jpg  Vorschau für die Auswahl
#
# Nahtlos heißt: Das Ende wird über eine gute halbe Sekunde in den Anfang
# geblendet, sodass beim Wiederholen kein Ruck zu sehen ist.
#
# Braucht ffmpeg (brew install ffmpeg). Die App selbst braucht es nicht.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RAW="$ROOT/assets/higgsfield-raw"
OUT="$ROOT/assets/wallpapers"
FADE=0.6
mkdir -p "$OUT"

for NAME in glut nordlicht kristall duenen tiefsee wolken; do
    SRC="$RAW/wallpaper-$NAME/$NAME.mp4"
    [ -f "$SRC" ] || { echo "fehlt: $SRC" >&2; exit 1; }
    DURATION="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$SRC")"
    OFFSET="$(echo "$DURATION - 2 * $FADE" | bc -l)"

    ffmpeg -v error -y -i "$SRC" -an -filter_complex \
        "[0:v]split[a][b];[a]trim=0:$FADE,setpts=PTS-STARTPTS[head];[b]trim=start=$FADE,setpts=PTS-STARTPTS[body];[body][head]xfade=transition=fade:duration=$FADE:offset=$OFFSET,format=yuv420p[v]" \
        -map "[v]" -r 30 -c:v libx264 -profile:v high -preset slow -crf 25 \
        -movflags +faststart "$OUT/$NAME.mp4"

    ffmpeg -v error -y -i "$OUT/$NAME.mp4" -frames:v 1 -q:v 4 "$OUT/$NAME.jpg"
    ffmpeg -v error -y -i "$OUT/$NAME.mp4" -frames:v 1 -vf "scale=480:-2" -q:v 5 "$OUT/$NAME-thumb.jpg"
    echo "$NAME: $(du -h "$OUT/$NAME.mp4" | cut -f1) Video, $(du -h "$OUT/$NAME.jpg" | cut -f1) Standbild"
done
echo "Fertig: $OUT ($(du -sh "$OUT" | cut -f1))"
