#!/bin/sh
# Compare wizard-ui-claude with Claude Code, cell by cell, in a private tmux server.
#
#   crates/claudew/tools/compare.sh                  the look against tests/fixtures/ref
#   REF=live crates/claudew/tools/compare.sh         capture the real `claude` first
#   REF=live SAVE=1 crates/claudew/tools/compare.sh  ... and store it as the new fixtures
#
# Prints a mismatch count per scene and size, writes a side by side per scene to
# $WORK/report, and exits 1 if any cell differs for a reason compare.py does not list.
# The light theme is compared too. A live capture sends two tiny prompts and a /compact per
# size through `claude` (CLAUDE_BIN), on whatever account it is logged in to; it never
# touches ~/.claude.
# Needs tmux and python3. Every tmux call uses the socket `lookclaude`.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
crate=$(dirname "$here")
root=$(cd "$crate/../.." && pwd)
work=${WORK:-$(mktemp -d "${TMPDIR:-/tmp}/claudew-compare.XXXXXX")}
sizes=${SIZES:-80x24,120x40,200x50}
# The directory the fixtures were captured in, user name scrubbed. The header shows it,
# and its length decides wraps, so both programs run in exactly this path.
cwd=/tmp/claude-1000/-home-owner/00000000-0000-0000-0000-000000000000/scratchpad/look-claude/proj
reset() {
    mkdir -p "$cwd/src"
    printf 'fn main() {\n    println!("hello");\n}\n' > "$cwd/main.rs"
    printf '# notes\n\nalpha\nbeta\n' > "$cwd/notes.md"
    echo 'pub fn add(a: i32, b: i32) -> i32 { a + b }' > "$cwd/src/lib.rs"
}
reset
(cd "$root" && cargo build -q -p claudew)
bin="$root/target/debug/claudew"

if [ "${REF:-}" = live ]; then
    ref="$work/ref"
    claude=${CLAUDE_BIN:-claude}
    cmd="$claude --setting-sources project --strict-mcp-config --permission-mode default --model haiku"
    python3 "$here/drive.py" --target ref --cmd "$cmd --settings '{\"tui\":\"fullscreen\"}'" --cwd "$cwd" --out "$ref" --group static --sizes "$sizes"
    python3 "$here/drive.py" --target ref --cmd "$cmd --settings '{\"tui\":\"fullscreen\",\"theme\":\"light\"}'" --cwd "$cwd" --out "$ref" --group light --sizes "$sizes"
    python3 "$here/drive.py" --target ref --cmd "$cmd --settings '{\"tui\":\"fullscreen\"}'" --cwd "$cwd" --out "$ref" --group turn --sizes "$sizes" \
        --before "printf 'fn main() {\n    println!(\"hello\");\n}\n' > main.rs"
    # scrub what is personal: the account's address and organisation, the home directory
    python3 - "$ref" <<'PY'
import glob, re, sys
for f in glob.glob(sys.argv[1] + '/*.ansi'):
    t = open(f, encoding='utf-8').read()
    t = re.sub(r'[\w.+-]+@[\w-]+(\.[\w-]+)+', 'owner@example.com', t)
    t = re.sub(r"(Organization:(\x1b\[[0-9;]*m)?\s+).*", r"\1Example Organization", t)
    t = re.sub(r'-home-[a-z0-9_]+/', '-home-owner/', t)
    open(f, 'w', encoding='utf-8').write(t)
PY
    if [ "${SAVE:-}" = 1 ]; then
        rm -f "$crate/tests/fixtures/ref"/*
        cp "$ref"/* "$crate/tests/fixtures/ref/"
        python3 "$here/mkreplay.py" "$ref" "$cwd" "$crate/tests/fixtures/replay"
    fi
else
    ref="$crate/tests/fixtures/ref"
fi

python3 "$here/mkreplay.py" "$ref" "$cwd" "$work/replay"
for size in $(echo "$sizes" | tr , ' '); do
    for group in static turn light; do
        script="$work/replay/$group-$size.jsonl" theme=dark
        [ "$group" = light ] && script="$work/replay/static-$size.jsonl" theme=light
        reset
        python3 "$here/drive.py" --target look --cwd "$cwd" --out "$work/look" --group "$group" --sizes "$size" \
            --cmd "env CLAUDEW_BACKEND=replay CLAUDEW_REPLAY=$script $bin --theme $theme --cwd $cwd" > /dev/null
    done
done
echo "captures and report in $work"
python3 "$here/compare.py" "$ref" "$work/look" --report "$work/report" --sizes "$sizes"
