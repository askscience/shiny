"""Make a planet to preview: found it some days ago and let a simple player
visit it every few hours, through the `preview` example (the real rules).

    cargo build --release -p oorbit-sim --example preview
    python3 -I plugins/oorbit/scripts/preview-seed.py STATE_FILE [DAYS] [CLIMATE]
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
PREVIEW = os.path.join(HERE, "..", "..", "..", "target", "release", "examples", "preview")
STATE = sys.argv[1]
DAYS = float(sys.argv[2]) if len(sys.argv) > 2 else 6
CLIMATE = sys.argv[3] if len(sys.argv) > 3 else "temperate"
HOUR = 3_600_000
now = int(time.time() * 1000)
t = now - int(DAYS * 24 * HOUR)


def run(*args):
    out = subprocess.run([PREVIEW, STATE, str(t), *args], capture_output=True, text=True).stdout
    return json.loads(out)


if os.path.exists(STATE):
    os.remove(STATE)
run("found", CLIMATE)
while t < now - HOUR:
    view = run("state")["data"]
    if view.get("event"):
        run("action", json.dumps({"type": "resolve_event", "choice": view["event"]["choices"][0]["id"]}))
    if not view["planet"]["research"]:
        for tech in sorted((x for x in view["techs"] if x["status"] == "available"), key=lambda x: x["cost"]):
            if run("action", json.dumps({"type": "research", "tech": tech["id"]}))["success"]:
                break
    era = view.get("era") or {}
    wanted = [era.get("monument"), "hut", "house", "field", "fire_pit", "hunting_camp", "storehouse", "pasture",
              "mine", "foundry", "temple", "scribe_house", "great_granary", "stone_circle", "ziggurat"]
    built = [x["building"] for x in view["planet"]["tiles"] if x["building"]]
    for b in wanted:
        d = next((x for x in view["buildings"] if x["id"] == b), None)
        if not d or d["status"] != "available" or not d["tiles"]:
            continue
        if b not in ("hut", "house", "field") and b in built:
            continue
        if run("action", json.dumps({"type": "build", "building": b, "tile": d["tiles"][-1]}))["success"]:
            break
    t += 6 * HOUR
view = run("state")["data"]
p = view["planet"]
print("era", p["era"], "people", round(p["population"]), "buildings", sum(1 for x in p["tiles"] if x["building"]))
