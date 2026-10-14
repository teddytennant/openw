#!/usr/bin/env python3
"""Drive real `wizard` (skin claude, so it starts wizard-ui-claude) against tools/fake_openai.py
and check what only shows with Wizard's own data.

    live.py --wizard BIN --look BIN --cwd DIR --work DIR [--sizes 80x24,120x40]

Two throwaway homes under WORK: one whose provider is the fake server, one with no working
provider at all. Nothing under the real ~/.wizard is read or written. Captures go to
WORK/live/<scene>-<WxH>.ansi; every failed check is printed, and the exit status is 1 if any.
tmux runs on the socket `lookclaude` only.
"""
import argparse, json, os, re, socket, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
SGR = re.compile(r'\x1b\[[0-9;:]*m')


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def home(path, provider):
    os.makedirs(os.path.join(path, '.wizard'), exist_ok=True)
    with open(os.path.join(path, '.wizard', 'config.toml'), 'w') as f:
        f.write(provider + '\n[ui]\nskin = "claude"\n\n[update]\nnotify = false\nauto = false\n')


def text(path):
    with open(path, encoding='utf-8') as f:
        return SGR.sub('', f.read())


def check(fails, scene, ok, what):
    if not ok:
        fails.append('%s: %s' % (scene, what))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--wizard', required=True)
    ap.add_argument('--look', required=True)
    ap.add_argument('--cwd', required=True)
    ap.add_argument('--work', required=True)
    ap.add_argument('--sizes', default='120x40')
    a = ap.parse_args()
    out = os.path.join(a.work, 'live')
    os.makedirs(out, exist_ok=True)
    bindir = os.path.join(a.work, 'live-bin')
    os.makedirs(bindir, exist_ok=True)
    for name, target in (('wizard', a.wizard), ('wizard-ui-claude', a.look)):
        link = os.path.join(bindir, name)
        if os.path.lexists(link):
            os.remove(link)
        os.symlink(os.path.abspath(target), link)
    version = subprocess.run([a.wizard, '--version'], capture_output=True, text=True).stdout.split()
    version = version[-1] if version else ''
    port = free_port()
    log = os.path.join(a.work, 'fake.log')
    fake = subprocess.Popen([sys.executable, os.path.join(HERE, 'fake_openai.py'), str(port), a.cwd, log],
                            stderr=subprocess.DEVNULL)
    good = os.path.join(a.work, 'live-home')
    bad = os.path.join(a.work, 'live-home-broken')
    home(good, 'model = "fake-model"\nactive_provider = "fake"\n\n[[providers]]\nname = "fake"\n'
               'kind = "openai"\nbase_url = "http://127.0.0.1:%d/v1"\nmodel = "fake-model"\n' % port)
    # a provider whose key is never set
    home(bad, 'model = "grok-4"\nactive_provider = "xai"\n\n[[providers]]\nname = "xai"\nkind = "xai"\n'
              'base_url = "http://127.0.0.1:9/v1"\nmodel = "grok-4"\napi_key_env = "CLAUDEW_LIVE_NO_KEY"\n')
    env = 'env -u XAI_API_KEY WIZARD_DEV_SKILLS=/nonexistent CLAUDEW_STATE_DIR=%s PATH=%s:$PATH' % (
        os.path.join(a.work, 'live-state'), bindir)
    fails = []
    try:
        for size in a.sizes.split(','):
            main_rs = 'fn main() {\n    println!("hello");\n}\n'
            with open(os.path.join(a.cwd, 'main.rs'), 'w') as f:
                f.write(main_rs)
            for group, h in (('live', good), ('live-fail', bad)):
                subprocess.run([sys.executable, os.path.join(HERE, 'drive.py'), '--target', 'look',
                                '--cwd', a.cwd, '--out', out, '--group', group, '--sizes', size,
                                '--cmd', '%s HOME=%s wizard' % (env, h)], check=True,
                               stdout=subprocess.DEVNULL)
            fails += checks(out, size, version, log)
    finally:
        fake.kill()
        fake.wait()
    for f in fails:
        print('live check failed: ' + f)
    print('live checks: %d failed' % len(fails))
    sys.exit(1 if fails else 0)


def footer_mode(t):
    rows = [r for r in t.split('\n') if r.strip()]
    m = re.search(r'([a-z][\w ]*?) (?:mode )?on\b', rows[-1]) if rows else None
    return m.group(1) if m else ''


