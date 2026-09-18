#!/usr/bin/env python3
"""Privileged, bounded Linux namespace test. Run after cargo build --bins.
Owns only randomly named namespaces and a private TemporaryDirectory.
"""
import json, os, pathlib, signal, subprocess, tempfile, time, uuid
ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target')) / 'debug'
# This is an OS/TUN integration test, not a low-rate RF throughput benchmark.
# Match the secure TUN test's explicit profile so TCP backoff does not dominate
# the wall-clock deadline. Slow profiles remain covered by deterministic tests.
PROFILE = {'sf': 7, 'bandwidth': 500000}

def run(args, **kw):
    return subprocess.run(args, check=True, timeout=kw.pop('timeout', 90), **kw)

def wait_for(check, seconds=15):
    until = time.monotonic() + seconds
    while not check():
        if time.monotonic() > until: raise RuntimeError('readiness timeout')
        time.sleep(.05)

def main():
    if os.geteuid() != 0: raise SystemExit('Run with sudo on Linux (requires iproute2 and /dev/net/tun)')
    namespaces=[]; children=[]
    with tempfile.TemporaryDirectory(prefix='loramesh-tun-') as tmp:
        tmp=pathlib.Path(tmp)
        try:
            scenario=json.loads((ROOT/'scenarios/virtual-lab.json').read_text())
            for node in scenario['nodes']:node['profile']=PROFILE.copy()
            scenario['duration_us']=300_000_000; scenario['max_events']=100_000
            (tmp/'scenario.json').write_text(json.dumps(scenario))
            out=open(tmp/'radios.json','w'); err=open(tmp/'radios.log','w')
            lab=subprocess.Popen([BIN/'loramesh-sim',tmp/'scenario.json','--pty','--output',tmp/'trace.json'],stdout=out,stderr=err);children.append(lab)
            def paths():
                try:return json.loads((tmp/'radios.json').read_text())
                except (ValueError,FileNotFoundError):return None
            wait_for(paths); radios=paths()
            for i in (1,2):
                ns='lm-'+uuid.uuid4().hex[:10];run(['ip','netns','add',ns]);namespaces.append(ns)
                run(['ip','-n',ns,'link','set','lo','up'])
                cfg={'radio':radios[str(i)],'power_dbm':14,'link':{'node':i,'peer':3-i,'profile':PROFILE.copy()},'adapter':{'kind':'tun','name':'lora0','address':f'10.107.0.{i}','peer_address':f'10.107.0.{3-i}'},'metrics':str(tmp/f'metrics{i}.json')}
                (tmp/f'config{i}.json').write_text(json.dumps(cfg))
                child=subprocess.Popen(['ip','netns','exec',ns,str(BIN/'loramesh-link'),str(tmp/f'config{i}.json')],stdout=open(tmp/f'node{i}.out','w'),stderr=open(tmp/f'node{i}.log','w'));children.append(child)
            wait_for(lambda:all('ready' in (tmp/f'node{i}.out').read_text() for i in (1,2)))
            run(['ip','netns','exec',namespaces[0],'ping','-n','-c','2','-W','20','10.107.0.2'])
            # Linux emits IPv4 fragments for non-DF datagrams exceeding the explicit MTU.
            run(['ip','netns','exec',namespaces[0],'ping','-n','-c','1','-W','40','-M','dont','-s','2000','10.107.0.2'])
            df=subprocess.run(['ip','netns','exec',namespaces[0],'ping','-n','-c','1','-W','2','-M','do','-s','2000','10.107.0.2'],capture_output=True,timeout=10)
            assert df.returncode != 0 and (b'message too long' in df.stderr.lower() or b'mtu' in df.stderr.lower()),df.stderr
            server="""import pathlib,socket,sys
u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);u.bind(('10.107.0.2',7777));u.settimeout(60)
t=socket.socket();t.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);t.bind(('10.107.0.2',7778));t.listen();t.settimeout(90)
pathlib.Path(sys.argv[1]).touch()
p,a=u.recvfrom(65535);assert p==b'u'*4096;u.sendto(p,a)
c,a=t.accept();c.settimeout(90);p=b''
while len(p)<8192:
 b=c.recv(8192-len(p));assert b;p+=b
assert p==b't'*8192;c.sendall(p);c.close()
"""
            ready=tmp/'echo.ready'
            srv=subprocess.Popen(['ip','netns','exec',namespaces[1],'python3','-c',server,str(ready)],stderr=open(tmp/'echo-server.log','w'));children.append(srv)
            client="""import socket,time
u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);u.settimeout(90);u.sendto(b'u'*4096,('10.107.0.2',7777));assert u.recv(65535)==b'u'*4096
t=socket.create_connection(('10.107.0.2',7778),90);t.settimeout(90);t.sendall(b't'*8192);p=b''
while len(p)<8192:
 b=t.recv(8192-len(p));assert b;p+=b
assert p==b't'*8192
"""
            def echo_ready():
                if srv.poll() is not None:raise RuntimeError('echo server exited: '+(tmp/'echo-server.log').read_text())
                return ready.exists()
            wait_for(echo_ready)
            run(['ip','netns','exec',namespaces[0],'python3','-c',client],timeout=180)
            assert srv.wait(timeout=10)==0
            print('PASS: Linux TUN ping, non-DF fragmentation, DF MTU rejection, UDP and TCP')
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    try:child.wait(timeout=5)
                    except subprocess.TimeoutExpired:child.kill();child.wait()
            for ns in reversed(namespaces): subprocess.run(['ip','netns','del',ns],check=False,timeout=10)
            # Preserve logs/replay inputs even on failure before temporary cleanup.
            import shutil
            artifacts=ROOT/'target'/'tun-artifacts';artifacts.mkdir(parents=True,exist_ok=True)
            for f in tmp.iterdir():
                if f.is_file():shutil.copy2(f,artifacts/f.name)
if __name__=='__main__':main()
