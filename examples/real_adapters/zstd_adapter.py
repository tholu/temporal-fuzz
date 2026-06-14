#!/usr/bin/env python3
import base64, ctypes, ctypes.util, hashlib, json, os, sys

class InBuffer(ctypes.Structure):
    _fields_ = [('src', ctypes.c_void_p), ('size', ctypes.c_size_t), ('pos', ctypes.c_size_t)]
class OutBuffer(ctypes.Structure):
    _fields_ = [('dst', ctypes.c_void_p), ('size', ctypes.c_size_t), ('pos', ctypes.c_size_t)]

def load_zstd():
    candidates = [os.environ.get('ZSTD_LIB'), ctypes.util.find_library('zstd'), '/opt/homebrew/lib/libzstd.dylib', '/usr/local/lib/libzstd.dylib', 'libzstd.so.1', 'libzstd.so']
    for c in candidates:
        if not c: continue
        try: return ctypes.CDLL(c)
        except OSError: pass
    raise RuntimeError('libzstd not found; set ZSTD_LIB')

lib = load_zstd()
lib.ZSTD_createDStream.restype = ctypes.c_void_p
lib.ZSTD_freeDStream.argtypes = [ctypes.c_void_p]
lib.ZSTD_initDStream.argtypes = [ctypes.c_void_p]
lib.ZSTD_initDStream.restype = ctypes.c_size_t
lib.ZSTD_decompressStream.argtypes = [ctypes.c_void_p, ctypes.POINTER(OutBuffer), ctypes.POINTER(InBuffer)]
lib.ZSTD_decompressStream.restype = ctypes.c_size_t
lib.ZSTD_isError.argtypes = [ctypes.c_size_t]
lib.ZSTD_isError.restype = ctypes.c_uint
lib.ZSTD_getErrorName.argtypes = [ctypes.c_size_t]
lib.ZSTD_getErrorName.restype = ctypes.c_char_p

def zerr(code):
    return lib.ZSTD_getErrorName(code).decode('utf-8', 'replace')

def new_ctx():
    ctx = lib.ZSTD_createDStream()
    if not ctx: raise RuntimeError('ZSTD_createDStream failed')
    rc = lib.ZSTD_initDStream(ctx)
    if lib.ZSTD_isError(rc): raise RuntimeError('ZSTD_initDStream:' + zerr(rc))
    return ctx

def finish(status, out=b'', err='', obs=None):
    obs = {} if obs is None else obs
    h_input = out if status == 'ok' else err.encode('utf-8', 'replace')
    obs = dict(obs, output_len=len(out), error=err[:160])
    print(json.dumps({'status': status, 'output_hash': hashlib.sha256(h_input).hexdigest(), 'observations': obs}))

def feed(ctx, chunk):
    if not chunk:
        return b'', 0
    in_arr = ctypes.create_string_buffer(chunk)
    ib = InBuffer(ctypes.cast(in_arr, ctypes.c_void_p), len(chunk), 0)
    pieces = []
    last_rc = 0
    while ib.pos < ib.size:
        out_arr = ctypes.create_string_buffer(1 << 16)
        ob = OutBuffer(ctypes.cast(out_arr, ctypes.c_void_p), ctypes.sizeof(out_arr), 0)
        rc = lib.ZSTD_decompressStream(ctx, ctypes.byref(ob), ctypes.byref(ib))
        if lib.ZSTD_isError(rc):
            raise RuntimeError(zerr(rc))
        last_rc = int(rc)
        if ob.pos:
            pieces.append(out_arr.raw[:ob.pos])
    return b''.join(pieces), last_rc

def main():
    req = json.load(sys.stdin)
    payload = base64.b64decode(req['payload_b64'])
    ctx = new_ctx(); out = bytearray(); last_rc = None
    try:
        for op in req['ops']:
            name = op['op']
            if name == 'FEED':
                off = int(op['offset']); ln = int(op['len'])
                got, last_rc = feed(ctx, payload[off:off+ln]); out.extend(got)
            elif name == 'FEED_ZERO':
                got, last_rc = feed(ctx, b''); out.extend(got)
            elif name in ('FLUSH','DRAIN','SEEK','RECONFIGURE'):
                continue
            elif name == 'RESET':
                lib.ZSTD_freeDStream(ctx); ctx = new_ctx()
            elif name in ('EOS','CLOSE'):
                break
        finish('ok', bytes(out), obs={'remaining_hint': last_rc})
    except Exception as e:
        finish('error', bytes(out), type(e).__name__ + ':' + str(e), {'partial_len': len(out)})
    finally:
        try: lib.ZSTD_freeDStream(ctx)
        except Exception: pass
if __name__ == '__main__': main()

