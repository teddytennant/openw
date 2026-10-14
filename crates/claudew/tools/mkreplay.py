#!/usr/bin/env python3
"""Write the replay scripts the look is driven with, from the reference captures.

    mkreplay.py REF_DIR CWD OUT_DIR

The slash commands, models and saved sessions are read off the 200x50 captures so the look is
handed the same data the reference showed. The turn script repeats the two tiny turns the
reference was put through: `echo hi`, then an edit of main.rs behind a permission prompt.
"""
import json, os, re, sys, time

ANSI = re.compile(r'\x1b\[[0-9;]*m')


def rows(ref, name, size='200x50'):
    path = os.path.join(ref, '%s-%s.ansi' % (name, size))
    if not os.path.exists(path):
        return []
    return [ANSI.sub('', l).rstrip() for l in open(path, encoding='utf-8').read().split('\n')]


SIZES = ('200x50', '120x40', '80x24')


def fill(desc):
    """A description cut with an ellipsis gets more words so it is cut again."""
    return desc[:-1] + 'x and the rest of the description goes on' if desc.endswith('…') else desc


def menu_commands(ref, name):
    """Commands a menu capture shows, from every size; the widest capture's text wins."""
    seen = {}
    order = []
    for size in SIZES:
        for n, d in menu_commands_at(ref, name, size):
            if n not in seen:
                order.append(n)
                seen[n] = d
    return [(n, seen[n]) for n in order]


def menu_commands_at(ref, name, size):
    out = []
    for line in rows(ref, name, size):
        m = re.match(r'^  [❯ ] /(\S+)\s+(.*)$', line)
        if m:
            out.append([m.group(1), m.group(2)])
        elif out and re.match(r'^ {30,}\S', line):
            out[-1][1] += ' ' + line.strip()
        elif line.startswith('─'):
            break
    return [(n, fill(d)) for n, d in out]


def help_commands(ref):
    seen, out = set(), []
    for size in SIZES:
        lines = rows(ref, 'help-commands', size)
        for i, line in enumerate(lines):
            m = re.match(r'^   [❯ ] /(\S+)$', line)
            if m and i + 1 < len(lines) and m.group(1) not in seen:
                seen.add(m.group(1))
                out.append((m.group(1), fill(lines[i + 1].strip())))
    return out


def models(ref):
    out = []
    for line in rows(ref, 'model'):
        m = re.match(r'^\s+[❯↓ ]\s*\d+\.\s+(.+?)\s{2,}(.*)$', line)
        if m:
            out.append((m.group(1).replace(' ✔', ''), m.group(2), '✔' in m.group(1)))
    return out


def sessions(ref, cwd, now, size):
    out, lines = [], rows(ref, 'resume', size)
    unit = {'second': 1, 'minute': 60, 'hour': 3600, 'day': 86400}
    for i, line in enumerate(lines[:-1]):
        m = re.match(r'^     (\d+) (second|minute|hour|day)s? ago', lines[i + 1])
        if m and re.match(r'^   [❯↓↑ ] \S', line):
            title = line[5:].strip()
            is_id = re.fullmatch(r'[0-9a-f]{8}', title) is not None
            out.append({
                'id': (title if is_id else '%08x' % (i * 2654435761 % 2**32)) + '-0000-4000-8000-000000000000',
                'title': '' if is_id else title,
                'cwd': cwd,
                'updated': now - int(m.group(1)) * unit[m.group(2)] - 2,
            })
    # older sessions the window never reaches, so the count and the arrow show as they do there
    for n in range(24):
        out.append({'id': '%08x-0000-4000-8000-000000000000' % (0x5e55 + n), 'title': 'older session %d' % n,
                    'cwd': cwd, 'updated': now - 40 * 86400 - n})
    return out


def ev(e):
    return json.dumps({'event': e}, ensure_ascii=False)


