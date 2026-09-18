#!/usr/bin/env python3
"""Measure actual two-process CPU/RSS, PTY IPv4 goodput and latency on Unix.
Build release bins first. Outputs one JSON report; no root or hardware needed.
"""
import json, pathlib, socket, struct, subprocess, tempfile, time, statistics
ROOT=pathlib.Path(__file__).resolve().parents[1]
BIN=ROOT/'target'/'release'

def packet(marker):
    p=bytearray(1500);p[0]=0x45;struct.pack_into('!H',p,2,1500);p[8]=64;p[9]=17;p[12:20]=bytes([10,107,0,1,10,107,0,2]);struct.pack_into('!HHHHI',p,20,1234,5678,1480,0,marker)
    checksum=sum(struct.unpack('!10H',p[:20]));checksum=(checksum&65535)+(checksum>>16);struct.pack_into('!H',p,10,~checksum&65535);return p

def wait(check,seconds=15):
    until=time.monotonic()+seconds
    while not check():
        if time.monotonic()>until:raise RuntimeError('initialization timeout')
        time.sleep(.02)

def phase(loaded):
    children=[];sockets=[]
    with tempfile.TemporaryDirectory(prefix='lm-bench-') as directory:
        d=pathlib.Path(directory)
        try:
            scenario=json.loads((ROOT/'scenarios/virtual-lab.json').read_text());scenario['duration_us']=90_000_000;scenario['max_events']=100_000
            (d/'scenario.json').write_text(json.dumps(scenario))
            lab=subprocess.Popen([BIN/'loramesh-sim',d/'scenario.json','--pty'],stdout=open(d/'paths','w'),stderr=open(d/'lab.log','w'));children.append(lab)
            def paths():
                try:return json.loads((d/'paths').read_text())
                except ValueError:return None
            wait(paths)
            radios=paths()
            for i in (1,2):
                s=socket.socket(socket.AF_UNIX,socket.SOCK_DGRAM);s.bind(str(d/f'client{i}'));s.settimeout(30);sockets.append(s)
                cfg={'radio':radios[str(i)],'power_dbm':14,'link':{'node':i,'peer':3-i,'profile':{'sf':7}},'adapter':{'kind':'datagram','bind':str(d/f'packet{i}'),'peer':str(d/f'client{i}')},'metrics':str(d/f'metrics{i}.json')}
                (d/f'config{i}.json').write_text(json.dumps(cfg))
                children.append(subprocess.Popen([BIN/'loramesh-link',d/f'config{i}.json'],stdout=open(d/f'out{i}','w'),stderr=open(d/f'err{i}','w')))
            wait(lambda:all('ready' in (d/f'out{i}').read_text() for i in (1,2)))
            latencies=[];start=time.monotonic()
            if loaded:
                for n in range(6):
                    p=packet(n);before=time.monotonic();sockets[0].sendto(p,str(d/'packet1'));assert sockets[1].recv(65535)==p;latencies.append(time.monotonic()-before)
                time.sleep(.3) # allow the last aggregate ACK to finish
            else:time.sleep(3)
            elapsed=time.monotonic()-start
            for child in children[1:]:child.terminate()
            for child in children[1:]:assert child.wait(timeout=5)==0
            nodes=[json.loads((d/f'metrics{i}.json').read_text()) for i in (1,2)]
            return {'mode':'loaded' if loaded else 'idle','radio_profile':scenario['nodes'][0]['profile'],'packets':len(latencies),'elapsed_sec':elapsed,'application_goodput_bytes_sec':len(latencies)*1472/elapsed,'p50_latency_sec':statistics.median(latencies) if latencies else None,'p95_latency_sec':max(latencies) if latencies else None,'nodes':nodes}
        finally:
            for s in sockets:s.close()
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    try:child.wait(timeout=5)
                    except subprocess.TimeoutExpired:child.kill();child.wait()
if __name__=='__main__':print(json.dumps({'version':1,'phases':[phase(False),phase(True)]},indent=2))