def checks(out, size, version, log):
    fails = []
    p = lambda scene: os.path.join(out, '%s-%s.ansi' % (scene, size))
    tag = lambda scene: '%s %s' % (scene, size)

    t = text(p('live-home'))
    check(fails, tag('live-home'), 'Wizard v%s' % version in t,
          'the header names wizard %s, the real binary' % version)

    t = text(p('live-modes-1'))
    check(fails, tag('live-modes'), 'Options can only be changed' not in t,
          'cycling modes prints nothing')
    mode = footer_mode(t)
    s = text(p('live-modes-status'))
    m = re.search(r'Mode:\s+(\S+)', s)
    check(fails, tag('live-modes'), m is not None and m.group(1) == mode,
          'the footer (%r) and /status (%r) agree after a fast cycle' % (mode, m and m.group(1)))
    check(fails, tag('live-modes'), footer_mode(text(p('live-modes-turn'))) == mode,
          'a turn does not move the mode')
    # what the fake model was offered on that turn says which mode wizard was really in
    offered = [json.loads(l) for l in open(log) if '"say ok"' in l][-1]['tools']
    check(fails, tag('live-modes'), (sorted(offered) == ['web_fetch', 'web_search']) == (mode == 'chat'),
          'wizard ran the turn in the mode the footer shows (%r, offered %s)' % (mode, offered))

    t = text(p('live-edit'))
    check(fails, tag('live-edit'), re.search(r'^\s+1\s+fn main\(\) \{', t, re.M),
          'the edit diff shows the line before the change, numbered 1')
    check(fails, tag('live-edit'), re.search(r'^\s+2 -\s+println!\("hello"\);', t, re.M),
          'the removed line is numbered 2')
    check(fails, tag('live-edit'), re.search(r'^\s+2 \+\s+println!\("hi"\);', t, re.M),
          'the added line is numbered 2')
    check(fails, tag('live-edit'), re.search(r'^\s+3\s+\}', t, re.M),
          'the line after the change is shown, numbered 3')

    t = text(p('live-interrupt')).split('❯ please slow')[-1]
    check(fails, tag('live-interrupt'), 'Interrupted' in t, 'the interrupt is shown')
    check(fails, tag('live-interrupt'), not re.search(r'for \d+s · done', t),
          'no done line after an interrupt with no tool call')

    t = text(p('live-queue'))
    check(fails, tag('live-queue'), re.search(r'^❯ queued one', t, re.M), 'the queued prompt is shown')
    check(fails, tag('live-queue'), 'ctrl+x ctrl+s to send now' in t, 'the send-now hint is shown')
    check(fails, tag('live-queue'), 'Press up to edit queued messages' in t, 'the queue placeholder')
    t = text(p('live-queue-esc'))
    check(fails, tag('live-queue'), re.search(r'^❯ queued one', t, re.M) and 'ok' in t.split('queued one')[-1],
          'after Esc the queued prompt is sent and answered')

    t = text(p('live-plan'))
    check(fails, tag('live-plan'), 'Ready to code?' in t and 'Would you like to' in t,
          'a finished plan asks for approval')
    check(fails, tag('live-plan'), '// entry point' in t, 'the plan text is shown')
    check(fails, tag('live-plan'), '❯ 2. No, keep planning' in text(p('live-plan-down')),
          'Down moves to keep planning')
    t = text(p('live-plan-kept'))
    check(fails, tag('live-plan'), 'Ready to code?' not in t and 'plan mode on' in t,
          'choosing No closes the prompt and stays in plan mode')

    t = text(p('live-resume'))
    ages = re.findall(r'(\d+) (second|minute|hour)s? ago · \S+ · [0-9.]+[KMG]?B', t)
    check(fails, tag('live-resume'), ages, 'sessions show their age, branch and size')
    # the session was last written by /status, a few seconds before the picker opened
    check(fails, tag('live-resume'), any(u != 'second' or int(n) >= 3 for n, u in ages),
          'the clock moves: the session is older than a second or two (%s)' % ages)

    t = text(p('live-search'))
    check(fails, tag('live-search'), 'Search prompts · everywhere' in t and 'ctrl+s to scope' in t,
          'history search covers every session and offers ctrl+s')
    check(fails, tag('live-search'), re.search(r'\b[1-9]\d*[smhd] ago', t),
          'earlier prompts are not all 0s ago')

    t = text(p('live-fail-up'))
    rows = re.findall(r'^❯\s+(.*?)\s*$', t, re.M)
    check(fails, tag('live-fail'), rows and rows[-1] == '/resume',
          'Up in a new session recalls the last prompt of the session before (%r)' % rows[-1:])

    t = text(p('live-exit'))
    check(fails, tag('live-exit'), 'wizard --resume' in t, 'the exit names what the user runs')

    t = text(p('live-fail'))
    check(fails, tag('live-fail'), 'wizard setup' in t, 'the setup hint shows')
    check(fails, tag('live-fail'), 'v0.1.0' not in t and 'Wizard v%s' % version in t,
          'the header names the real wizard, not the look crate')
    t = text(p('live-fail-typed'))
    check(fails, tag('live-fail'), not re.search(r'esc to interrupt|…\s*$', t, re.M),
          'nothing looks busy after a failed start')
    meta = open(os.path.join(out, 'live-fail-exit-%s.meta' % size)).read()
    check(fails, tag('live-fail'), 'alt=0' in meta, 'ctrl+c twice exits')
    return fails


if __name__ == '__main__':
    main()
