#!/usr/bin/env python3
"""Drive Claude Code or wizard-ui-claude through the comparison scenes in a private tmux server.

Every tmux call goes to the socket `lookclaude` (`tmux -L lookclaude`), never the default
server. Each scene ends in one or more captures: `<out>/<scene>-<WxH>.ansi` is
`capture-pane -e -p` (text with SGR), `<out>/<scene>-<WxH>.meta` the cursor cell and whether
the cursor and the alternate screen are on.
"""
import argparse, os, re, subprocess, sys, time

SOCKET = 'lookclaude'
SIZES = ['80x24', '120x40', '200x50']
# NAME=value pairs passed through to the UI under test (LOOK_ENV, space separated)
EXTRA_ENV = os.environ.get('LOOK_ENV', '').split()

P_BASH = 'Run the bash command `echo hi` and then reply with exactly: done'
P_EDIT = 'Read main.rs, then change hello to hi in it, then reply with exactly: done'
LONG = ('this is a long line of input that keeps going so that it has to wrap at the right edge of the '
        'prompt box more than once at eighty columns and at least once at one hundred and twenty')

# (scene name, steps). A step is ('type', text), ('keys', k1, k2, ...), ('wait', seconds),
# ('idle',), ('until', regex, timeout), ('raw_snap', suffix) (no settling), ('fast_keys', ...)
# (no waiting after), ('quit',) (ctrl+c until the alternate screen closes)
# or ('snap', suffix). ('ref_keys', ...) is sent to the
# reference only.
STATIC = [
    ('home', [('snap', '')]),
    ('shortcuts', [('type', '?'), ('snap', ''), ('keys', 'BSpace')]),
    ('multiline', [('type', 'line one\\'), ('keys', 'Enter'), ('type', 'line two'), ('snap', '')]),
    ('ctrlc', [('fast_keys', 'C-c'), ('raw_snap', '-hint'), ('wait', 2.5), ('snap', '-after')]),
    ('bash-mode', [('type', '!'), ('snap', ''), ('type', 'ls'), ('snap', '-typed'),
                   ('keys', 'BSpace', 'BSpace', 'BSpace')]),
    # the reference has a fourth mode (auto) the backend does not; one more press brings it round
    ('modes', [('wait', 1.0), ('keys', 'BTab'), ('snap', '-1'), ('keys', 'BTab'), ('snap', '-2'),
               ('keys', 'BTab'), ('snap', '-3'), ('ref_keys', 'BTab'), ('wait', 1.0),
               ('snap', '-4')]),
    ('mention', [('type', '@'), ('wait', 0.8), ('snap', ''), ('type', 's'), ('snap', '-query'),
                 ('keys', 'Down'), ('snap', '-selected'), ('keys', 'C-c')]),
    ('slash', [('type', '/'), ('snap', ''), ('keys', 'Down', 'Down'), ('snap', '-down'),
               ('type', 'mo'), ('snap', '-filter'), ('type', 'zzz'), ('snap', '-none'),
               ('keys', 'C-c')]),
    # the panel takes a few seconds to take keys after it is drawn; Tab sent sooner is lost
    ('help', [('type', '/help'), ('snap', '-typed'), ('keys', 'Enter'),
              ('until', r'For more help', 15), ('wait', 3.0), ('snap', ''),
              ('keys', 'Tab'), ('wait', 3.0), ('snap', '-commands'), ('keys', 'Down', 'Down'),
              ('snap', '-commands-down'), ('keys', 'Escape'), ('wait', 0.5)]),
    ('model', [('type', '/model'), ('keys', 'Enter'), ('until', r'Select model', 15),
               ('wait', 0.5), ('snap', ''), ('keys', 'Down'), ('snap', '-down'),
               ('keys', 'Escape'), ('wait', 1.5), ('snap', '-kept')]),
    ('status', [('type', '/status'), ('keys', 'Enter'), ('until', r'Esc to cancel', 15),
                ('wait', 0.8), ('snap', ''), ('keys', 'Escape'), ('wait', 0.5)]),
    ('resume', [('type', '/resume'), ('keys', 'Enter'), ('until', r'Resume session', 15),
                ('wait', 1.0), ('snap', ''), ('keys', 'Down'), ('snap', '-down'),
                ('keys', 'Escape'), ('wait', 0.5)]),
    ('tab', [('type', '/mo'), ('keys', 'Tab'), ('snap', ''), ('keys', 'C-c'), ('wait', 1.0)]),
    ('wrap', [('type', LONG), ('snap', ''), ('keys', 'C-c')]),
    # a double press is two within 800 ms; the hint snap takes longer than that
    ('escape', [('type', 'abc'), ('fast_keys', 'Escape'), ('raw_snap', '-hint'), ('wait', 1.5),
                ('keys', 'Escape', 'Escape'), ('snap', '-cleared')]),
    ('resize', [('resize', 100, 30), ('wait', 1.0), ('snap', '')]),
    ('exit', [('quit',), ('wait', 0.5), ('snap', '')]),
]

