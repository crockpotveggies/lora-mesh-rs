#!/usr/bin/env python3
"""One physical LoStik: real driver TX completion and secure daemon no-peer behavior, not delivery.
Run only with an antenna attached and an appropriate frequency/power for your location.
"""
import argparse,json,pathlib,socket,struct,subprocess,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
BIN=ROOT/'target/release'
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('port');parser.add_argument('--frequency',type=int,required=True);parser.add_argument('--power',type=int,default=2)
    parser.add_argument('--transmit',action='store_true',help='Explicitly enable RF tests; requires attached antenna')
    parser.add_argument('--output',type=pathlib.Path,default=ROOT/'target/hardware/single-radio.json')
    args=parser.parse_args()
    if not args.transmit:parser.error('This test transmits; explicitly pass --transmit after checking antenna/location settings')
    start=time.monotonic()
    probe=subprocess.run([BIN/'loramesh-radio',args.port,'--sf','7','--frequency',str(args.frequency),'--power',str(args.power),'--send',b'LoRa Mesh hardware test'.hex(),'--duration-ms','1500'],capture_output=True,text=True,timeout=12)
    assert probe.returncode==0,probe.stderr
    assert 'Transmitted(1)' in probe.stderr and 'State(Receiving)' in probe.stderr
    report={'port':args.port,'frequency_hz':args.frequency,'power_dbm':args.power,'sf':7,'bandwidth_hz':125000,'probe_elapsed_sec':time.monotonic()-start,'probe':probe.stderr,'scope':'one physical radio; TX completion and absent-peer expiry only; no RF delivery confirmation'}
    report.update(probe_passed=True, passed=False, stage='secure-daemon')
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n')
    with tempfile.TemporaryDirectory(prefix='lm-hw-',dir='/tmp') as tmp:
        d=pathlib.Path(tmp);identities=d/'identities'
        subprocess.run([BIN/'loramesh-keygen',identities,'1=10.107.0.1','2=10.107.0.2'],check=True,capture_output=True)
        config=json.loads((identities/'1/config.json').read_text())
        config.update(radio=args.port,power_dbm=args.power,adapter={'kind':'datagram','bind':str(d/'packets'),'peer':str(d/'client')},metrics=str(d/'metrics.json'))
        config['mesh']['link'].update(profile={'sf':7,'frequency':args.frequency},max_retries=2,lifetime_us=10000000,duty_per_mille=100)
        config['mesh'].update(announce_us=10000000,neighbor_timeout_us=30000000,static_routes={'2':2})
        path=identities/'1/config.json';path.write_text(json.dumps(config))
        with socket.socket(socket.AF_UNIX,socket.SOCK_DGRAM) as client,open(d/'out','w') as out,open(d/'err','w') as err:
            client.bind(str(d/'client'));client.settimeout(.05)
            child=subprocess.Popen([BIN/'loramesh-mesh',path],stdout=out,stderr=err)
            try:
                until=time.monotonic()+8
                while 'ready' not in (d/'out').read_text():
                    if child.poll() is not None or time.monotonic()>until:raise RuntimeError((d/'err').read_text() or 'daemon readiness timeout')
                    time.sleep(.05)
                packet=bytearray(32);packet[0]=0x45;struct.pack_into('!H',packet,2,32);packet[8]=64;packet[9]=17;packet[12:20]=bytes([10,107,0,1,10,107,0,2]);struct.pack_into('!HHHH',packet,20,1234,5678,12,0);packet[28:]=b'TEST'
                checksum=sum(struct.unpack('!10H',packet[:20]));checksum=(checksum&65535)+(checksum>>16);struct.pack_into('!H',packet,10,~checksum&65535)
                client.sendto(packet,str(d/'packets'));time.sleep(12)
                try:client.recv(65535);raise AssertionError('unexpected delivery with no peer')
                except socket.timeout:pass
                child.terminate();assert child.wait(timeout=5)==0
                metrics=json.loads((d/'metrics.json').read_text())
                assert metrics['radio_transmitted']>=1,metrics
                assert metrics['radio_failures']==0,metrics
                assert metrics['mesh']['no_route_expired']==1 and metrics['mesh']['delivered']==0,metrics
                report['daemon']=metrics;report['daemon_log']=(d/'err').read_text();report['passed']=True;report['stage']='complete'
            finally:
                if child.poll() is None:
                    child.terminate()
                    try:child.wait(timeout=5)
                    except subprocess.TimeoutExpired:child.kill();child.wait()
        # Temporary private identities and state are removed; the report contains metrics only.
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n')
    print('PASS: physical transmit completion, secure daemon radio transmission, bounded absent-peer expiry; no end-to-end RF claim')
    print(args.output)
if __name__=='__main__':main()
