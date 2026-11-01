#!/usr/bin/env python3
"""Print the developer's shell.json with the idle screensaver and lock off."""
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
config = json.loads(path.read_text()) if path.exists() else {}
config["idle"] = {**config.get("idle", {}), "screensaver": 0, "lock": 0}
print(json.dumps(config, indent=2))
