#!/usr/bin/env python3
"""A fake OpenAI-compatible server, so live.py can drive real `wizard acp` sessions with no model.

    fake_openai.py PORT PROJECT_DIR LOG

The last word of the user's message picks the answer: md, long, slow, bash, git, read, edit,
write, plan; anything else gets "ok". A tool result always gets "done".
"""
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18999
CWD = sys.argv[2] if len(sys.argv) > 2 else '.'
LOG = open(sys.argv[3] if len(sys.argv) > 3 else '/dev/null', 'a')

MD = ('Here is the summary.\n\n| file | lines |\n|------|-------|\n| a.rs | 3 |\n| b.rs | 120 |\n\n'
      'Use `cargo build` to build **`src/`**, see [Docs](https://example.com/docs).\n\n'
      '```rust\nfn main() { println!("hi"); }\n```\n')
LONG = '\n\n'.join('Paragraph %d of a long answer that goes on for a while so the transcript scrolls.' % i
                   for i in range(1, 61))


def pick(tools, *names):
    for n in names:
        for t in tools:
            if t.lower() == n.lower():
                return t
    return names[0]


class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *a):
        pass

    def send_json(self, obj):
        b = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        self.send_json({'object': 'list', 'data': [{'id': 'fake-model', 'object': 'model', 'created': 1},
                                                   {'id': 'fake-other', 'object': 'model', 'created': 0}]})

    def chunk(self, delta, finish=None):
        obj = {'id': 'x', 'object': 'chat.completion.chunk', 'created': 1, 'model': 'fake-model',
               'choices': [{'index': 0, 'delta': delta, 'finish_reason': finish}]}
        data = ('data: %s\n\n' % json.dumps(obj)).encode()
        self.wfile.write(b'%x\r\n%s\r\n' % (len(data), data))
        self.wfile.flush()

    def text(self, s, delay=0.0):
        for i in range(0, len(s), 40):
            self.chunk({'content': s[i:i + 40]})
            if delay:
                time.sleep(delay)

    def tool(self, name, args):
        self.chunk({'tool_calls': [{'index': 0, 'id': 'call_%d' % int(time.time() * 1000), 'type': 'function',
                                    'function': {'name': name, 'arguments': json.dumps(args)}}]})

    def do_POST(self):
        try:
            self.answer()
        except (BrokenPipeError, ConnectionResetError):
            pass  # wizard hung up: an interrupted turn

    def answer(self):
        n = int(self.headers.get('Content-Length', 0))
        body = json.loads(self.rfile.read(n) or b'{}')
        msgs = body.get('messages', [])
        tools = [t.get('function', {}).get('name', '') for t in body.get('tools', [])]
        last = msgs[-1] if msgs else {}
        users = [m for m in msgs if m.get('role') == 'user']
        u = users[-1].get('content', '') if users else ''
        if isinstance(u, list):
            u = ' '.join(p.get('text', '') for p in u if isinstance(p, dict))
        LOG.write(json.dumps({'tools': tools, 'last_role': last.get('role'), 'user': u[-200:],
                              'last': str(last.get('content'))[:300]}) + '\n')
        LOG.flush()
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Transfer-Encoding', 'chunked')
        self.end_headers()
        self.chunk({'role': 'assistant'})
        key = u.strip().split()[-1] if u.strip() else ''
        if last.get('role') == 'tool':
            self.text('done')
            fin = 'stop'
        elif key == 'md':
            self.text(MD, 0.05)
            fin = 'stop'
        elif key == 'long':
            self.text(LONG, 0.01)
            fin = 'stop'
        elif key == 'slow':
            self.text('Thinking about it', 0)
            time.sleep(40)
            self.text(' and done.')
            fin = 'stop'
        elif key == 'bash':
            self.tool(pick(tools, 'execute', 'bash'), {'command': 'echo hi', 'description': 'Print hi'})
            fin = 'tool_calls'
        elif key == 'git':
            self.tool(pick(tools, 'execute', 'bash'), {'command': 'git init -q && echo ok', 'description': 'Initialize a git repository'})
            fin = 'tool_calls'
        elif key == 'read':
            self.tool(pick(tools, 'read_file', 'read', 'view'), {'path': CWD + '/main.rs'})
            fin = 'tool_calls'
        elif key == 'edit':
            self.tool(pick(tools, 'edit_file', 'edit', 'str_replace'), {'path': CWD + '/main.rs', 'old_string': 'hello', 'new_string': 'hi', 'old': 'hello', 'new': 'hi'})
            fin = 'tool_calls'
        elif key == 'plan' and 'exit_plan' in tools:
            self.tool('exit_plan', {'plan': '1. Add `// entry point` as the first line of main.rs.\n2. Save the file.'})
            fin = 'tool_calls'
        elif key == 'write':
            self.tool(pick(tools, 'write_file', 'write', 'create_file'), {'path': CWD + '/w.txt', 'content': ''.join('%d\n' % i for i in range(1, 31))})
            fin = 'tool_calls'
        else:
            self.text('ok')
            fin = 'stop'
        self.chunk({}, fin)
        usage = {'id': 'x', 'object': 'chat.completion.chunk', 'created': 1, 'model': 'fake-model', 'choices': [],
                 'usage': {'prompt_tokens': 100, 'completion_tokens': 10, 'total_tokens': 110}}
        data = ('data: %s\n\ndata: [DONE]\n\n' % json.dumps(usage)).encode()
        self.wfile.write(b'%x\r\n%s\r\n0\r\n\r\n' % (len(data), data))
        self.wfile.flush()


ThreadingHTTPServer(('127.0.0.1', PORT), H).serve_forever()
