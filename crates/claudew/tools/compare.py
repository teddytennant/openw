#!/usr/bin/env python3
"""Cell-by-cell diff of two capture directories written by drive.py.

    compare.py REF_DIR LOOK_DIR [--report DIR] [--sizes 80x24,120x40,200x50]

A cell is its character, foreground, background and the bold, dim, italic, underline, inverse
and strikethrough flags. Before diffing, both sides go through NORMALISE (text that differs
for a legitimate reason: branding, version, model names, times, counts, ids) and then EXPLAINED
masks cells that differ for a stated reason the look cannot or should not remove. Everything
else is a mismatch. Exit status is 1 when any scene has one.
"""
import argparse, fnmatch, glob, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from cells import parse, visible, fmt

VERSION = ''
BLANK = (' ', (None, None, False, False, False, False, False, False))

# (pattern, replacement). Applied to each row's text on both sides; the replacement takes the
# style of the first cell it replaces and the rest of the row shifts to fit. Only what changes
# from run to run is here: placeholders, spinner verbs, clocks, ids, counts.
NORMALISE = [
    (r'Try ".*"\s*$', 'Try "prompt"'),
    # tips are picked at random
    (r'^  ⎿ (?= |\xa0)(.)Tip: .*$', r'  ⎿ \1Tip: …'),
    # the spinner keeps its shape and colours: glyph, verb, then the seconds, tokens and effort
    (r'^[·✢*✳✶✻✽](?= \S+…)', '*'),
    (r'(?<=^\* )[A-Z][a-zé]+(?=…)', 'Verb'),
    (r'(?<=^\* Verb… \()\d+[hms]( \d+[ms])*(?= · )', 'Ns'),
    (r'(?<= · [↓↑] )[0-9.]+k?(?= tokens)', 'N'),
    (r'^✻ \S+ for (\d+[hms] ?)+ · done \d+:\d\d [AP]M', '✻ Worked for Ns · done H:MM XM'),
    (r'\d\d:\d\d [AP]M (\S+)(\s*)$', r'HH:MM XM \1\2'),
    (r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}', 'SESSION'),
    (r'↓ \d+ more', '↓ N more'),
    (r'\(\d+ of \d+\)', '(N of M)'),
    # the rule after the counter fills the row, so it goes with the counter
    (r'History \d+/\d+ ─*\s*$', 'History N/M'),
]

# Applied to the reference only: its product name and version become what the look must print,
# so a wrong version in the look is a mismatch. VERSION is filled in from --version.
REF_ONLY = [
    (r'Claude Code(?= v\d)', 'Wizard'),
    (r'(?<=Wizard )v\d[0-9.a-z-]*', 'v{VERSION}'),
    (r'(?<=Version:)(\s+)2\.\d+\.\d+', r'\g<1>{VERSION}'),
    (r'claude --resume', 'wizard-ui-claude --resume'),
    (r'\bClaude\b(?! Code| API|/| mobile|\.ai)', 'Wizard'),
]

# Reference scenes of a dialog Wizard never raises: it asks before no tool runs (its one
# question is a finished plan, compared in `plan`). The look still has these prompts for the
# replay, but a match with them would prove nothing about Wizard.
NOT_COMPARED = [
    ('edit-permission*', 'Wizard never asks before an edit'),
    ('write-permission', 'Wizard never asks before a write'),
    ('bash-permission', 'Wizard never asks before a command'),
    ('bash-esc', 'turning a command down at a prompt Wizard never shows'),
]

# (scene glob, which side the pattern is looked for on, row pattern, reason). The matched span
# of the row is left out of the diff on both sides. A pattern ending in `$$` masks from the
# match to the end of the row. The rest live in explained.tsv.
EXPLAINED = []


def load_rules(path):
    """More EXPLAINED rules from a file: `glob<TAB>side<TAB>pattern<TAB>reason` per line."""
    out = []
    if path and os.path.exists(path):
        for line in open(path, encoding='utf-8'):
            line = line.rstrip('\n')
            if line and not line.startswith('#'):
                out.append(tuple(line.split('\t')))
    return out


def text_of(row):
    return ''.join(c if c else '\u200b' for c, _ in row)


def normalise(row, w, rules=None):
    for pat, rep in (rules if rules is not None else NORMALISE):
        text = text_of(row)
        out, pos = [], 0
        for m in re.finditer(pat, text):
            out.extend(row[pos:m.start()])
            style = row[m.start()][1] if m.start() < len(row) else BLANK[1]
            out.extend((ch, style) for ch in m.expand(rep))
            pos = m.end()
        if pos:
            out.extend(row[pos:])
            row = (out + [BLANK] * w)[:w]
    return row


