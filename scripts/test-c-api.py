#!/usr/bin/env python3
"""Compile and run direct C and C++ consumers against the current shared library."""
import os, subprocess, sys, tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
library = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / 'target/debug'
with tempfile.TemporaryDirectory(prefix='forwiroh-c-', dir='/tmp') as temporary:
    temporary = Path(temporary)
    for compiler, standard, suffix in [(os.environ.get('CC', 'cc'), 'c11', 'c'),
                                        (os.environ.get('CXX', 'c++'), 'c++17', 'cpp')]:
        binary = temporary / ('consumer-' + suffix)
        subprocess.run([compiler, '-x', 'c' if suffix == 'c' else 'c++', '-std=' + standard,
                        '-Wall', '-Wextra', '-Werror', '-I' + str(root / 'crates/forwiroh/include'),
                        str(root / 'Tests/compat/c-packets.c'), '-L' + str(library),
                        '-Wl,-rpath,' + str(library), '-lforwiroh', '-o', str(binary)], check=True)
        subprocess.run([str(binary), str(temporary / ('state-' + suffix))], check=True)
print('C and C++ packet consumers: PASS')
