#!/usr/bin/env python3
"""Run the shell installer against local release fixtures; no network/install to HOME."""
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parent.parent


class InstallerTest(unittest.TestCase):
    def run_installer(self, system='Linux', machine='x86_64', android=False,
                      corrupt=False, missing=False, pinned=False):
        with tempfile.TemporaryDirectory(prefix='installer-test-') as directory:
            base = Path(directory)
            fixtures = base / 'fixtures'
            fixtures.mkdir()
            mock = base / 'mock'
            mock.mkdir()
            destination = base / 'install with spaces'
            destination.mkdir()
            binary = 'forwiroh.exe' if system == 'MINGW64_NT' else 'forwiroh'
            installed = destination / binary
            installed.write_text('existing executable')
            arch = 'aarch64' if machine in ('arm64', 'aarch64') else 'x86_64'
            platform = ('apple-darwin' if system == 'Darwin' else
                        'pc-windows-msvc' if binary.endswith('.exe') else
                        'linux-android' if android else 'unknown-linux-musl')
            extension = 'zip' if binary.endswith('.exe') else 'tar.gz'
            asset = f'forwiroh-{arch}-{platform}.{extension}'
            payload = b'#!/bin/sh\nprintf "forwiroh 0.1.1\\n"\n'
            if extension == 'zip':
                with zipfile.ZipFile(fixtures / asset, 'w') as archive:
                    archive.writestr(binary, payload)
            else:
                with tarfile.open(fixtures / asset, 'w:gz') as archive:
                    info = tarfile.TarInfo(binary)
                    info.size = len(payload)
                    info.mode = 0o755
                    archive.addfile(info, io.BytesIO(payload))
            digest = hashlib.sha256((fixtures / asset).read_bytes()).hexdigest()
            (fixtures / 'CLI-SHA256SUMS').write_text('' if missing else f'{digest}  {asset}\n')
            if corrupt:
                with (fixtures / asset).open('ab') as output:
                    output.write(b'corrupted')
            (mock / 'uname').write_text('''#!/bin/sh
case "$1" in
 -m) printf '%s\\n' "$MOCK_MACHINE" ;;
 -s) printf '%s\\n' "$MOCK_SYSTEM" ;;
 -o) printf '%s\\n' "$MOCK_KERNEL" ;;
 *) exit 1 ;;
esac
''')
            (mock / 'curl').write_text('''#!/bin/sh
set -eu
output=
url=
head=false
while [ "$#" -gt 0 ]; do
 case "$1" in
  -o) output=$2; shift ;;
  -w|--proto) shift ;;
  -fsSLI) head=true ;;
  https://*) url=$1 ;;
 esac
 shift
done
printf '%s\\n' "$url" >> "$MOCK_REQUESTS"
if [ "$head" = true ]; then
 printf 'https://github.com/trufae/forwiroh/releases/tag/v0.1.1'
else
 case "$url" in */releases/download/v0.1.1/*) ;; *) exit 98 ;; esac
 cp "$MOCK_FIXTURES/${url##*/}" "$output"
fi
''')
            for path in mock.iterdir():
                path.chmod(0o755)
            env = dict(os.environ, PATH=f'{mock}:{os.environ["PATH"]}',
                       MOCK_MACHINE=machine, MOCK_SYSTEM=system,
                       MOCK_KERNEL='Android' if android else 'GNU/Linux',
                       MOCK_FIXTURES=str(fixtures), MOCK_REQUESTS=str(base / 'requests'),
                       FORWIROH_INSTALL_DIR=str(destination))
            for name in ('ANDROID_ROOT', 'FORWIROH_VERSION', 'FORWIROH_REPOSITORY'):
                env.pop(name, None)
            if pinned:
                env['FORWIROH_VERSION'] = 'v0.1.1'
            result = subprocess.run(['sh', str(ROOT / 'install.sh')], env=env,
                                    text=True, capture_output=True, timeout=15)
            if corrupt or missing or machine == 'unsupported':
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(installed.read_text(), 'existing executable')
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(installed.read_bytes(), payload)
                self.assertTrue(os.access(installed, os.X_OK))
                requests = (base / 'requests').read_text().splitlines()
                self.assertTrue(all('/download/v0.1.1/' in url for url in requests if not url.endswith('/latest')))
                if pinned:
                    self.assertFalse(any(url.endswith('/latest') for url in requests))

    def test_platform_selection_and_installation(self):
        for system, machine, android in (
            ('Linux', 'x86_64', False), ('Linux', 'aarch64', False),
            ('Linux', 'aarch64', True), ('Linux', 'x86_64', True),
            ('Darwin', 'arm64', False), ('Darwin', 'x86_64', False),
            ('MINGW64_NT', 'x86_64', False), ('MINGW64_NT', 'aarch64', False),
        ):
            with self.subTest(system=system, machine=machine, android=android):
                self.run_installer(system, machine, android)

    def test_rejects_corrupt_download_without_replacing_existing_binary(self):
        self.run_installer(corrupt=True)

    def test_rejects_missing_checksum_without_replacing_existing_binary(self):
        self.run_installer(missing=True)

    def test_pinned_version(self):
        self.run_installer(pinned=True)

    def test_rejects_unsupported_architecture(self):
        self.run_installer(machine='unsupported')


if __name__ == '__main__':
    unittest.main()
