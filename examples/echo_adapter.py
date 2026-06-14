#!/usr/bin/env python3
import base64
import hashlib
import json
import sys


def main():
    request = json.load(sys.stdin)
    payload = base64.b64decode(request["payload_b64"])
    output = bytearray()

    for op in request["ops"]:
        name = op["op"]
        if name == "FEED":
            offset = int(op["offset"])
            length = int(op["len"])
            output.extend(payload[offset : offset + length])
        elif name in {"FEED_ZERO", "FLUSH", "DRAIN", "RESET", "RECONFIGURE", "SEEK"}:
            continue
        elif name in {"EOS", "CLOSE"}:
            break

    print(
        json.dumps(
            {
                "status": "ok",
                "output_hash": hashlib.sha256(bytes(output)).hexdigest(),
                "observations": {"output_len": len(output)},
            }
        )
    )


if __name__ == "__main__":
    main()