def cell_key(cell):
    ch, key = cell
    return (ch, visible(key, ch))


def compare(ref_path, look_path, scene, extra):
    size = ref_path.rsplit('-', 1)[1][:-5]
    full = '%s-%s' % (scene, size)
    applies = lambda g: fnmatch.fnmatch(scene, g) or fnmatch.fnmatch(full, g)
    w, h = (int(x) for x in size.split('x'))
    ref_rules = [(p, r.replace('{VERSION}', VERSION)) for p, r in REF_ONLY] + NORMALISE
    ref = [normalise(r, w, ref_rules) for r in parse(open(ref_path, encoding='utf-8').read(), w, h)]
    look = [normalise(r, w) for r in parse(open(look_path, encoding='utf-8').read(), w, h)]
    masked = [[None] * w for _ in range(h)]
    for glob_, side, pat, reason in extra:
        # drop: `anchor@@row`. Rows of the reference matching `row` are features the look does
        # not have; they are taken out and what was above them, down to the first row matching
        # `anchor`, moves down, the way a bottom-anchored panel a row shorter would sit.
        if side != 'drop' or not applies(glob_):
            continue
        # An anchor starting with `>` is the last row matching it, not the first.
        anchor, rowpat = pat.split('@@')[:2]
        last = anchor.startswith('>')
        hits = [y for y, r in enumerate(ref) if re.search(anchor.lstrip('>'), text_of(r))]
        if not hits:
            continue
        top = hits[-1] if last else hits[0]
        for y in [y for y, r in enumerate(ref) if y > top and re.search(rowpat, text_of(r))]:
            del ref[y]
            ref.insert(top, [BLANK] * w)
            del masked[y]
            masked.insert(top, [reason] * w)
            top += 1
    for glob_, side, pat, reason in extra:
        # squeeze-ref, squeeze-look: rows of that side matching are a field the other side has
        # no counterpart for, in a panel drawn from the top. They are taken out and what was
        # under them moves up.
        if not side.startswith('squeeze-') or not applies(glob_):
            continue
        grid = ref if side == 'squeeze-ref' else look
        rx = re.compile(pat)
        gone = [y for y, r in enumerate(grid) if rx.search(text_of(r))]
        for y in reversed(gone):
            del grid[y]
            grid.append([BLANK] * w)
        for y in range(h - len(gone), h):
            masked[y] = [reason] * w
    for glob_, side, pat, reason in extra:
        # cut: `N`. The reference panel was N rows taller than the screen lets a panel be, so
        # it lost its last N rows at the bottom and sits N rows lower than it would whole. Once
        # `drop` has made it fit, it moves up by N and the rows it lost are masked.
        if side != 'cut' or not applies(glob_):
            continue
        n = int(pat)
        ref = ref[n:] + [[BLANK] * w for _ in range(n)]
        masked = masked[n:] + [[reason] * w for _ in range(n)]
    for glob_, side, pat, reason in EXPLAINED + extra:
        if side in ('drop', 'cut', 'cursor', 'shape') or side.startswith('squeeze-') or not applies(glob_):
            continue
        if side == 'block':
            # block: `start@@end`, every row from one matching start to the next matching end
            start, end = (re.compile(x) for x in pat.split('@@'))
            for grid in (ref, look):
                y0 = next((y for y, r in enumerate(grid) if start.search(text_of(r))), None)
                if y0 is None:
                    continue
                y1 = next((y for y in range(y0, h) if end.search(text_of(grid[y]))), h - 1)
                for y in range(y0, y1 + 1):
                    for x in range(w):
                        masked[y][x] = reason
            continue
        to_end = pat.endswith('$$')
        rx = re.compile(pat[:-2] if to_end else pat)
        sides = {'ref': [ref], 'look': [look], 'both': [ref, look]}[side]
        for grid in sides:
            for y, row in enumerate(grid):
                for m in rx.finditer(text_of(row)):
                    for x in range(m.start(), w if to_end else max(m.end(), m.start() + 1)):
                        if x < w:
                            masked[y][x] = reason
    bad, explained, lines = 0, {}, []
    for y in range(h):
        diffs = []
        for x in range(w):
            if cell_key(ref[y][x]) != cell_key(look[y][x]):
                if masked[y][x]:
                    explained[masked[y][x]] = explained.get(masked[y][x], 0) + 1
                else:
                    diffs.append(x)
        if diffs:
            bad += len(diffs)
            lines.append('row %d: %d cells, first at col %d' % (y, len(diffs), diffs[0]))
            lines.append('  ref  |%s' % text_of(ref[y]).rstrip())
            lines.append('  look |%s' % text_of(look[y]).rstrip())
            x = diffs[0]
            lines.append('  col %d: ref %r %s / look %r %s' % (
                x, ref[y][x][0], fmt(visible(ref[y][x][1], ref[y][x][0])),
                look[y][x][0], fmt(visible(look[y][x][1], look[y][x][0]))))
    rm, lm = (open(p[:-5] + '.meta').read().strip() if os.path.exists(p[:-5] + '.meta') else ''
              for p in (ref_path, look_path))
    hidden = 'visible=0'
    if hidden in rm and hidden in lm:
        rm, lm = (re.sub(r'cursor=\S+ ', '', m) for m in (rm, lm))
    if rm != lm:
        reason = next((r for g, s, p, r in extra if s == 'cursor' and applies(g)), None)
        if reason:
            explained[reason] = explained.get(reason, 0) + 1
        else:
            bad += 1
            lines.append('cursor: ref %s / look %s' % (rm, lm))
    return bad, explained, lines, ref, look


