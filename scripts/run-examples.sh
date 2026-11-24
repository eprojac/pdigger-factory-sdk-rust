#!/bin/sh
# Runs every example that the READMEs and docs/ quote, with no network: the replays over the
# fixture recordings, the client examples against the SDK's fake unit on 127.0.0.1
# (examples/fake_server.rs, the recording replayed as its camera), and the headless
# screenshots. Then checks that every code block of the docs marked
# `<!-- example: <path> -->` is that file, unchanged.
set -eu
cd "$(dirname "$0")/.."
cargo build --workspace --examples --all-features --locked -q
bin=target/debug/examples
out=$(mktemp -d)
server=
recorded=
cleanup() {
  if [ -n "$server" ]; then kill "$server" 2>/dev/null || true; fi
  if [ -n "$recorded" ]; then kill "$recorded" 2>/dev/null || true; fi
  rm -rf "$out"
}
trap cleanup EXIT

# GNU timeout bounds each example (gtimeout on macOS with Homebrew's coreutils; neither: no bound).
limit=""
if command -v timeout > /dev/null 2>&1; then
  limit="timeout 60"
elif command -v gtimeout > /dev/null 2>&1; then
  limit="gtimeout 60"
fi

# run <example> [args]: runs it (60 s at most) and shows its output.
run() {
  name=$1
  shift
  echo "== examples/$name.rs $*"
  if ! $limit "$bin/$name" "$@" > "$out/$name.txt" 2>&1; then
    cat "$out/$name.txt"
    echo "examples/$name.rs failed"
    exit 1
  fi
  cat "$out/$name.txt"
}

# expect <example> <text>: its output has that line.
expect() {
  if ! grep -qxF "$2" "$out/$1.txt"; then
    echo "examples/$1.rs did not print: $2"
    exit 1
  fi
}

# expect_match <example> <regex>: its output has a line that matches (extended regex, whole line).
expect_match() {
  if ! grep -qxE "$2" "$out/$1.txt"; then
    echo "examples/$1.rs printed no line like: $2"
    exit 1
  fi
}

recording=fixtures/recordings/dry1-0296-0306-full.ndjson.gz
run replay "$recording"
expect replay "5:01.6 Right hand moves stone 643 1.09 units in 1.2 s"
run testing_quickstart
expect testing_quickstart "5:01.6 Right hand lifts stone 643"
# The tutorials' programs print what the Python and TypeScript ones print over the same frames.
run tutorial_high_level fixtures/recordings/dry1-0018-0153.ndjson.gz 155
expect tutorial_high_level "0:20.5 Both hands push stones 86, 90 and 94 (6 pushes in 0.6 s)"
expect tutorial_high_level "1:19.2 Left hand takes stone 155 away (1.0 s)"
expect tutorial_high_level "1:19.2 stone 155: left hand, carried_away, 1.0 s"
expect tutorial_high_level "0:45.5 tray 1: stone 20261122T023847.0Z-1-117 was removed"
expect tutorial_high_level "2:24.0 tray 1 leaving: 44 stones (30 sure), 0 added, 6 removed, 18 unclear"
expect tutorial_high_level "tray 1: 0:22.5 to 2:24.0, 6 stones removed for certain"
expect tutorial_high_level "frames: 676 (with world 676, without 0, revisions 0)"
expect tutorial_high_level "stone 155: 5 points from 1:15.6 to 1:19.8"
[ "$(grep -c ': .* hand, ' "$out/tutorial_high_level.txt")" -eq 9 ] || { echo "tutorial_high_level printed no 9 moves"; exit 1; }

# The first tutorial and the reports quickstart: one report per tray, as rows, and the facts
# behind a report that is not complete, as the other languages print them.
slice=fixtures/recordings/dry1-0018-0153.ndjson.gz
run quickstart_reports "$slice"
diff -u fixtures/golden/dry1-0018-0153.reports.txt "$out/quickstart_reports.txt" || { echo "quickstart_reports differs"; exit 1; }
run tutorial_reports "$slice"
reports=$(wc -l < fixtures/golden/dry1-0018-0153.reports.txt)
head -n "$reports" "$out/tutorial_reports.txt" | diff -u fixtures/golden/dry1-0018-0153.reports.txt - \
  || { echo "tutorial_reports: the reports differ"; exit 1; }
row=$(sed -n "$((reports + 1))p" "$out/tutorial_reports.txt")
# (the changes at least 0.9 likely: the six certain removals and nine unclear additions; every move)
for part in '"tray":1' '"complete":false' '"unclear":18' '"moved":["20261122T023847.0Z-1-85",' \
  '"added":["20261122T024104.0Z-1-177","20261122T024018.8Z-1-193","20261122T024029.7Z-1-201",' \
  '"20261122T024124.2Z-1-282","20261122T024126.9Z-1-284"],' \
  '"removed":["20261122T023847.0Z-1-117","20261122T023847.0Z-1-127","20261122T023847.0Z-1-126",'; do
  case $row in *"$part"*) ;; *) echo "tutorial_reports: the row of tray 1 has no $part: $row"; exit 1 ;; esac
done
tail -n +"$((reports + 2))" "$out/tutorial_reports.txt" | diff -u fixtures/golden/dry1-0018-0153.trays.txt - \
  || { echo "tutorial_reports: the facts differ"; exit 1; }

# The low-level tutorial against the fake unit playing the recording (not as a camera), at frame 1510.
"$bin/fake_server" "$recording" > "$out/recorded.txt" 2>&1 &
recorded=$!
i=0
while ! grep -q '^http://' "$out/recorded.txt" 2>/dev/null; do
  i=$((i + 1))
  if [ "$i" -gt 300 ]; then
    cat "$out/recorded.txt"
    echo "the fake unit did not start"
    exit 1
  fi
  sleep 0.1
