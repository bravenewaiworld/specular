#!/usr/bin/env python3
"""Turns a wireless trace from a device's log into a scenario that replays it on a desktop.

    tools/trace-to-scenario.py specular.log > scenario.json
    SPECULAR_SCENARIO=scenario.json cargo run -p specular-desktop

Reads trace lines (`wireless <service> <seconds> <kind> <payload>`, anywhere in a log line; see
specular-core's `wireless::trace`) and writes `{"wifi": <script>, "bluetooth": <script>}` in the
format `wireless::scenario` reads. Events before a service's first command become its `start`;
each command becomes an answer, made of the events that followed it until the next command, at
the same delays. Answers are used once each, in order, so repeating the same steps on the desktop
replays the same events. An event that arrived after a later command is replayed after that one.

Passwords are never in the trace, so an answer to a join matches any password.
"""

import json
import re
import sys

LINE = re.compile(r"wireless (\w+) (\d+(?:\.\d+)?) (start|call|event) ?(.*)$")


def convert(lines):
    scripts = {}
    for line in lines:
        m = LINE.search(line.rstrip("\n"))
        if not m:
            continue
        service, t, kind, payload = m.group(1), float(m.group(2)), m.group(3), m.group(4)
        script = scripts.setdefault(service, {"start": [], "answers": [], "_since": t})
        if kind == "start":
            script["_since"] = t
        elif kind == "call":
            to = json.loads(payload)
            if isinstance(to, dict) and isinstance(to.get("Join"), dict):
                to["Join"].pop("password", None)
            script["answers"].append({"to": to, "then": []})
            script["_since"] = t
        else:
            timeline = script["answers"][-1]["then"] if script["answers"] else script["start"]
            timeline.append({"after_ms": round((t - script["_since"]) * 1000), "event": json.loads(payload)})
    for script in scripts.values():
        del script["_since"]
    return scripts


if __name__ == "__main__":
    with open(sys.argv[1]) if len(sys.argv) > 1 else sys.stdin as f:
        json.dump(convert(f), sys.stdout, indent=1)
        print()
