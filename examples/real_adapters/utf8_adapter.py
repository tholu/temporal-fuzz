#!/usr/bin/env python3
import base64, codecs, hashlib, json, sys

def finish(status, text='', err='', obs=None):
    if obs is None: obs = {}
    h_input = text.encode('utf-8', 'replace') if status == 'ok' else err.encode('utf-8', 'replace')
    print(json.dumps({'status': status, 'output_hash': hashlib.sha256(h_input).hexdigest(), 'observations': dict(obs, chars=len(text), error=err[:120])}))

def main():
    req = json.load(sys.stdin)
    payload = base64.b64decode(req['payload_b64'])
    dec = codecs.getincrementaldecoder('utf-8')('strict')
    parts = []
    try:
        for op in req['ops']:
            name = op['op']
            if name == 'FEED':
                off = int(op['offset']); ln = int(op['len'])
                parts.append(dec.decode(payload[off:off+ln], final=False))
            elif name == 'FEED_ZERO':
                parts.append(dec.decode(b'', final=False))
            elif name in ('FLUSH','DRAIN'):
                continue
            elif name == 'RESET':
                dec = codecs.getincrementaldecoder('utf-8')('strict')
            elif name in ('SEEK','RECONFIGURE'):
                continue
            elif name in ('EOS','CLOSE'):
                parts.append(dec.decode(b'', final=True))
                break
        finish('ok', ''.join(parts))
    except Exception as e:
        finish('error', ''.join(parts), type(e).__name__ + ':' + str(e), {'partial_chars': sum(map(len, parts))})
if __name__ == '__main__': main()

