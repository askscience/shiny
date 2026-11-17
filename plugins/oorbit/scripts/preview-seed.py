"""Make a planet to preview: found it and let the simulation's steady player
(`oorbit_sim::autoplay`, the real rules) play it for some days, through the
`preview` example.

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
DAYS = sys.argv[2] if len(sys.argv) > 2 else "6"
CLIMATE = sys.argv[3] if len(sys.argv) > 3 else "temperate"
now = str(int(time.time() * 1000))

if os.path.exists(STATE):
    os.remove(STATE)
subprocess.run([PREVIEW, STATE, now, "autoplay", DAYS, CLIMATE], check=True, stdout=subprocess.DEVNULL)
view = json.loads(subprocess.run([PREVIEW, STATE, now, "state"], capture_output=True, text=True, check=True).stdout)["data"]
p = view["planet"]
print("era", p["era"], "people", round(p["population"]), "buildings", sum(1 for x in p["tiles"] if x["building"]))
