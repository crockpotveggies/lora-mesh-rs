#!/usr/bin/env python3
"""Three encrypted mesh daemons with real TUNs, dropped privileges, and a virtual RF line."""
import json,os,pathlib,subprocess,tempfile,time,uuid,shutil
ROOT=pathlib.Path(__file__).resolve().parents[1]
BIN=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',ROOT/'target'))/'debug'
def run(args,**kw):return subprocess.run(args,check=True,timeout=kw.pop('timeout',120),**kw)
def wait(check,seconds=20):
    until=time.monotonic()+seconds
    while not check():
        if time.monotonic()>until:raise RuntimeError('readiness timeout')
        time.sleep(.05)
def main():
    if os.geteuid()!=0:raise SystemExit('Requires root on Linux; daemons drop to uid/gid 65534 before loading keys or starting radio workers')
    children=[];namespaces=[]
    with tempfile.TemporaryDirectory(prefix='lm-secure-tun-') as directory:
        d=pathlib.Path(directory);d.chmod(0o711)
        try:
            case=json.loads((ROOT/'scenarios/mesh/line.json').read_text())
            for node in case['nodes']:node['profile']['bandwidth']=500000
            scenario={'version':1,'name':'secure-tun-line','seed':42,'duration_us':480000000,'max_events':200000,'nodes':case['nodes'],'links':case['links'],'traffic':[],'expect':{}}
            (d/'scenario.json').write_text(json.dumps(scenario))
            lab=subprocess.Popen([BIN/'loramesh-sim',d/'scenario.json','--pty','--output',d/'trace.json'],stdout=open(d/'paths','w'),stderr=open(d/'lab.log','w'));children.append(lab)
            def paths():
                try:return json.loads((d/'paths').read_text())
                except (ValueError,FileNotFoundError):return None
            wait(paths);radios=paths()
            # Grant only the test service account access to these ephemeral virtual radios.
            pathlib.Path(radios['1']).parent.chmod(0o711)
            for path in radios.values():os.chown(path,65534,65534)
            root=d/'identities'
            run([BIN/'loramesh-keygen',root,'1=10.107.0.1','2=10.107.0.2','3=10.107.0.3','--links','1-2,2-3'])
            root.chmod(0o711)
            for i in (1,2,3):
                ns='lm-sec-'+uuid.uuid4().hex[:10];run(['ip','netns','add',ns]);namespaces.append(ns);run(['ip','-n',ns,'link','set','lo','up'])
                folder=root/str(i);cfg=json.loads((folder/'config.json').read_text());cfg['radio']=radios[str(i)];cfg['mesh']['link']['profile']={'sf':7,'bandwidth':500000};cfg['run_as']={'uid':65534,'gid':65534};cfg['adapter']={'kind':'tun','name':'mesh0','address':f'10.107.0.{i}','peer_address':f'10.107.0.{2 if i!=2 else 1}'}
                cfg['mesh']['static_routes']={'2':2,'3':2} if i==1 else {'1':1,'3':3} if i==2 else {'1':2,'2':2}
                (folder/'config.json').write_text(json.dumps(cfg))
                for path in [folder,*folder.iterdir()]:os.chown(path,65534,65534)
                child=subprocess.Popen(['ip','netns','exec',ns,str(BIN/'loramesh-mesh'),str(folder/'config.json')],stdout=open(d/f'node{i}.out','w'),stderr=open(d/f'node{i}.log','w'));children.append(child)
            wait(lambda:all('ready uid=65534 gid=65534' in (d/f'node{i}.out').read_text() for i in (1,2,3)))
            for child in children[1:]:
                status=pathlib.Path(f'/proc/{child.pid}/status').read_text()
                fields={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in status.splitlines() if ':' in line}
                assert fields['Uid'].split()==['65534']*4,fields['Uid']
                assert int(fields['CapEff'],16)==0 and fields['NoNewPrivs']=='1'
            run(['ip','netns','exec',namespaces[0],'ping','-n','-c','2','-i','2','-W','45','10.107.0.3'])
            run(['ip','netns','exec',namespaces[0],'ping','-n','-c','1','-W','60','-M','dont','-s','2000','10.107.0.3'])
            df=subprocess.run(['ip','netns','exec',namespaces[0],'ping','-n','-c','1','-W','2','-M','do','-s','1500','10.107.0.3'],capture_output=True,timeout=10)
            assert df.returncode!=0 and (b'message too long' in df.stderr.lower() or b'mtu' in df.stderr.lower()),df.stderr
            server="""import pathlib,socket,sys
u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);u.bind(('10.107.0.3',7777));u.settimeout(120)
t=socket.socket();t.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);t.bind(('10.107.0.3',7778));t.listen();t.settimeout(120)
pathlib.Path(sys.argv[1]).touch()
p,a=u.recvfrom(65535);assert p==b'u'*2048;u.sendto(p,a)
c,a=t.accept();c.settimeout(120);p=b''
while len(p)<4096:
 b=c.recv(4096-len(p));assert b;p+=b
assert p==b't'*4096;c.sendall(p);c.close()
"""
            ready=d/'echo.ready'
            srv=subprocess.Popen(['ip','netns','exec',namespaces[2],'python3','-c',server,str(ready)],stderr=open(d/'echo-server.log','w'));children.append(srv)
            def echo_ready():
                if srv.poll() is not None:raise RuntimeError('echo server exited: '+(d/'echo-server.log').read_text())
                return ready.exists()
            wait(echo_ready)
            client="""import socket
u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);u.settimeout(120);u.sendto(b'u'*2048,('10.107.0.3',7777));assert u.recv(65535)==b'u'*2048
t=socket.create_connection(('10.107.0.3',7778),120);t.settimeout(120);t.sendall(b't'*4096);p=b''
while len(p)<4096:
 b=t.recv(4096-len(p));assert b;p+=b
assert p==b't'*4096
"""
            run(['ip','netns','exec',namespaces[0],'python3','-c',client],timeout=260);assert srv.wait(timeout=10)==0
            print('PASS: authenticated multihop TUN ping, fragmentation, DF, UDP, TCP; non-root workers, zero effective capabilities, no_new_privs')
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    try:child.wait(timeout=5)
                    except subprocess.TimeoutExpired:child.kill();child.wait()
            for ns in reversed(namespaces):subprocess.run(['ip','netns','del',ns],check=False,timeout=10)
            artifacts=ROOT/'target'/'secure-tun-artifacts';artifacts.mkdir(parents=True,exist_ok=True)
            for path in d.iterdir():
                if path.is_file() and path.name not in ('scenario.json',):shutil.copy2(path,artifacts/path.name)
            for i in (1,2,3):
                metrics=d/'identities'/str(i)/'metrics.json'
                if metrics.exists():shutil.copy2(metrics,artifacts/f'metrics{i}.json')
            # Private keys and replay state are deliberately never copied to test artifacts.
if __name__=='__main__':main()