def head(ref, cwd, size):
    seen, cmds = set(), []
    # the two entries after /dataviz are never on screen, but their two-row height decides
    # whether anything is drawn under it
    after = [('workflow-authoring', fill('Reference for writing a Workflow tool script (script API and gotchas, resume, quality patterns, worked examples). Load before authoring a script for a workflow…')),
             ('update-config', fill('Use this skill to configure the harness via settings.json. Automated behaviors ("from now on when X", "each time X", "whenever X", "before/after X") require hooks configured in settings…'))]
    lists = [menu_commands(ref, 'slash'), after] + [menu_commands(ref, n) for n in ('slash-filter', 'help-typed')] + [help_commands(ref)]
    tail = [('compact', 'Clear conversation history but keep a summary in context. Optional: /compact [instructions for summarization]'),
            ('fewer-permission-prompts', 'Scan your transcripts for common read-only tool calls')]
    for name, desc in [c for l in lists for c in l] + tail:
        if name not in seen:
            seen.add(name)
            cmds.append({'name': name, 'description': desc, 'input_hint': ''})
    ms = models(ref)
    current = next((n for n, _, cur in ms if cur), ms[0][0] if ms else 'model')
    slug = lambda n: 'claude-' + re.sub(r'[^a-z0-9]+', '-', n.lower()).strip('-')
    opts = [{'id': slug(n), 'name': n, 'provider': d} for n, d, _ in ms]
    opts += [{'id': 'extra-%d' % i, 'name': 'Extra %d' % i, 'provider': ''} for i in range(3)]
    config = {
        'model': slug(current), 'models': opts, 'effort': 'medium', 'efforts': ['low', 'medium', 'high'],
        'mode': 'manual', 'modes': ['manual', 'accept edits'], 'cwd': cwd, 'backend': 'replay',
    }
    return [
        ev({'Ready': {'session_id': '0191b6f0-3c1a-7a52-9a0e-6d3e9b2f1c44', 'config': config}}),
        ev({'Commands': cmds}),
        json.dumps({'sessions': sessions(ref, cwd, int(time.time()), size)}, ensure_ascii=False),
    ]


def tool(id_, kind, title, inp, status, output=None, diff=None):
    return ev({'Tool': {'id': id_, 'name': kind, 'kind': kind, 'title': title, 'input': inp,
                        'status': status, 'output': output, 'diff': diff, 'parent_id': None}})


def find_command(ref, cwd, size):
    """The search command the reference ran at this size, read off its detailed transcript."""
    path = os.path.join(ref, 'edit-detail-%s.ansi' % size)
    if os.path.exists(path):
        lines = [ANSI.sub('', l).rstrip() for l in open(path, encoding='utf-8').read().split('\n')]
        for i, line in enumerate(lines):
            if line.startswith('● Bash(find '):
                text = line[len('● Bash('):]
                for more in lines[i + 1:]:
                    if not more.startswith('      '):
                        break
                    text += ' ' + more.strip()
                return text[:-1] if text.endswith(')') else text
    return "find %s -name main.rs -not -path '*/target/*' 2>/dev/null" % cwd


def turn(cwd, find):
    path = cwd + '/main.rs'
    old = 'fn main() {\n    println!("hello");\n}\n'
    new = 'fn main() {\n    println!("hi");\n}\n'
    diff = {'path': path, 'old': old, 'new': new}
    wait = lambda what: json.dumps({'await': what})
    sleep = lambda ms: json.dumps({'sleep': ms})
    end = ev({'TurnEnd': 'end_turn'})
    return [
        wait('prompt'), ev('TurnStart'), sleep(5000),
        tool('t1', 'execute', 'Printing hi', {'command': 'echo hi'}, 'running'), sleep(300),
        tool('t1', 'execute', 'Printing hi', {'command': 'echo hi'}, 'completed', 'hi\n'),
        ev({'TextDelta': 'done'}), end,
        wait('prompt'), ev('TurnStart'), sleep(300),
        tool('t2', 'execute', 'Finding main.rs', {'command': find}, 'completed', path + '\n'),
        tool('t3', 'read', 'main.rs', {'path': path}, 'completed', old),
        tool('t4', 'edit', 'main.rs', {'path': path}, 'pending', None, diff),
        ev({'Permission': {'id': 'p1', 'tool': 'Edit', 'kind': 'edit', 'title': path, 'input': {},
                           'diff': diff, 'rule': 'edits'}}),
        wait('decide'), sleep(200),
        tool('t4', 'edit', 'main.rs', {'path': path}, 'completed', None, diff),
        ev({'TextDelta': 'done'}), end,
        wait('prompt'), ev('TurnStart'), sleep(600),
        ev({'Notice': {'level': 'info', 'text': 'Compacted (ctrl+o to see full summary)'}}), end,
    ]


def main():
    ref, cwd, out = sys.argv[1:4]
    os.makedirs(out, exist_ok=True)
    for size in ('80x24', '120x40', '200x50'):
        h = head(ref, cwd, size)
        open(os.path.join(out, 'static-%s.jsonl' % size), 'w', encoding='utf-8').write('\n'.join(h) + '\n')
        script = h + turn(cwd, find_command(ref, cwd, size))
        open(os.path.join(out, 'turn-%s.jsonl' % size), 'w', encoding='utf-8').write('\n'.join(script) + '\n')


if __name__ == '__main__':
    main()
