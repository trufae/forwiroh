#!/usr/bin/env python3
"""Real daemon/control-socket/CLI smoke test; no external network dependency."""
import errno, fcntl, json, os, pty, select, socket, struct, subprocess, sys, tempfile, termios, time
from pathlib import Path
assert os.geteuid() != 0, 'Run the socket-only CLI smoke test as a normal user, without sudo'
print(f'Socket-only daemon/CLI test, uid={os.geteuid()}, no TUN setup')
binary = str(Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/forwiroh').resolve())
version_output=subprocess.check_output([binary,'version'],text=True).strip()
assert subprocess.check_output([binary,'--version'],text=True).strip()==version_output
assert version_output.startswith('forwiroh ')
version=version_output.split(' ',1)[1]
def terminal_session():
    os.setsid()
    fcntl.ioctl(0,termios.TIOCSCTTY,0)
    os.tcsetpgrp(0,os.getpgrp())
def terminal_read(fd, marker):
    output=b''; deadline=time.monotonic()+10
    while marker not in output:
        remaining=deadline-time.monotonic()
        if remaining<=0: raise RuntimeError('terminal prompt timed out')
        if not select.select([fd],[],[],remaining)[0]: continue
        chunk=os.read(fd,8192)
        if not chunk: raise RuntimeError('terminal closed before prompt')
        output+=chunk
    return output
def terminal_wait(process, fd):
    output=b''; deadline=time.monotonic()+5
    while process.poll() is None:
        if time.monotonic()>=deadline:
            raise RuntimeError(f'terminal pairing did not exit: {output[-1024:]!r}')
        # Drain terminal echo/output so a session leader can finish exiting on macOS.
        if select.select([fd],[],[],.05)[0]:
            try:
                chunk=os.read(fd,8192)
            except OSError as e:
                if e.errno!=errno.EIO: raise
                chunk=b''
            if not chunk: return process.wait(timeout=max(.01,deadline-time.monotonic()))
            output+=chunk
    return process.returncode
def rpc(root, command):
    payload=json.dumps(command).encode()
    with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
        s.settimeout(15); s.connect(str(root/'control.sock'));s.sendall(b'\x01'+struct.pack('!I',len(payload))+payload)
        def read(n):
            out=b''
            while len(out)<n:
                part=s.recv(n-len(out))
                if not part: raise RuntimeError('control socket closed')
                out+=part
            return out
        assert read(1)==b'\x01'; size=struct.unpack('!I',read(4))[0]; assert size<=1024*1024
        reply=json.loads(read(size))
        if 'error' in reply: raise RuntimeError(reply['error'])
        return reply['data']
with tempfile.TemporaryDirectory() as tmp:
    roots=[Path(tmp)/'home',Path(tmp)/'phone']; processes=[]
    try:
        for root in roots:
            with root.with_suffix('.log').open('w') as output:
                processes.append(subprocess.Popen([binary,'--state',str(root),'serve','--name',root.name,'--offline'],stdout=output,stderr=subprocess.PIPE))
        for root in roots:
            deadline=time.monotonic()+10
            while not (root/'control.sock').exists():
                if time.monotonic()>deadline: raise RuntimeError('daemon startup timed out')
                time.sleep(.02)
            assert rpc(root,{'op':'status'})['version']==version
            assert f'Version: {version}\n' in root.with_suffix('.log').read_text()
        # Exercise a real controlling terminal, including /dev/tty on macOS.
        for answer in (b'\n',b'\x03',b'y\n'):
            master,slave=pty.openpty()
            try:
                approver=subprocess.Popen([binary,'--state',str(roots[0]),'pair'],stdin=slave,stdout=slave,stderr=slave,preexec_fn=terminal_session)
                processes.append(approver)
            finally:
                os.close(slave)
            try:
                output=terminal_read(master,b'Waiting for pairing request')
                uri=next(line for line in output.decode().splitlines() if line.startswith('forwiroh://pair?data='))
                # CLI accepts on stdin, avoiding the secret in argv/history.
                join=subprocess.Popen([binary,'--state',str(roots[1]),'pair','--accept','-'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                processes.append(join)
                join.stdin.write(uri+'\n');join.stdin.close();join.stdin=None
                terminal_read(master,b'Approve pairing? [y/N]')
                time.sleep(.1)
                assert approver.poll() is None, 'approval prompt exited before terminal input'
                os.write(master,answer)
                assert terminal_wait(approver,master)==0, 'terminal pairing approval failed'
                stdout,stderr=join.communicate(timeout=10)
                if answer==b'y\n':
                    assert join.returncode==0,(stdout,stderr)
                else:
                    assert join.returncode!=0, 'pairing must require explicit approval'
                    assert rpc(roots[0],{'op':'pending'})==[]
                    assert rpc(roots[0],{'op':'status'})['peers']==[]
                    assert rpc(roots[1],{'op':'status'})['peers']==[]
            finally:
                os.close(master)
        status=json.loads(subprocess.check_output([binary,'--state',str(roots[0]),'peers']))
        assert status['peers'][0]['name']=='phone'
        subprocess.run([binary,'--state',str(roots[0]),'allow','phone','ssh','127.0.0.1:22'],check=True,stdout=subprocess.DEVNULL)
        services=json.loads(subprocess.check_output([binary,'--state',str(roots[1]),'connect','home']))
        assert services['services']==['ssh']
        with socket.socket() as target, socket.socket() as reserve:
            target.bind(('127.0.0.1',0)); target.listen(); target.settimeout(10)
            reserve.bind(('127.0.0.1',0)); remote_port=reserve.getsockname()[1]; reserve.close()
            rule=f"{remote_port}:localhost:{target.getsockname()[1]}"
            reverse=json.loads(subprocess.check_output([binary,'--state',str(roots[1]),'reverse','home',rule]))
            with socket.create_connection(('127.0.0.1',remote_port),timeout=10) as source:
                source.sendall(b'cli reverse'); source.shutdown(socket.SHUT_WR)
                accepted,_=target.accept()
                with accepted:
                    accepted.settimeout(10)
                    payload=b''
                    while True:
                        part=accepted.recv(1024)
                        if not part: break
                        payload+=part
                    assert payload==b'cli reverse'
                    accepted.sendall(b'reply'); accepted.shutdown(socket.SHUT_WR)
                payload=b''
                while True:
                    part=source.recv(1024)
                    if not part: break
                    payload+=part
                assert payload==b'reply'
            subprocess.run([binary,'--state',str(roots[1]),'stop',reverse['id']],check=True,stdout=subprocess.DEVNULL)
        subprocess.run([binary,'--state',str(roots[0]),'revoke','phone'],check=True,stdout=subprocess.DEVNULL)
        assert rpc(roots[0],{'op':'status'})['peers']==[]
        print('CLI daemon, terminal approval/rejection/Ctrl-C, pairing, permissions, reverse TCP and revocation: PASS')
    finally:
        for p in processes: p.terminate()
        for p in processes:
            try: p.wait(timeout=10)
            except subprocess.TimeoutExpired: p.kill();p.wait();raise RuntimeError('daemon failed to shut down')
