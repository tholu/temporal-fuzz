#!/usr/bin/env python3
import base64
import hashlib
import json
import sys
import time


def main():
    request = json.load(sys.stdin)
    payload = base64.b64decode(request["payload_b64"])
    output = bytearray()
    corrupt_after_reset = False
    saw_flush = False
    zero_after_flush = 0
    split_length_field = False

    for op in request["ops"]:
        name = op["op"]
        if name == "FEED":
            offset = int(op["offset"])
            length = int(op["len"])
            end = offset + length
            if end == 4:
                split_length_field = True
            chunk = payload[offset:end]
            if corrupt_after_reset:
                output.extend(chunk[::-1])
            else:
                output.extend(chunk)
        elif name == "RESET":
            corrupt_after_reset = True
        elif name == "FLUSH":
            saw_flush = True
        elif name == "FEED_ZERO" and saw_flush:
            zero_after_flush += 1
            if zero_after_flush >= 3:
                time.sleep(10)
        elif name == "CLOSE":
            print("closed before eos", file=sys.stderr)
            sys.exit(7)
        elif name == "EOS":
            break

    digest_input = bytes(output)
    status = "ok"
    if split_length_field and len(payload) >= 4:
        digest_input += b"|split-length-field"
    if corrupt_after_reset:
        status = "error"

    print(
        json.dumps(
            {
                "status": status,
                "output_hash": hashlib.sha256(digest_input).hexdigest(),
                "observations": {
                    "split_length_field": split_length_field,
                    "corrupt_after_reset": corrupt_after_reset,
                },
            }
        )
    )


if __name__ == "__main__":
    main()