TURN = [
    ('turn', [('type', P_BASH), ('snap', '-typed'), ('fast_keys', 'Enter'), ('wait', 0.5),
              ('raw_snap', '-spinner'), ('idle',), ('snap', '-done'), ('keys', 'C-o'), ('wait', 0.6),
              ('snap', '-detail'), ('keys', 'C-o'), ('wait', 0.4)]),
    ('edit', [('type', P_EDIT), ('keys', 'Enter'),
              ('until', r'Do you want to make this edit', 60), ('wait', 0.5), ('snap', '-permission'),
              ('keys', 'Down'), ('snap', '-permission-down'), ('keys', 'Up'),
              ('keys', 'Enter'), ('idle',), ('snap', '-done'), ('keys', 'C-o'), ('wait', 0.6),
              ('snap', '-detail'), ('keys', 'C-o'), ('wait', 0.4)]),
    ('history', [('keys', 'Up'), ('snap', '-1'), ('keys', 'Up'), ('snap', '-2'),
                 ('keys', 'Down', 'Down'), ('snap', '-back')]),
    ('compact', [('type', '/compact'), ('keys', 'Enter'), ('wait', 2), ('idle',), ('snap', '')]),
    ('clear', [('type', '/clear'), ('keys', 'Enter'), ('wait', 1.5), ('snap', '')]),
    ('exit-summary', [('quit',), ('wait', 0.5), ('snap', '')]),
]

# The light theme, for the scenes that carry the most colour without a model call.
LIGHT = [
    ('light-home', [('snap', '')]),
    ('light-shortcuts', [('type', '?'), ('snap', ''), ('keys', 'BSpace')]),
    ('light-modes', [('wait', 1.0), ('keys', 'BTab'), ('snap', '-1'), ('keys', 'BTab'), ('snap', '-2'),
                     ('keys', 'BTab'), ('ref_keys', 'BTab'), ('wait', 1.0)]),
    ('light-slash', [('type', '/mo'), ('snap', ''), ('keys', 'C-c'), ('wait', 1.0)]),
    ('light-bash', [('type', '!'), ('snap', ''), ('keys', 'BSpace'), ('wait', 0.5)]),
    ('light-model', [('type', '/model'), ('keys', 'Enter'), ('until', r'Select model', 15),
                     ('wait', 0.5), ('snap', ''), ('keys', 'Escape'), ('wait', 1.5), ('snap', '-kept')]),
]

P_MD = ('No tools. Output exactly this markdown and nothing else: a table |file|lines| with rows '
        '|a.rs|3| and |b.rs|120|, then **`src/`** in bold, then [Docs](https://example.com/docs), '
        'then a fenced rust block containing: fn main() { println!("hi"); }')
P_GIT = 'Run this exact bash command with the Bash tool: git init -q && echo ok. Then reply with exactly: done'
P_WRITE = ('Use the Write tool to create w.txt containing the numbers 1 to 30, one per line. '
           'Then reply with exactly: done')

