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
def rpc_response(root, command):
    payload=command if isinstance(command,bytes) else json.dumps(command).encode()
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
        return json.loads(read(size))
def rpc(root, command):
    reply=rpc_response(root,command)
    if 'error' in reply: raise RuntimeError(reply['error'])
    return reply['data']
def expect_error(root, command, message, code='invalid_command'):
    reply=rpc_response(root,command)
    assert reply.get('error_code')==code,reply
    assert message in reply['error'],reply
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
            api=rpc(root,{'op':'api_info'})
            assert api['control_api_version']==1 and api['c_abi_version']==1
            assert api['aliases']['forward_start']=='forward' and 'vpn_packet' not in api['backends']
            assert f'Version: {version}\n' in root.with_suffix('.log').read_text()
            fixtures=Path(__file__).resolve().parents[1]/'crates/forwiroh/tests/fixtures/control-errors.json'
            for fixture in json.loads(fixtures.read_text()):
                if sys.platform=='linux' and fixture.get('platform')=='non_linux':continue
                reply=rpc_response(root,fixture['raw'].encode() if 'raw' in fixture else fixture['request'])
                assert reply.get('error_code')==fixture['error_code'],(fixture,reply)
                assert isinstance(reply.get('error'),str) and 'data' not in reply,reply
            for op in ('stop','forward_stop','reverse_stop','socks_stop','vpn_stop'):
                assert rpc(root,{'op':op,'id':'missing'}) is None
            expect_error(root,{'op':'stop'},'missing id')
            expect_error(root,{'op':'stop','id':42},'missing id')
            expect_error(root,{'op':'stop','id':'missing','extra':True},'unknown field')
            expect_error(root,{'op':'not_a_command'},'unknown variant')
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
                    assert rpc(roots[0],{'op':'peer_pending'})==[]
                    assert rpc(roots[0],{'op':'status'})['peers']==[]
                    assert rpc(roots[1],{'op':'status'})['peers']==[]
            finally:
                os.close(master)
        status=json.loads(subprocess.check_output([binary,'--state',str(roots[0]),'peers']))
        assert status['peers'][0]['name']=='phone'
        subprocess.run([binary,'--state',str(roots[0]),'allow','phone','ssh','127.0.0.1:22'],check=True,stdout=subprocess.DEVNULL)
        services=json.loads(subprocess.check_output([binary,'--state',str(roots[1]),'connect','home']))
        assert services['services']==['ssh']
        policy=rpc(roots[0],{'op':'peer_policy_get','peer':'phone'})
        changed=rpc(roots[0],{'op':'peer_policy_update','peer':'phone','expected_revision':policy['revision'],'patch':{'reverse_ports':[8080]}})
        assert changed['policy']['services']=={'ssh':'127.0.0.1:22'}
        expect_error(roots[0],{'op':'peer_policy_update','peer':'phone','expected_revision':policy['revision'],'patch':{'restricted':True}},'policy changed','policy_conflict')
        for kind in ('forward','socks'):
            for stop in (kind+'_stop','stop'):
                command={'op':'forward' if kind=='forward' else 'socks_start','peer':'home','listen':'127.0.0.1:0'}
                if kind=='forward':command['service']='ssh'
                tunnel=rpc(roots[1],command)['id']
                for other in ('forward','reverse','socks','vpn'):
                    if other==kind:continue
                    expect_error(roots[1],{'op':other+'_stop','id':tunnel},'expected '+other,'wrong_tunnel_kind')
                    assert any(t['id']==tunnel for t in rpc(roots[1],{'op':'status'})['tunnels'])
                assert rpc(roots[1],{'op':stop,'id':tunnel}) is None
                assert rpc(roots[1],{'op':stop,'id':tunnel}) is None
                assert subprocess.check_output([binary,'--state',str(roots[1]),'stop',tunnel],text=True).strip()=='null'
        with socket.socket() as target, socket.socket() as reserve:
            target.bind(('127.0.0.1',0)); target.listen(); target.settimeout(10)
            reserve.bind(('127.0.0.1',0)); remote_port=reserve.getsockname()[1]; reserve.close()
            rule=f"{remote_port}:localhost:{target.getsockname()[1]}"
            reverse=json.loads(subprocess.check_output([binary,'--state',str(roots[1]),'reverse','home',rule]))
            for other in ('forward','socks','vpn'):
                expect_error(roots[1],{'op':other+'_stop','id':reverse['id']},'expected '+other,'wrong_tunnel_kind')
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
            assert rpc(roots[1],{'op':'reverse_stop','id':reverse['id']}) is None
            assert rpc(roots[1],{'op':'stop','id':reverse['id']}) is None
        subprocess.run([binary,'--state',str(roots[0]),'revoke','phone'],check=True,stdout=subprocess.DEVNULL)
        assert rpc(roots[0],{'op':'status'})['peers']==[]
        print('CLI daemon, terminal approval/rejection/Ctrl-C, pairing, stop kind checks/idempotence, permissions, reverse TCP and revocation: PASS')
    finally:
        for p in processes: p.terminate()
        for p in processes:
            try: p.wait(timeout=10)
            except subprocess.TimeoutExpired: p.kill();p.wait();raise RuntimeError('daemon failed to shut down')