# the brand mark and the header's model line differ by design in every scene
SHAPE_MASKS = [r'^[ ▐▛█▜▝▀▌▘▟▙]{9}  \S.*$']
NOTICE = re.compile(r' ?[○◐●◉] \w+ · /effort ?|ctrl\+g to edit in \S+|Esc again to clear')


def fields(row):
    """A row's shape: where each field starts and how it is drawn. A field is a run of cells
    with no two blanks in a row; its text does not count."""
    out, x, w = [], 0, len(row)
    while x < w:
        if row[x][0] == ' ':
            x += 1
            continue
        start = x
        while x < w and not (row[x][0] == ' ' and (x + 1 >= w or row[x + 1][0] == ' ')):
            x += 1
        out.append((start, visible(row[start][1], row[start][0])))
    return out


def shape(ref_path, look_path, scene, extra):
    """Rows whose field layout differs. Used where the look is fed Wizard's own data, so the
    words cannot match but the columns, colours and rows they sit in should. A row matching a
    `shape` rule in explained.tsv on either side is left out: its layout follows from the data
    (more commands, longer names), and the rule says how."""
    size = ref_path.rsplit('-', 1)[1][:-5]
    full = '%s-%s' % (scene, size)
    rules = [(re.compile(p), r) for g, s, p, r in extra
             if s == 'shape' and (fnmatch.fnmatch(scene, g) or fnmatch.fnmatch(full, g))]
    w, h = (int(x) for x in size.split('x'))
    grids = []
    for path in (ref_path, look_path):
        grid = []
        for r in parse(open(path, encoding='utf-8').read(), w, h):
            r = normalise(r, w)
            for pat in SHAPE_MASKS:
                for m in re.finditer(pat, text_of(r)):
                    for x in range(m.start(), min(m.end(), w)):
                        r[x] = BLANK
            for m in NOTICE.finditer(text_of(r)):
                for x in range(m.start(), min(m.end(), w)):
                    r[x] = BLANK
            grid.append(r)
        grids.append(grid)
    # rows only the reference has come out, as in the cell diff
    for g, side, pat, reason in extra:
        if side == 'drop' and (fnmatch.fnmatch(scene, g) or fnmatch.fnmatch(full, g)):
            anchor, rowpat = pat.split('@@')[:2]
            ref = grids[0]
            hits = [y for y, r in enumerate(ref) if re.search(anchor.lstrip('>'), text_of(r))]
            if not hits:
                continue
            top = hits[-1] if anchor.startswith('>') else hits[0]
            for y in [y for y, r in enumerate(ref) if y > top and re.search(rowpat, text_of(r))]:
                del ref[y]
                ref.insert(top, [BLANK] * w)
                top += 1
    # fields only one side's panel has come out, as in the cell diff
    for g, side, pat, reason in extra:
        if side.startswith('squeeze-') and (fnmatch.fnmatch(scene, g) or fnmatch.fnmatch(full, g)):
            grid = grids[0] if side == 'squeeze-ref' else grids[1]
            rx = re.compile(pat)
            keep = [r for r in grid if not rx.search(text_of(r))]
            grid[:] = keep + [[BLANK] * w for _ in range(h - len(keep))]
    explained = {}
    bad = []
    for y in range(h):
        if fields(grids[0][y]) == fields(grids[1][y]):
            continue
        why = next((r for rx, r in rules
                    if rx.search(text_of(grids[0][y])) or rx.search(text_of(grids[1][y]))), None)
        if why:
            explained[why] = explained.get(why, 0) + 1
        else:
            bad.append(y)
    lines = []
    for y in bad:
        lines.append('row %d' % y)
        lines.append('  ref  |%s' % text_of(grids[0][y]).rstrip())
        lines.append('  look |%s' % text_of(grids[1][y]).rstrip())
    return len(bad), explained, lines


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('ref')
    ap.add_argument('look')
    ap.add_argument('--report')
    ap.add_argument('--sizes', default='80x24,120x40,200x50')
    ap.add_argument('--version', required=True, help="the wizard version the replay reports, which the look must show")
    ap.add_argument('--rules', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), 'explained.tsv'))
    a = ap.parse_args()
    global VERSION
    VERSION = a.version
    sizes = a.sizes.split(',')
    extra = load_rules(a.rules)
    scenes = sorted({os.path.basename(p).rsplit('-', 1)[0] for p in glob.glob(os.path.join(a.ref, '*.ansi'))})
    if a.report:
        os.makedirs(a.report, exist_ok=True)
    total, reasons = 0, {}
    print('Claude data (the replay hands the look what the reference showed), cells that differ')
    print('%-28s %s' % ('scene', ' '.join('%14s' % s for s in sizes)))
    skipped = []
    for scene in scenes:
        why = next((r for g, r in NOT_COMPARED if fnmatch.fnmatch(scene, g)), None)
        if why:
            skipped.append((scene, why))
            continue
        cols = []
        for size in sizes:
            rp = os.path.join(a.ref, '%s-%s.ansi' % (scene, size))
            lp = os.path.join(a.look, '%s-%s.ansi' % (scene, size))
            if not os.path.exists(rp):
                cols.append('-')
                continue
            if not os.path.exists(lp):
                cols.append('missing')
                total += 1
                continue
            bad, explained, lines, ref, look = compare(rp, lp, scene, extra)
            total += bad
            ex = sum(explained.values())
            for k, v in explained.items():
                reasons[k] = reasons.get(k, 0) + v
            cols.append('%d (+%d)' % (bad, ex) if ex else str(bad))
            if a.report:
                w = int(size.split('x')[0])
                with open(os.path.join(a.report, '%s-%s.txt' % (scene, size)), 'w', encoding='utf-8') as f:
                    f.write('%s %s: %d mismatched cells, %d explained\n\n' % (scene, size, bad, ex))
                    f.write('\n'.join(lines) + '\n\n')
                    for k, v in sorted(explained.items()):
                        f.write('explained %5d  %s\n' % (v, k))
                    f.write('\nside by side (reference | look), after normalising\n')
                    for r, l in zip(ref, look):
                        mark = ' ' if [cell_key(c) for c in r] == [cell_key(c) for c in l] else '!'
                        f.write('%s %s | %s\n' % (mark, text_of(r).ljust(w), text_of(l).rstrip()))
        print('%-28s %s' % (scene, ' '.join('%14s' % c for c in cols)))
    wz = sorted({os.path.basename(p).rsplit('-', 1)[0] for p in glob.glob(os.path.join(a.look, 'wz-*.ansi'))})
    if wz:
        print('\nWizard data, rows whose layout differs from the reference scene of the same name')
        print('%-28s %s' % ('scene', ' '.join('%14s' % s for s in sizes)))
    shape_total = 0
    for scene in wz:
        cols = []
        for size in sizes:
            rp = os.path.join(a.ref, '%s-%s.ansi' % (scene[3:], size))
            lp = os.path.join(a.look, '%s-%s.ansi' % (scene, size))
            if not (os.path.exists(rp) and os.path.exists(lp)):
                cols.append('-')
                continue
            bad, explained, lines = shape(rp, lp, scene, extra)
            shape_total += bad
            ex = sum(explained.values())
            for k, v in explained.items():
                reasons[k] = reasons.get(k, 0) + v
            cols.append('%d (+%d)' % (bad, ex) if ex else str(bad))
            if a.report:
                with open(os.path.join(a.report, '%s-%s.txt' % (scene, size)), 'w', encoding='utf-8') as f:
                    f.write('%s %s: %d rows differ in layout\n\n%s\n' % (scene, size, bad, '\n'.join(lines)))
        print('%-28s %s' % (scene, ' '.join('%14s' % c for c in cols)))
    if skipped:
        print('\nnot compared (Claude data for a dialog Wizard never raises):')
        for scene, why in skipped:
            print('  %-26s %s' % (scene, why))
    print('\nunexplained mismatched cells: %d' % total)
    if wz:
        print('rows laid out differently with Wizard data, not explained by the data: %d' % shape_total)
    if reasons:
        print('explained (cells left out of the count):')
        for k, v in sorted(reasons.items(), key=lambda kv: -kv[1]):
            print('  %6d  %s' % (v, k))
    sys.exit(1 if total or shape_total else 0)


if __name__ == '__main__':
    main()