# A markdown reply and scrolling back over it, a shell command and a new file behind
# permission prompts, one turned down with Esc and one allowed.
TURN2 = [
    ('md', [('type', P_MD), ('keys', 'Enter'), ('idle',), ('snap', ''), ('keys', 'PPage'),
            ('wait', 1.0), ('snap', '-pgup'), ('keys', 'C-End'), ('wait', 1.0)]),
    ('bash', [('type', P_GIT), ('keys', 'Enter'), ('until', r'Do you want to proceed', 90),
              ('wait', 1.0), ('snap', '-permission'), ('keys', 'Escape'), ('idle',), ('snap', '-esc')]),
    ('write', [('type', P_WRITE), ('keys', 'Enter'), ('until', r'Do you want to create', 90),
               ('wait', 1.0), ('snap', '-permission'), ('keys', 'Enter'), ('idle',), ('snap', '-done')]),
]

# The look with Wizard's own models, modes and commands, compared by shape with the
# reference scenes of the same name.
WIZARD = [
    ('wz-home', [('snap', '')]),
    ('wz-slash', [('type', '/'), ('snap', ''), ('keys', 'C-c'), ('wait', 1.0)]),
    ('wz-slash-filter', [('type', '/mo'), ('snap', ''), ('keys', 'C-c'), ('wait', 1.0)]),
    ('wz-modes', [('keys', 'BTab'), ('snap', '-1'), ('keys', 'BTab'), ('snap', '-2'),
                  ('keys', 'BTab'), ('snap', '-3'), ('keys', 'BTab'), ('snap', '-4')]),
    ('wz-model', [('type', '/model'), ('keys', 'Enter'), ('wait', 0.5), ('snap', ''),
                  ('keys', 'Escape'), ('wait', 0.5)]),
    ('wz-status', [('type', '/status'), ('keys', 'Enter'), ('wait', 0.5), ('snap', ''),
                   ('keys', 'Escape'), ('wait', 0.5)]),
    ('wz-help', [('type', '/help'), ('keys', 'Enter'), ('wait', 0.5), ('keys', 'Tab'),
                 ('snap', '-commands'), ('keys', 'Escape'), ('wait', 0.5)]),
]

GROUPS = {'static': STATIC, 'turn': TURN, 'light': LIGHT, 'turn2': TURN2, 'wizard': WIZARD}


def tmux(*args, check=True):
    env = dict(os.environ)
    env.pop('TMUX', None)
    env['TMUX_TMPDIR'] = os.environ.get('LOOK_TMUX_TMPDIR', '/tmp')
    r = subprocess.run(['tmux', '-L', SOCKET, *args], capture_output=True, text=True, env=env)
    if check and r.returncode != 0:
        raise SystemExit('tmux %s: %s' % (' '.join(args), r.stderr.strip()))
    return r.stdout


class Pane:
    def __init__(self, name, w, h, cwd, cmd):
        self.name = name
        tmux('kill-session', '-t', name, check=False)
        tmux('-f', '/dev/null', 'new-session', '-d', '-s', name, '-x', str(w), '-y', str(h), '-c', cwd,
             'env', '-i', 'HOME=' + os.environ['HOME'], 'PATH=' + os.environ['PATH'],
             'USER=' + os.environ.get('USER', ''), 'LANG=en_US.UTF-8', 'TERM=tmux-256color',
             'SHELL=/bin/sh', *EXTRA_ENV, 'sh')
        for opt, val in (('status', 'off'), ('escape-time', '0'), ('focus-events', 'on'),
                         ('mouse', 'on'), ('history-limit', '2000')):
            tmux('set', '-g', opt, val)
        tmux('resize-window', '-t', name, '-x', str(w), '-y', str(h))
        self.send_literal("PS1='$ '; clear; " + cmd)
        self.keys('Enter')

    def changed(self, before, timeout=1.5):
        """Wait until the screen is no longer `before`: a slow UI must take a key before the next."""
        end = time.time() + timeout
        while time.time() < end and self.text() == before:
            time.sleep(0.05)

    def keys(self, *ks):
        for k in ks:
            before = self.text()
            tmux('send-keys', '-t', self.name, k)
            self.changed(before)
            time.sleep(0.12)

    def send_literal(self, text):
        before = self.text()
        tmux('send-keys', '-t', self.name, '-l', text)
        self.changed(before)

    def text(self):
        return tmux('capture-pane', '-t', self.name, '-p')

    def wait_for(self, pattern, timeout):
        end = time.time() + timeout
        while time.time() < end:
            t = self.text()
            if re.search(pattern, t, re.M):
                return t
            time.sleep(0.25)
        raise SystemExit('%s: timed out waiting for %r\n%s' % (self.name, pattern, self.text()))

    def idle(self, timeout=120):
        end = time.time() + timeout
        calm = 0
        while time.time() < end:
            if 'esc to interrupt' in self.text():
                calm = 0
            else:
                calm += 1
                if calm >= 3:
                    return
            time.sleep(0.4)
        raise SystemExit('%s: still busy after %ss' % (self.name, timeout))

    def settle(self, timeout=5.0):
        """Wait until two captures 600 ms apart are the same; a UI may lag behind its input."""
        end = time.time() + timeout
        last = self.text()
        while time.time() < end:
            time.sleep(0.6)
            now = self.text()
            if now == last:
                return
            last = now

    def snap(self, path, settle=True):
        if settle:
            self.settle()
        else:
            time.sleep(0.15)
        with open(path + '.ansi', 'w', encoding='utf-8') as f:
            f.write(tmux('capture-pane', '-t', self.name, '-e', '-p'))
        with open(path + '.meta', 'w') as f:
            f.write(tmux('display', '-p', '-t', self.name,
                         'cursor=#{cursor_x},#{cursor_y} visible=#{cursor_flag} alt=#{alternate_on}'))

    def kill(self):
        tmux('kill-session', '-t', self.name, check=False)


