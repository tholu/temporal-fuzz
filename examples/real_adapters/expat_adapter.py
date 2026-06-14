#!/usr/bin/env python3
import base64, hashlib, json, sys, xml.parsers.expat

def make_parser(events):
    p = xml.parsers.expat.ParserCreate()
    p.StartElementHandler = lambda name, attrs: events.append(['S', name, sorted(attrs.items())])
    p.EndElementHandler = lambda name: events.append(['E', name])
    p.CharacterDataHandler = lambda data: events.append(['T', data]) if data else None
    return p

def normalize_events(events):
    # Expat may split CharacterData callbacks differently depending on FEED
    # boundaries. Adjacent text events are semantically one text node here.
    merged = []
    for event in events:
        if event and event[0] == 'T' and merged and merged[-1][0] == 'T':
            merged[-1][1] += event[1]
        else:
            merged.append(event)
    return merged

def finish(status, events=None, err='', obs=None):
    obs = {} if obs is None else obs
    events = normalize_events([] if events is None else events)
    canonical = json.dumps(events, sort_keys=True, separators=(',', ':')).encode()
    h_input = canonical if status == 'ok' else err.encode('utf-8', 'replace')
    obs = dict(obs, events=len(events), error=err[:160])
    print(json.dumps({'status': status, 'output_hash': hashlib.sha256(h_input).hexdigest(), 'observations': obs}))

def main():
    req = json.load(sys.stdin)
    payload = base64.b64decode(req['payload_b64'])
    events = []
    parser = make_parser(events)
    try:
        for op in req['ops']:
            name = op['op']
            if name == 'FEED':
                off = int(op['offset']); ln = int(op['len'])
                parser.Parse(payload[off:off+ln], False)
            elif name == 'FEED_ZERO':
                parser.Parse(b'', False)
            elif name in ('FLUSH','DRAIN','SEEK','RECONFIGURE'):
                continue
            elif name == 'RESET':
                events = []
                parser = make_parser(events)
            elif name in ('EOS','CLOSE'):
                parser.Parse(b'', True)
                break
        finish('ok', events)
    except Exception as e:
        finish('error', events, type(e).__name__ + ':' + str(e), {'partial_events': len(events)})
if __name__ == '__main__': main()
