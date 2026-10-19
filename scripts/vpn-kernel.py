#!/usr/bin/env python3
"""Privileged Linux integration: two network namespaces, real TUNs, ICMP and TCP.
Run only in a disposable Linux VM: sudo python3 scripts/vpn-kernel.py.
All namespaces, devices, routes and daemons are removed in finally.
"""
import json, os, socket, struct, subprocess, sys, tempfile, time, uuid
from pathlib import Path
assert sys.platform == 'linux' and os.geteuid() == 0, 'requires Linux root in a disposable test VM'
binary=str(Path(sys.argv[1] if len(sys.argv)>1 else 'target/debug/forwiroh').resolve())
tag=uuid.uuid4().hex[:7]
names=['fs-'+tag+'s','fs-'+tag+'c'];procs=[]
def run(*args):return subprocess.run(args,check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True).stdout
def ip(ns,*args):return run('ip','-n',ns,*args)
def rpc(path,obj):
    data=json.dumps(obj).encode()
    with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as s:
        s.settimeout(30);s.connect(str(path/'control.sock'));s.sendall(b'\x01'+struct.pack('!I',len(data))+data)
        def read(n):
            data=b''
            while len(data)<n:
                part=s.recv(n-len(data))
                if not part:raise RuntimeError('control socket EOF')
                data+=part
            return data
        assert read(1)==b'\x01';size=struct.unpack('!I',read(4))[0];assert size<=8192
        obj=json.loads(read(size))
        if 'error' in obj:raise RuntimeError(obj['error'])
        return obj['data']
with tempfile.TemporaryDirectory() as tmp:
    roots=[Path(tmp)/'server',Path(tmp)/'client']
    try:
        for n in names:run('ip','netns','add',n)
        run('ip','link','add','v'+tag+'s','type','veth','peer','name','v'+tag+'c')
        for i,(ns,side) in enumerate(zip(names,['s','c'])):
            dev='v'+tag+side
            run('ip','link','set',dev,'netns',ns)
            ip(ns,'addr','add',f'10.200.0.{i+1}/24','dev',dev)
            ip(ns,'link','set',dev,'up');ip(ns,'link','set','lo','up')
            ip(ns,'tuntap','add','dev','forwiroh0','mode','tun')
            ip(ns,'addr','add',f'10.250.0.{i+1}/32','dev','forwiroh0')
            ip(ns,'link','set','forwiroh0','mtu','1280','up')
        ip(names[0],'link','add','destination','type','dummy')
        ip(names[0],'addr','add','172.23.0.1/32','dev','destination');ip(names[0],'link','set','destination','up')
        ip(names[0],'route','add','10.250.0.2/32','dev','forwiroh0')
        ip(names[1],'route','add','172.23.0.1/32','dev','forwiroh0')
        for ns,root in zip(names,roots):
            procs.append(subprocess.Popen(['ip','netns','exec',ns,binary,'--state',str(root),'serve','--name',root.name],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE))
        for root in roots:
            deadline=time.monotonic()+25
            while not (root/'control.sock').exists():
                if time.monotonic()>deadline:raise RuntimeError('daemon startup timed out')
                time.sleep(.05)
        invite=rpc(roots[0],{'op':'invite'})
        import concurrent.futures
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            future=pool.submit(rpc,roots[1],{'op':'pair','uri':invite['uri'],'device':'linux'})
            deadline=time.monotonic()+20
            while not (pending:=rpc(roots[0],{'op':'pending'})):
                if time.monotonic()>deadline:raise RuntimeError('pair request timed out')
                time.sleep(.05)
            rpc(roots[0],{'op':'approve','id':pending[0]['id'],'accept':True});future.result(timeout=20)
        rpc(roots[0],{'op':'policy','peer':'client','policy':{'vpn':{'routes':['172.23.0.1/32'],'local_ip':'10.250.0.1','peer_ip':'10.250.0.2','tun':'forwiroh0'}}})
        link=rpc(roots[1],{'op':'vpn','peer':'server','routes':['172.23.0.1/32'],'tun':'forwiroh0'})
        output=run('ip','netns','exec',names[1],'ping','-c','3','-W','3','172.23.0.1')
        assert '0% packet loss' in output,output
        payload=Path(tmp)/'payload';payload.write_bytes(b'Forwiroh VPN\n'*10000)
        procs.append(subprocess.Popen(['ip','netns','exec',names[0],sys.executable,'-m','http.server','8088','--bind','172.23.0.1','--directory',tmp],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE))
        deadline=time.monotonic()+5
        while True:
            result=subprocess.run(['ip','netns','exec',names[1],'curl','--noproxy','*','--fail','--silent','--max-time','4','http://172.23.0.1:8088/payload'],capture_output=True)
            if result.returncode==0:break
            if time.monotonic()>deadline:raise RuntimeError(result.stderr.decode())
            time.sleep(.05)
        assert result.stdout==payload.read_bytes()
        stats=rpc(roots[1],{'op':'vpn_stats','id':link['id']});assert stats['sent']>0 and stats['received']>100000,stats
        rpc(roots[0],{'op':'revoke','peer':'client'})
        failed=subprocess.run(['ip','netns','exec',names[1],'ping','-c','1','-W','1','172.23.0.1'],capture_output=True)
        assert failed.returncode!=0,'revoked VPN still forwarded packets'
        print('Kernel VPN: real TUN ICMP, 130 KB TCP, counters and revocation PASS')
    finally:
        for p in procs:p.terminate()
        for p in procs:
            try:p.wait(timeout=10)
            except subprocess.TimeoutExpired:p.kill();p.wait()
            if p.returncode not in (0,-15):print(p.stderr.read().decode(),file=sys.stderr)
        for ns in names:subprocess.run(['ip','netns','del',ns],capture_output=True)