done
run tutorial_low_level "$(grep -m 1 '^http://' "$out/recorded.txt")" 1510
kill "$recorded" 2>/dev/null || true
recorded=
cat > "$out/tutorial_low_level.expected" << 'END'
ready true, worker idle: 3.8 frames a second, 241.6 ms each
checkpoint 8d09531c915f on trt: 6 engines
70 joints, 62 bones, 11 colours
stone 627: 51 points, g 1480 to 1530
focus on stone 627: dry run true
frame g 1510 at 5:01.2: 42 stones, rect
maps of g 1510: 40 x 64 cells, 920 of the tray, 1169 of stones
overlay of g 3079: 44 polygons, 7 segments, 8 circles at 1920 x 1080 px
scene of g 3079: 42 stones, 7 bones
g 1510: 42 outlines, 168 points
g 1510: 50 of 62 bones
left hand: Hand on a stone, wrist at 1528, 595 px
right hand: Holding a stone, wrist at 1270, 415 px
regions of g 1510: a 1106-byte PNG
picture: /api/v2/videos/dry1/media at 5:01.2, 960 x 540
END
# (the fake does not draw: the draw lists are the unit's recorded ones, of dry2's frame 3079)
diff -u "$out/tutorial_low_level.expected" "$out/tutorial_low_level.txt" || { echo "tutorial_low_level differs"; exit 1; }

"$bin/fake_server" "$recording" --live > "$out/server.txt" 2>&1 &
server=$!
i=0
while ! grep -q '^http://' "$out/server.txt" 2>/dev/null; do
  i=$((i + 1))
  if [ "$i" -gt 300 ]; then
    cat "$out/server.txt"
    echo "the fake unit did not start"
    exit 1
  fi
  sleep 0.1
done
url=$(grep -m 1 '^http://' "$out/server.txt")
echo "== the fake unit: $url (its camera cam-live shown: g from 0, t from 0.5 s, the frames after its warm-up)"
run quickstart "$url"
[ "$(wc -l < "$out/quickstart.txt")" -eq 20 ] || { echo "the quickstart printed no 20 events"; exit 1; }
run interpreter "$url"
expect interpreter "0:06.9 Right hand moves stone 643 1.09 units in 1.2 s"
run errors "$url"
# The fake's camera session counts g from 0 but releases no frame of its warm-up; a unit's long session no longer
# holds frame 1 either.
expect_match errors 'frame 1( is still held \(session [0-9]+\)|: HTTP 404 not_retained)'
run session "$url"
expect session "cam-live    live camera: Live camera"
expect session "first live camera: cam-live (FakeFactory)"
run streams "$url"
expect streams "frame g 16 at 3.7 s: 42 stones (revision: false)"
run low_level "$url"
expect low_level "newest frame: g 50 at 10.5 s, 41 stones"
run replay_url "$url/developers/data/dry1-0296-0306-full.ndjson.gz"
expect replay_url "4:55.8 stone 683 appears"
run tray_points fixtures/recordings/dry1-0018-0153.ndjson.gz 1 "$url"
expect tray_points "tray 1: arrived 0:22.5, leaving 2:24.0"
expect tray_points "  removed: 20261122T023847.0Z-1-117"
expect tray_points "seek to 0:22.5 would show dry1 from 0:22.5"
expect tray_points "seek to 2:24.0 would show dry1 from 2:24.0"
# Last: it selects the camera again (connecting, warming up on the fake's clock, which runs in real time), then live.
run live_camera "$url" 4
expect_match live_camera 'cam-live \(Live camera\) is live: session [0-9]+, 0\.25 s behind the camera'
expect_match live_camera 'frame g [0-9]+: a [0-9]+-byte JPEG'
expect_match live_camera '[0-9]{2}:[0-9]{2}:[0-9]{2} UTC  [0-9]+:[0-9]{2}\.[0-9] .+'
[ "$(wc -l < "$out/live_camera.txt")" -eq 6 ] || { echo "live_camera printed no 4 events"; exit 1; }
run screenshots "$out/img"
for picture in viewer-dry1-0296-0306-full-g1516 overlay-dry1-0296-0306-full-g1516 scene-dry1-0296-0306-full-g1516 \
  reports-dry1-0018-0153; do
  [ -s "$out/img/$picture.png" ] || { echo "no $picture picture"; exit 1; }
done

# The docs' code blocks are the examples.
mkdir -p "$out/snippets"
checked=0
for doc in README.md crates/*/README.md docs/*.md docs/tutorials/*.md; do
  [ -f "$doc" ] || continue
  prefix="$out/snippets/$(echo "$doc" | tr '/' '_')"
  awk -v prefix="$prefix" '
    /^<!-- example: [^ ]+ -->$/ { path = $3; next }
    path != "" && /^```rust$/ { n++; file = prefix "." n; print path > (file ".path"); close(file ".path"); capture = 1; next }
    capture && /^```$/ { capture = 0; path = ""; close(file); next }
    capture { print > file }
  ' "$doc"
done
for snippet in "$out"/snippets/*.path; do
  [ -f "$snippet" ] || continue
  file=$(cat "$snippet")
  if ! diff -u "$file" "${snippet%.path}" > "$out/diff.txt"; then
    cat "$out/diff.txt"
    echo "a code block of the docs differs from $file"
    exit 1
  fi
  checked=$((checked + 1))
done
echo "examples: all ran; $checked code blocks of the docs are the examples"
