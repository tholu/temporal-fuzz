# temporal-fuzz

`temporal-fuzz` is a standalone defensive research tool for finding parser and decoder bugs caused by delivery timing and chunk boundary differences. It compares a whole-input baseline against temporal schedules that feed the same bytes in different chunks and with streaming control operations.

This is Temporal Differential Fuzzing for Streaming Boundary Confusion. Use it only on targets you own or are authorized to test. It identifies and triages inconsistencies; it does not implement exploit development.

## Build

```bash
cargo build --release
```

Run from source:

```bash
cargo run -- run --target python3 --target-arg examples/echo_adapter.py --input sample.bin --iterations 100
```

## Adapter Protocol

The target adapter is executed once per test case. It receives JSON on stdin:

```json
{
  "payload_b64": "SGVsbG8=",
  "ops": [
    { "op": "FEED", "offset": 0, "len": 5 },
    { "op": "EOS" }
  ]
}
```

It must write JSON on stdout:

```json
{
  "status": "ok",
  "output_hash": "stable-result-id",
  "observations": {}
}
```

`status` must be `ok` or `error`. `output_hash` should be deterministic for the decoded result, AST, normalized parse tree, or other semantic output you want to compare.

Supported ops:

- `FEED { offset, len }`
- `FEED_ZERO`
- `FLUSH`
- `DRAIN`
- `RESET`
- `RECONFIGURE { mode, limit }`
- `SEEK { offset }`
- `EOS`
- `CLOSE`

## Commands

Run fuzzing:

```bash
temporal-fuzz run --target ./adapter --input sample.bin --iterations 10000
```

Replay a saved finding or generated case:

```bash
temporal-fuzz replay --target ./adapter --case crashes/id.json
```

Generate cases without executing a target:

```bash
temporal-fuzz generate --input sample.bin --out cases/ --count 1000
```

Minimize a reproducing case:

```bash
temporal-fuzz minimize --target ./adapter --case crashes/id.json --out minimized.json
```

Useful options:

- `--seed N` for deterministic schedule generation
- `--timeout-ms N` for per-execution timeout
- `--corpus DIR` to fuzz the primary input plus every file in a corpus directory
- `--progress-every N` to control progress output
- `--target-arg ARG` to pass adapter arguments without shell parsing
- `--embed-payload false` to save payload bytes beside findings instead of embedding base64 in each finding

## Detection Model

For each payload, the baseline is:

```text
FEED(full payload) + EOS
```

Variants include chunked feeds and control-heavy schedules with `FLUSH`, `DRAIN`, `RESET`, `RECONFIGURE`, `SEEK`, `FEED_ZERO`, `EOS`, and `CLOSE`.

Findings are saved when the baseline and variant differ by:

- crash vs non-crash
- timeout vs non-timeout
- `status` mismatch
- `output_hash` mismatch
- major runtime difference
- invalid stdout JSON

Output directories:

- `crashes/`
- `hangs/`
- `divergences/`
- `interesting/`

Saved findings include the input filename, payload hash, schedule, baseline result, variant result, stderr snippets, command line, and timestamp. By default the payload is embedded as base64 for single-file replayability. With `--embed-payload false`, payload bytes are stored under a `payloads/` directory next to the finding and referenced by relative path.

If the whole-input baseline does not return valid `ok` adapter output, that input is reported as a baseline failure and skipped. This keeps variant findings tied to streaming-vs-whole-input differences rather than a broken adapter setup.

## Schedule Generation

The generator splits payloads around:

- random offsets
- powers of two
- regular small-file intervals
- likely header and magic boundaries
- likely length-field locations and delimiter bytes

It mutates schedules by inserting, removing, and reordering ops; shrinking and growing feeds; mutating `SEEK` and `RECONFIGURE`; and injecting stream-control operations.

## Adapter Authoring

Wrap the parser or decoder under test in a small executable that translates temporal ops into your streaming API. The adapter should keep output deterministic and avoid including wall-clock data or pointer values in `output_hash`.

For parsers that produce structured results, hash a canonical JSON representation. For decoders, hash decoded bytes. For APIs with recoverable parse failures, return `status: "error"` and a deterministic error hash rather than crashing.

## Example Workflow

Create a sample payload:

```bash
printf '0010abcdefghij' > sample.bin
```

Run against the deterministic echo adapter:

```bash
cargo run -- run --target python3 --target-arg examples/echo_adapter.py --input sample.bin --iterations 20 --seed 1
```

Run against the intentionally buggy adapter:

```bash
cargo run -- run --target python3 --target-arg examples/buggy_adapter.py --input sample.bin --iterations 100 --seed 1 --timeout-ms 200
```

Replay a finding:

```bash
cargo run -- replay --target python3 --target-arg examples/buggy_adapter.py --case divergences/id-000000.json
```

Minimize it:

```bash
cargo run -- minimize --target python3 --target-arg examples/buggy_adapter.py --case divergences/id-000000.json --out minimized.json
```
