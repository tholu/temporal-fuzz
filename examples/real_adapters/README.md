# temporal-fuzz real adapter examples

Real adapter examples for `temporal-fuzz`.

Adapters:

- `zlib_adapter.py` — wraps `zlib.decompressobj()`
- `gzip_adapter.py` — wraps `zlib.decompressobj(16 + zlib.MAX_WBITS)`
- `utf8_adapter.py` — wraps Python's incremental UTF-8 decoder
- `bz2_adapter.py` — wraps Python stdlib `bz2.BZ2Decompressor`
- `lzma_adapter.py` — wraps Python stdlib `lzma.LZMADecompressor(format=FORMAT_AUTO)`
- `zstd_adapter.py` — wraps `libzstd` `ZSTD_decompressStream` through `ctypes`
- `expat_adapter.py` — wraps Python stdlib Expat and normalizes adjacent text callbacks before hashing

Expected boundary behavior: robust APIs should produce zero findings in `--mode boundary`.

Most adapters are stdlib-only. `zstd_adapter.py` requires a loadable system `libzstd`; set `ZSTD_LIB=/path/to/libzstd.dylib` or `ZSTD_LIB=/path/to/libzstd.so` if auto-detection fails.

Example commands:

```sh
printf 'hello world\nhello world\n' > sample.txt
python3 - <<'PY'
import zlib, gzip
p = open('sample.txt','rb').read()
open('sample.z','wb').write(zlib.compress(p))
open('sample.gz','wb').write(gzip.compress(p))
PY

temporal-fuzz run --mode boundary \
  --target python3 --target-arg examples/real_adapters/zlib_adapter.py \
  --input sample.z --iterations 100 --out-dir findings/zlib-boundary

temporal-fuzz run --mode boundary \
  --target python3 --target-arg examples/real_adapters/gzip_adapter.py \
  --input sample.gz --iterations 100 --out-dir findings/gzip-boundary

temporal-fuzz run --mode boundary \
  --target python3 --target-arg examples/real_adapters/utf8_adapter.py \
  --input sample.txt --iterations 100 --out-dir findings/utf8-boundary

temporal-fuzz run --mode boundary \
  --target python3 --target-arg examples/real_adapters/expat_adapter.py \
  --input sample.xml --iterations 100 --out-dir findings/expat-boundary
```

Stateful/chaos modes may intentionally produce divergences because controls like `RESET` change parser/decoder state.
