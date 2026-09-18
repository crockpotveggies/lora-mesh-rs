#!/usr/bin/env python3
"""Check behavioral-test coverage of the new protocol; legacy code is reported separately."""
import json,sys
thresholds={'src/link/mod.rs':95,'src/link/wire.rs':98,'src/link/simulation.rs':90, 'src/mesh/mod.rs':92, 'src/mesh/wire.rs':98, 'src/mesh/security.rs':94, 'src/mesh/routing.rs':97, 'src/mesh/simulation.rs':90}
thresholds.update({'src/radio/controller.rs':90, 'src/radio/protocol.rs':90, 'src/radio/runtime.rs':85})
files=json.load(open(sys.argv[1]))['data'][0]['files']
for suffix,minimum in thresholds.items():
    source=next(f for f in files if f['filename'].endswith(suffix))
    percent=source['summary']['lines']['percent']
    print(f'{suffix}: {percent:.2f}% lines (minimum {minimum}%)')
    if percent<minimum:raise SystemExit('coverage threshold failed')
