#!/usr/bin/env python3
"""Compile and run direct C and C++ consumers against the current shared library."""
import os, re, subprocess, sys, tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
library = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / 'target/debug'
baseline = root / 'Tests/compat/baseline-93883c1'
# Include the frozen declarations with renamed functions, then compare their
# function-pointer types to today's declarations. Both opaque Handle declarations
# refer to the same forward-declared type. No baseline implementation is linked.
names = re.findall(r'\b(forwiroh_\w+)\s*\(', (baseline / 'forwiroh.h').read_text())
names.remove('forwiroh_android_init')  # Host checks; Android builds validate JNI separately.
with tempfile.TemporaryDirectory(prefix='forwiroh-c-', dir='/tmp') as temporary:
    temporary = Path(temporary)
    for compiler, standard, suffix in [(os.environ.get('CC', 'cc'), 'c11', 'c'),
                                        (os.environ.get('CXX', 'c++'), 'c++17', 'cpp')]:
        common = [compiler, '-x', 'c' if suffix == 'c' else 'c++', '-std=' + standard,
                  '-Wall', '-Wextra', '-Werror']
        for consumer, header in [('c-packets', root / 'crates/forwiroh/include'),
                                 ('c-legacy', baseline), ('c-packets-ios', root / 'apps/ios/Shared')]:
            binary = temporary / (consumer + '-' + suffix)
            source = 'c-packets' if consumer == 'c-packets-ios' else consumer
            subprocess.run([*common, '-I' + str(header), str(root / ('Tests/compat/' + source + '.c')),
                            '-L' + str(library), '-Wl,-rpath,' + str(library), '-lforwiroh',
                            '-o', str(binary)], check=True)
            subprocess.run([str(binary), str(temporary / ('state-' + consumer + '-' + suffix))], check=True)
        signature = temporary / ('signatures.' + suffix)
        check = '#include <type_traits>\n' if suffix == 'cpp' else ''
        check += ''.join('#define ' + name + ' baseline_' + name + '\n' for name in names)
        check += '#include "' + str(baseline / 'forwiroh.h') + '"\n#undef FORWIROH_H\n'
        check += ''.join('#undef ' + name + '\n' for name in names)
        check += '#include "' + str(root / 'crates/forwiroh/include/forwiroh.h') + '"\n'
        for name in names:
            if suffix == 'cpp':
                check += 'static_assert(std::is_same<decltype(&' + name + '), decltype(&baseline_' + name + ')>::value, "ABI changed");\n'
            else:
                check += '_Static_assert(_Generic(&' + name + ', __typeof__(&baseline_' + name + '): 1, default: 0), "ABI changed");\n'
        signature.write_text(check)
        subprocess.run([*common, '-fsyntax-only', str(signature)], check=True)
print('C/C++ current, frozen-header, iOS bridge and signature checks: PASS')
