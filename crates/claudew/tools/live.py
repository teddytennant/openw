#!/usr/bin/env python3
"""Drive real `wizard` (skin claude, so it starts wizard-ui-claude) against tools/fake_openai.py
and check what only shows with Wizard's own data.

    live.py --wizard BIN --look BIN --cwd DIR --work DIR [--sizes 80x24,120x40]

Two throwaway homes under WORK: one whose provider is the fake server, one with no working
provider at all. Nothing under the real ~/.wizard is read or written. Captures go to
WORK/live/<scene>-<WxH>.ansi; every failed check is printed, and the exit status is 1 if any.
tmux runs on the socket `lookclaude` only.
"""
import argparse, json, os, re, shutil, socket, subprocess, sys, time

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
            other = os.path.join(a.work, 'proj2')
            os.makedirs(other, exist_ok=True)
            for f in ('w.txt',):
                if os.path.exists(os.path.join(a.cwd, f)):
                    os.remove(os.path.join(a.cwd, f))
            for group, h, cwd in (('live', good, a.cwd), ('live-other', good, other),
                                  ('live-resumed', good, a.cwd), ('live-fail', bad, a.cwd)):
                subprocess.run([sys.executable, os.path.join(HERE, 'drive.py'), '--target', 'look',
                                '--cwd', cwd, '--out', out, '--group', group, '--sizes', size,
                                '--cmd', '%s HOME=%s wizard' % (env, h)], check=True,
                               stdout=subprocess.DEVNULL)
            history = os.path.join(a.work, 'live-state', 'history.jsonl')
            fails += checks(out, size, version, log, a.cwd, history)
            # the next size starts from a fresh history and fresh sessions
            for d in ('live-state',):
                shutil.rmtree(os.path.join(a.work, d), ignore_errors=True)
            shutil.rmtree(os.path.join(good, '.wizard', 'sessions'), ignore_errors=True)
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


def requests(log, word):
    """What the fake model was offered for each prompt ending in `word`."""
    return [json.loads(l)['tools'] for l in open(log)
            if json.loads(l).get('last_role') == 'user' and json.loads(l).get('user', '').endswith(word)]


