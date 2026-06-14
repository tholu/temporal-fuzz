# temporal-fuzz real adapter examples

Stdlib-only adapter examples for `temporal-fuzz`.

Adapters:

- `zlib_adapter.py` — wraps `zlib.decompressobj()`
- `gzip_adapter.py` — wraps `zlib.decompressobj(16 + zlib.MAX_WBITS)`
- `utf8_adapter.py` — wraps Python's incremental UTF-8 decoder

Expected boundary behavior: robust APIs should produce zero findings in `--mode boundary`.

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
```

Stateful/chaos modes may intentionally produce divergences because controls like `RESET` change parser/decoder state.
