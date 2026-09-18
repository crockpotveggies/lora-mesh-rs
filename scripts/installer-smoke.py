#!/usr/bin/env python3
"""Install/upgrade/uninstall on disposable CI runners only. Never run on a workstation."""
import json, os, pathlib, subprocess, sys, tempfile
ROOT = pathlib.Path(__file__).resolve().parents[1]
def run(args): return subprocess.run([str(a) for a in args], check=True, timeout=180)
def main():
    if os.environ.get('CI') != 'true': raise SystemExit('Installer lifecycle tests require a disposable CI=true runner')
    dist = pathlib.Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix='loramesh-state-preservation-') as tmp:
        marker = pathlib.Path(tmp) / 'deployment-state'; marker.write_text('never touch external state')
        if sys.platform == 'darwin':
            pkg = next(dist.glob('*.pkg'))
            for _ in range(2): run(['sudo', 'installer', '-pkg', pkg, '-target', '/'])
            binary = pathlib.Path('/usr/local/bin/loramesh-mesh-sim'); resources = pathlib.Path('/usr/local/share/loramesh')
            uninstall = ['sudo', 'sh', resources / 'uninstall.sh']
        elif sys.platform == 'linux':
            deb = next(dist.glob('*.deb'))
            for _ in range(2): run(['sudo', 'apt-get', 'install', '-y', '--reinstall', str(deb)])
            binary = pathlib.Path('/usr/bin/loramesh-mesh-sim'); resources = pathlib.Path('/usr/share/loramesh')
            uninstall = ['sudo', 'apt-get', 'remove', '-y', 'loramesh']
        else:
            installer = next(dist.glob('*-setup.exe')); resources = pathlib.Path(tmp) / 'installed'
            for _ in range(2): run([installer, '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/DIR=' + str(resources)])
            binary = resources / 'bin/loramesh-mesh-sim.exe'
            uninstall = [resources / 'unins000.exe', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART']
        run([binary, '--version'])
        report = pathlib.Path(tmp) / 'report.json'
        run([binary, resources / 'scenarios/mesh/line.json', '--output', report])
        assert not json.loads(report.read_text())['failures']
        run(uninstall)
        # Windows uninstaller delegates cleanup to a child process.
        import time
        for _ in range(100):
            if not binary.exists(): break
            time.sleep(.1)
        assert not binary.exists(), 'uninstall left packaged executable'
        assert marker.read_text() == 'never touch external state'
    print('PASS: installation, reinstall/upgrade, installed simulation, uninstall, external state preserved')
if __name__ == '__main__': main()
