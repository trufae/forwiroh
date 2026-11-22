#!/usr/bin/env python3
"""Check the host library's frozen C ABI exports and canonical iOS header."""
import json
import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
library = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / 'target/debug'
manifest = json.loads((root / 'crates/forwiroh/tests/fixtures/control-v1.json').read_text())
darwin = sys.platform == 'darwin'
binary = library / ('libforwiroh.dylib' if darwin else 'libforwiroh.so')
args = ['nm', '-gU'] if darwin else ['nm', '-g', '--defined-only']
output = subprocess.check_output([*args, str(binary)], text=True)
symbols = {line.split()[-1].removeprefix('_') for line in output.splitlines() if line.split()}
missing = set(manifest['c_symbols']) - symbols
if missing:
    sys.exit('Missing C ABI v1 symbols: ' + ', '.join(sorted(missing)))
bridge = root / 'apps/ios/Shared/forwiroh.h'
assert bridge.read_text().splitlines() == [
    '/* Share declarations with every packaged SDK and native consumer. */',
    '#include "../../../crates/forwiroh/include/forwiroh.h"',
], 'The iOS bridge must include the canonical header, without local declarations'
print('C ABI v1 exports and canonical iOS header: PASS')
