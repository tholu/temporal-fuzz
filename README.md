# temporal-fuzz

`temporal-fuzz` is a standalone defensive research tool for finding parser and decoder bugs caused by delivery timing and chunk boundary differences. It compares a whole-input baseline against temporal schedules that feed the same bytes in different chunks.

This is Temporal Differential Fuzzing for Streaming Boundary Confusion. Use it only on targets you own or are authorized to test. It identifies and triages inconsistencies; it does not implement exploit development.

## Build

```bash
cargo build --release
```

Run from source:

```bash
cargo run -- run --target python3 --target-arg examples/echo_adapter.py --input sample.bin --iterations 100 --out-dir findings/echo
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

- `--mode boundary|stateful|chaos` to select schedule semantics; default is `boundary`
- `--out-dir DIR` for run output, `run.json`, and finding subdirectories
- `--seed N` for deterministic schedule generation
- `--timeout-ms N` for per-execution timeout
- `--corpus DIR` to fuzz the primary input plus every file in a corpus directory
- `--progress-every N` to control progress output
- `--target-arg ARG` to pass adapter arguments without shell parsing
- `--embed-payload false` to save payload bytes beside findings instead of embedding base64 in each finding

Help is available at the root and for each command:

```bash
temporal-fuzz --help
temporal-fuzz run --help
temporal-fuzz replay --help
temporal-fuzz generate --help
temporal-fuzz minimize --help
```

## Detection Model

For each payload, the baseline is:

```text
FEED(full payload) + EOS
```

The default `boundary` mode generates only baseline-equivalent chunking schedules: the same payload bytes, in the same order, without gaps, duplicates, or reordering, ending with one terminal `EOS`. This is the recommended first pass because findings correspond to whole-input-vs-streaming boundary behavior.

`stateful` mode keeps the effective byte stream equivalent but inserts semantic streaming controls such as `FLUSH`, `DRAIN`, `FEED_ZERO`, `RESET`, `SEEK`, and `RECONFIGURE`. Findings in this mode are labeled as stateful control findings, not pure boundary bugs.

`chaos` mode preserves the original aggressive prototype generator. It can remove, reorder, duplicate, or mutate feeds and inject controls including `CLOSE`; this is useful for adapter hardening and broader state-machine exploration, but it can intentionally create non-equivalent input streams.

Findings are saved when the baseline and variant differ by:

- crash vs non-crash
- timeout vs non-timeout
- `status` mismatch
- `output_hash` mismatch
- major runtime difference
- invalid stdout JSON

Each generated schedule records:

- `schedule_class`: `equivalent_boundary`, `stateful_control`, or `non_equivalent_stream`
- `baseline_equivalent`: whether the effective stream preserves the baseline bytes and terminal `EOS`
- `effective_stream_hash`
- `effective_stream_len`

Output directories under `--out-dir`:

- `crashes/`
- `hangs/`
- `divergences/`
- `interesting/`

Saved findings include the input filename, payload hash, schedule, schedule metadata, baseline result, variant result, stderr snippets, command line, and timestamp. By default the payload is embedded as base64 for single-file replayability. With `--embed-payload false`, payload bytes are stored under a `payloads/` directory next to the finding and referenced by relative path.

Each run also writes `<out-dir>/run.json` with command line, target argv, seed, mode, input/corpus paths, iteration count, timeout, final summary counts, and timestamp.

If the whole-input baseline does not return valid `ok` adapter output, that input is reported as a baseline failure and skipped. This keeps variant findings tied to streaming-vs-whole-input differences rather than a broken adapter setup.

## Schedule Generation

The generator splits payloads around:

- random offsets
- powers of two
- regular small-file intervals
- likely header and magic boundaries
- likely length-field locations and delimiter bytes

In `boundary` mode, the generator splits only the delivery schedule, not the logical input stream. In `stateful` mode it adds semantic streaming controls while preserving the effective bytes. In `chaos` mode it mutates schedules by inserting, removing, and reordering ops; shrinking and growing feeds; mutating `SEEK` and `RECONFIGURE`; and injecting stream-control operations.

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
cargo run -- run --mode boundary --target python3 --target-arg examples/echo_adapter.py --input sample.bin --iterations 100 --seed 1 --out-dir findings/echo
```

Run the recommended boundary pass against the intentionally buggy adapter:

```bash
cargo run -- run --mode boundary --target python3 --target-arg examples/buggy_adapter.py --input sample.bin --iterations 100 --seed 1 --timeout-ms 200 --out-dir findings/buggy-boundary
```

Run chaos mode when you intentionally want non-equivalent stream and state-machine stress:

```bash
cargo run -- run --mode chaos --target python3 --target-arg examples/buggy_adapter.py --input sample.bin --iterations 100 --seed 1 --timeout-ms 200 --out-dir findings/buggy-chaos
```

Replay a finding:

```bash
cargo run -- replay --target python3 --target-arg examples/buggy_adapter.py --case findings/buggy-boundary/divergences/id-000000.json
```

Minimize it:

```bash
cargo run -- minimize --mode boundary --target python3 --target-arg examples/buggy_adapter.py --case findings/buggy-boundary/divergences/id-000000.json --out minimized.json
```
