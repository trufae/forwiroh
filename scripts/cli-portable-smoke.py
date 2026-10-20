#!/usr/bin/env python3
"""Exercise the shipped CLI, local IPC, concurrent clients and saved state."""
import concurrent.futures
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1]).resolve())
version = subprocess.check_output([binary, '--version'], text=True).strip()
assert version == subprocess.check_output([binary, 'version'], text=True).strip()


def command(state, *args):
    return subprocess.check_output([binary, '--state', str(state), *args], text=True,
                                   stderr=subprocess.PIPE, timeout=20)


def stop(process):
    if process.poll() is None:
        if os.name == 'nt':
            process.terminate()
        else:
            process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


with tempfile.TemporaryDirectory(prefix='forwiroh-cli-') as temporary:
    roots = [Path(temporary) / name for name in ('home', 'phone')]
    processes = []
    logs = []
    try:
        for root in roots:
            log = root.with_suffix('.log').open('w+')
            logs.append(log)
            process = subprocess.Popen([binary, '--state', str(root), 'serve', '--offline',
                                        '--name', root.name], stdout=log, stderr=log)
            processes.append(process)
            deadline = time.monotonic() + 30
            while True:
                if process.poll() is not None:
                    log.seek(0)
                    raise RuntimeError(f'daemon exited: {log.read()}')
                try:
                    status = json.loads(command(root, 'status'))
                    assert status['name'] == root.name
                    assert status['version'] == version.split(' ', 1)[1]
                    break
                except subprocess.CalledProcessError:
                    if time.monotonic() > deadline:
                        raise RuntimeError('daemon startup timed out')
                    time.sleep(.05)
        original = json.loads(command(roots[0], 'status'))['id']
        # Distinct state directories must select distinct sockets/pipes.
        assert original != json.loads(command(roots[1], 'status'))['id']
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            replies = list(pool.map(lambda _: json.loads(command(roots[0], 'status')), range(24)))
        assert all(reply['id'] == original for reply in replies)
        duplicate = subprocess.run([binary, '--state', str(roots[0]), 'serve', '--offline'],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        assert duplicate.returncode != 0, 'second daemon must not take over the identity'
        stop(processes[0])
        logs[0].close()
        logs[0] = roots[0].with_suffix('.log').open('w+')
        processes[0] = subprocess.Popen([binary, '--state', str(roots[0]), 'serve', '--offline'],
                                         stdout=logs[0], stderr=logs[0])
        deadline = time.monotonic() + 30
        while True:
            try:
                assert json.loads(command(roots[0], 'status'))['id'] == original
                break
            except subprocess.CalledProcessError:
                if time.monotonic() > deadline:
                    logs[0].seek(0)
                    raise RuntimeError(f'restart timed out: {logs[0].read()}')
                time.sleep(.05)
        print('CLI version, isolated/concurrent local control, identity lock and persistence: PASS')
    finally:
        for process in processes:
            stop(process)
        for log in logs:
            log.close()