def run(target, cmd, cwd, size, group, out):
    w, h = (int(x) for x in size.split('x'))
    pane = Pane('%s-%s' % (target, size), w, h, cwd, cmd)
    t = pane.wait_for(r'^❯|Yes, I trust this folder', 40)
    if 'Yes, I trust this folder' in t:
        pane.keys('Down', 'Enter')
        pane.wait_for(r'^❯', 40)
    time.sleep(4.0 if target == 'ref' else 0.5)
    for scene, steps in GROUPS[group]:
        for step in steps:
            op = step[0]
            if op == 'type':
                pane.send_literal(step[1])
                time.sleep(0.35)
            elif op == 'fast_keys':
                for k in step[1:]:
                    tmux('send-keys', '-t', pane.name, k)
            elif op == 'keys' or (op == 'ref_keys' and target == 'ref'):
                pane.keys(*step[1:])
                time.sleep(0.3)
            elif op == 'quit':
                # ctrl+c twice, again if the UI is still up (a first press may only clear)
                for _ in range(3):
                    pane.keys('C-c')
                    time.sleep(0.2)
                    pane.keys('C-c')
                    end = time.time() + 5
                    while time.time() < end and tmux('display', '-p', '-t', pane.name,
                                                     '#{alternate_on}').strip() == '1':
                        time.sleep(0.25)
                    if tmux('display', '-p', '-t', pane.name, '#{alternate_on}').strip() != '1':
                        break
            elif op == 'resize':
                tmux('resize-window', '-t', pane.name, '-x', str(step[1]), '-y', str(step[2]))
            elif op == 'wait':
                time.sleep(step[1])
            elif op == 'idle':
                pane.idle()
            elif op == 'until':
                pane.wait_for(step[1], step[2])
            elif op == 'raw_snap':
                pane.snap(os.path.join(out, '%s%s-%s' % (scene, step[1], size)), settle=False)
            elif op == 'snap':
                pane.snap(os.path.join(out, '%s%s-%s' % (scene, step[1], size)))
        time.sleep(0.8)
    pane.kill()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--target', choices=['ref', 'look'], required=True)
    ap.add_argument('--cmd', required=True, help='shell command that starts the UI')
    ap.add_argument('--cwd', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--group', choices=sorted(GROUPS), default='static')
    ap.add_argument('--sizes', default=','.join(SIZES))
    ap.add_argument('--before', default='', help='shell command run before each size (reset files)')
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    for size in a.sizes.split(','):
        if a.before:
            subprocess.run(a.before, shell=True, check=True, cwd=a.cwd)
        run(a.target, a.cmd, a.cwd, size, a.group, a.out)
        print('%s %s %s: captured' % (a.target, a.group, size), flush=True)


if __name__ == '__main__':
    main()
