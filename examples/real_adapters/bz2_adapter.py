#!/usr/bin/env python3
import base64, bz2, hashlib, json, sys

def finish(status, out=b'', err='', obs=None):
    obs = {} if obs is None else obs
    h_input = out if status == 'ok' else err.encode('utf-8', 'replace')
    obs = dict(obs, output_len=len(out), error=err[:160])
    print(json.dumps({'status': status, 'output_hash': hashlib.sha256(h_input).hexdigest(), 'observations': obs}))

def main():
    req = json.load(sys.stdin)
    payload = base64.b64decode(req['payload_b64'])
    dec = bz2.BZ2Decompressor()
    out = bytearray()
    try:
        for op in req['ops']:
            name = op['op']
            if name == 'FEED':
                off = int(op['offset']); ln = int(op['len'])
                out.extend(dec.decompress(payload[off:off+ln]))
            elif name == 'FEED_ZERO':
                out.extend(dec.decompress(b''))
            elif name in ('FLUSH','DRAIN','SEEK','RECONFIGURE'):
                continue
            elif name == 'RESET':
                dec = bz2.BZ2Decompressor()
            elif name in ('EOS','CLOSE'):
                break
        finish('ok', bytes(out), obs={'eof': getattr(dec, 'eof', False), 'unused_len': len(getattr(dec, 'unused_data', b''))})
    except Exception as e:
        finish('error', bytes(out), type(e).__name__ + ':' + str(e), {'partial_len': len(out)})
if __name__ == '__main__': main()

