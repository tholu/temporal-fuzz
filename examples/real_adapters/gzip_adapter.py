#!/usr/bin/env python3
import base64, hashlib, json, sys, zlib

def finish(status, out=b'', err='', obs=None):
    if obs is None: obs = {}
    h_input = out if status == 'ok' else err.encode('utf-8', 'replace')
    print(json.dumps({'status': status, 'output_hash': hashlib.sha256(h_input).hexdigest(), 'observations': dict(obs, output_len=len(out), error=err[:120])}))

def main():
    req = json.load(sys.stdin)
    payload = base64.b64decode(req['payload_b64'])
    d = zlib.decompressobj(16 + zlib.MAX_WBITS)
    out = bytearray()
    try:
        for op in req['ops']:
            name = op['op']
            if name == 'FEED':
                off = int(op['offset']); ln = int(op['len'])
                out.extend(d.decompress(payload[off:off+ln]))
            elif name == 'FEED_ZERO':
                out.extend(d.decompress(b''))
            elif name in ('FLUSH','DRAIN','SEEK','RECONFIGURE'):
                continue
            elif name == 'RESET':
                d = zlib.decompressobj(16 + zlib.MAX_WBITS)
            elif name in ('EOS','CLOSE'):
                out.extend(d.flush())
                break
        finish('ok', bytes(out), obs={'unused_len': len(d.unused_data), 'unconsumed_len': len(d.unconsumed_tail)})
    except Exception as e:
        finish('error', bytes(out), type(e).__name__ + ':' + str(e), {'partial_len': len(out)})
if __name__ == '__main__': main()