def checks(out, size, version, log, cwd, history):
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
    check(fails, tag('live-queue'), 'ctrl+enter to send now' in t, 'the send-now hint is shown')
    check(fails, tag('live-queue'), 'Press up to edit queued messages' in t, 'the queue placeholder')
    t = text(p('live-queue-esc'))
    check(fails, tag('live-queue'), re.search(r'^❯ queued one', t, re.M) and 'ok' in t.split('queued one')[-1],
          'after Esc the queued prompt is sent and answered')

    t = text(p('live-badedit')).split('please badedit')[-1]
    check(fails, tag('live-badedit'), re.search(r'Error|not found|not in the file', t),
          'a failed edit shows as failed')
    check(fails, tag('live-badedit'), not re.search(r'^\s+\d+ \+\s*x\s*$', t, re.M),
          'a failed edit draws no diff of a change that never happened')

    t = text(p('live-plan-keep-ask'))
    check(fails, tag('live-plan-keep'), 'Ready to code?' in t and 'Would you like to' in t,
          'a finished plan asks for approval')
    check(fails, tag('live-plan-keep'), '// entry point' in t, 'the plan text is shown')
    check(fails, tag('live-plan-keep'), '❯ 2. No, keep planning' in text(p('live-plan-keep-down')),
          'Down moves to keep planning')
    for scene in ('live-plan-keep', 'live-plan-esc'):
        t = text(p(scene)).split('please plan')[-1]
        check(fails, tag(scene), 'Ready to code?' not in t and 'plan mode on' in t,
              'the answer closes the pane and plan mode stays on')
        check(fails, tag(scene), "User rejected Wizard's plan:" in t and '╭' in t and '// entry point' in t,
              'the rejected plan stays in the transcript, boxed')
        check(fails, tag(scene), 'Used 1 tool' not in t and 'Interrupted' not in t,
              'no tool fold and no interruption for an answered plan')
        check(fails, tag(scene), re.search(r'for \d+s · done', t), 'the turn closes with its done line')
    t = text(p('live-plan-approve')).split('please plan')[-1]
    check(fails, tag('live-plan-approve'), "User approved Wizard's plan" in t and '// entry point' in t,
          'the approved plan stays in the transcript')
    check(fails, tag('live-plan-approve'), 'plan mode on' not in t, 'approving leaves plan mode')

    check(fails, tag('live-plan-tab'), 'plan mode on' in text(p('live-plan-tab-footer')),
          'shift+tab reaches plan mode')
    check(fails, tag('live-plan-tab'), 'Ready to code?' in text(p('live-plan-tab')),
          'plan reached by shift+tab can finish a plan')
    offered = requests(log, 'please plan')[-1]
    check(fails, tag('live-plan-tab'), 'read_file' in offered and 'exit_plan' in offered,
          'plan mode reached by shift+tab offers read tools and exit_plan (%s)' % offered)

    before = footer_mode(text(p('live-queue-mode')))
    after = footer_mode(text(p('live-queue-mode-done')))
    check(fails, tag('live-queue-mode'), before == 'genie',
          'while the turn runs the footer keeps the confirmed mode (%r)' % before)
    check(fails, tag('live-queue-mode'), after == 'chat', 'then shows the mode wizard switched to (%r)' % after)
    offered = requests(log, 'please write')[-1]
    check(fails, tag('live-queue-mode'), sorted(offered) == ['web_fetch', 'web_search'],
          'the queued prompt ran in the mode changed before it (offered %s)' % offered)
    check(fails, tag('live-queue-mode'), not os.path.exists(os.path.join(cwd, 'w.txt')),
          'nothing was written from chat mode')

    t = text(p('live-modes-dismissed'))
    rows = re.findall(r'^❯(.*)$', t, re.M)
    check(fails, tag('live-modes'), rows and rows[-1].strip() == '',
          'after a local command the prompt has no placeholder (%r)' % rows[-1:])

    t = text(p('live-rewind'))
    check(fails, tag('live-rewind'),
          'Confirm you want to restore to the point before you sent this message:' in t
          and '1. Restore conversation' in t and '2. Never mind' in t,
          'Enter on a prompt asks to confirm the rewind')
    check(fails, tag('live-rewind'), 'Restore the code and/or conversation' in text(p('live-rewind-back')),
          'Esc goes back to the list')

    t = ''.join(text(p('live-resumed-up%d' % n)) for n in range(1, 13)) + text(p('live-resumed'))
    check(fails, tag('live-resumed'), re.search(r'^\s+2 -\s+println!\("hello"\);', t, re.M)
          and re.search(r'^\s+2 \+\s+println!\("hi"\);', t, re.M),
          'a resumed edit shows real line numbers')
    check(fails, tag('live-resumed'), re.search(r'Error|not found|not in the file', t),
          'a resumed failed edit shows as failed')

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

    here = [e['text'] for e in map(json.loads, open(history)) if e.get('cwd') == cwd]
    for n, scene in ((1, 'live-fail-up'), (2, 'live-fail-up2')):
        t = text(p(scene))
        rows = re.findall(r'^❯\s+(.*?)\s*$', t, re.M)
        want = here[-n] if len(here) >= n else None
        check(fails, tag('live-fail'), rows and rows[-1] == want,
              'Up %d recalls this project\'s prompt %r, not another project\'s (%r)' % (n, want, rows[-1:]))
        check(fails, tag('live-fail'), 'History %d/%d' % (len(here) + 1 - n, len(here)) in t,
              'the counter counts this project\'s prompts')
        check(fails, tag('live-fail'), 'Resume a previous' not in t and 'say other' not in t,
              'a recalled slash entry opens no menu')

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
