#!/usr/bin/env python3
"""The studio's copy of the engine's tool surface, and the check that it is the engine's.

WHY THIS EXISTS. The studio kept a hand-written copy of the engine's tool names in nine places (the rule
block, the guard script, the recall scripts, help.md, three match lists in the hook crate) and each was
repaired alone when a failure showed it: studio 4.4.2, 4.4.3, and the engine's 4.4.7 and 4.4.8 were four
patches on one defect (docs/sprints/jawata-mcp/ARCHITECT-REPORT-tool-name-drift.md in jawata-enterprise).
The cure is one file that says what the engine publishes, taken FROM the engine and checked against it.

THE FILE is src-tauri/engine-surface.json:

    { "engineVersion": "4.4.9",
      "live":    [ {"name": "analyze", "changesCode": false}, ... ],
      "retired": [ "add_dependency", ... ] }

`live` comes from the engine's own tools/list answer, over the wire, with the flag the engine publishes in
`_meta."jawata/changesCode"` (Tool.changesCode(), the flag a hired run is classified by). `retired` comes
from the engine's record of every name ever published, build/published-tool-names.txt in jawata-mcp.
The record's live names must equal the wire's tool names, or the engine disagrees with itself and this
refuses.

USAGE
    engine-surface.py generate --tools-list <tools-list.json> --record <published-tool-names.txt> --version <x.y.z>
        prints the file to stdout.
    engine-surface.py check --tools-list <...> --record <...> --vendored <engine-surface.json>
        exit 0 when the vendored file holds the engine's live and retired sets; exit 1 and a diff when not.
        engineVersion is reported but does not fail the check: an engine release that changed nothing
        on the surface is not a reason to fail a studio release.
EXIT  0 ok · 1 the surface differs · 2 misuse or an input that cannot be read (never a pass).
"""
import argparse
import json
import sys

META_KEY = "jawata/changesCode"


def die(message, code=2):
    print("engine-surface: " + message, file=sys.stderr)
    sys.exit(code)


def read_wire(path):
    try:
        with open(path, encoding="utf-8") as fh:
            document = json.load(fh)
    except (OSError, ValueError) as error:
        die("cannot read the tools/list answer %s: %s" % (path, error))
    try:
        tools = document["result"]["tools"]
    except (KeyError, TypeError):
        die("%s is not a tools/list answer (no result.tools)" % path)
    live = {}
    for tool in tools:
        name = tool.get("name")
        flag = (tool.get("_meta") or {}).get(META_KEY)
        if not isinstance(flag, bool):
            die("tool %r carries no boolean _meta[%r]: the engine that answered predates the field, "
                "so a list generated from it would guess" % (name, META_KEY))
        live[name] = flag
    if not live:
        die("the tools/list answer holds no tools")
    return live


def read_record(path):
    live, retired = set(), set()
    try:
        with open(path, encoding="utf-8") as fh:
            lines = fh.read().splitlines()
    except OSError as error:
        die("cannot read the record %s: %s" % (path, error))
    for line in lines:
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) < 2 or fields[1] not in ("live", "retired"):
            die("a record line must be name<TAB>live|retired...: %r" % line)
        (live if fields[1] == "live" else retired).add(fields[0])
    if not retired:
        die("the record holds no retired names: it was truncated or is not the record")
    return live, retired


def surface(wire, record, version):
    record_live, retired = record
    if set(wire) != record_live:
        only_wire = sorted(set(wire) - record_live)
        only_record = sorted(record_live - set(wire))
        die("the engine disagrees with its own record. Live on the wire only: %s. Live in the record only: %s."
            % (only_wire, only_record), 1)
    return {
        "engineVersion": version,
        "live": [{"name": name, "changesCode": wire[name]} for name in sorted(wire)],
        "retired": sorted(retired),
    }


def render(document):
    lines = ["{", '  "engineVersion": %s,' % json.dumps(document["engineVersion"]), '  "live": [']
    rows = ['    {"name": %s, "changesCode": %s}' % (json.dumps(t["name"]), json.dumps(t["changesCode"]))
            for t in document["live"]]
    lines.append(",\n".join(rows))
    lines.append("  ],")
    lines.append('  "retired": [')
    lines.append(",\n".join("    " + json.dumps(n) for n in document["retired"]))
    lines.append("  ]")
    lines.append("}")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["generate", "check"])
    parser.add_argument("--tools-list", required=True)
    parser.add_argument("--record", required=True)
    parser.add_argument("--version")
    parser.add_argument("--vendored")
    args = parser.parse_args()

    wire = read_wire(args.tools_list)
    record = read_record(args.record)

    if args.mode == "generate":
        if not args.version:
            die("generate needs --version")
        sys.stdout.write(render(surface(wire, record, args.version)))
        return

    if not args.vendored:
        die("check needs --vendored")
    try:
        with open(args.vendored, encoding="utf-8") as fh:
            vendored = json.load(fh)
    except (OSError, ValueError) as error:
        die("cannot read the vendored file %s: %s" % (args.vendored, error))
    engine = surface(wire, record, vendored.get("engineVersion", "?"))
    problems = []
    if vendored.get("live") != engine["live"]:
        have = {t["name"]: t["changesCode"] for t in vendored.get("live", [])}
        want = {t["name"]: t["changesCode"] for t in engine["live"]}
        for name in sorted(set(have) | set(want)):
            if have.get(name) != want.get(name):
                problems.append("live %s: vendored %s, engine %s" % (name, have.get(name, "absent"), want.get(name, "absent")))
    if vendored.get("retired") != engine["retired"]:
        have, want = set(vendored.get("retired", [])), set(engine["retired"])
        for name in sorted(have ^ want):
            problems.append("retired %s: vendored %s, engine %s" % (
                name, "yes" if name in have else "no", "yes" if name in want else "no"))
    if problems:
        print("engine-surface: the vendored surface is NOT the engine's:")
        for problem in problems:
            print("  " + problem)
        print("Refresh it with build/refresh-engine-surface.sh and commit the result.")
        sys.exit(1)
    print("engine-surface: the vendored surface (engine %s) holds the engine's %d live and %d retired names."
          % (vendored.get("engineVersion", "?"), len(engine["live"]), len(engine["retired"])))


if __name__ == "__main__":
    main()
