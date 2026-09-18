#!/usr/bin/env python3
"""Read-only PKG/DMG verification; does not install on the development machine."""
import hashlib, pathlib, subprocess, sys, tempfile

def run(args): return subprocess.run([str(a) for a in args], check=True, timeout=120)
def main():
    dist = pathlib.Path(sys.argv[1]).resolve()
    pkg = next(dist.glob('*.pkg')); dmg = next(dist.glob('*.dmg'))
    with tempfile.TemporaryDirectory(prefix='loramesh-installer-check-') as tmp:
        root = pathlib.Path(tmp); expanded = root / 'expanded'
        run(['pkgutil', '--expand-full', pkg, expanded])
        binary = next(expanded.rglob('loramesh-mesh-sim'))
        run([binary, '--version'])
        scenario = next(p for p in expanded.rglob('line.json') if p.parts[-3:] == ('scenarios', 'mesh', 'line.json'))
        run([binary, scenario, '--output', root / 'report.json'])
        run(['hdiutil', 'verify', dmg])
        mount = root / 'mounted'; mount.mkdir()
        run(['hdiutil', 'attach', '-readonly', '-nobrowse', '-mountpoint', mount, dmg])
        try:
            embedded = mount / pkg.name
            assert hashlib.sha256(pkg.read_bytes()).digest() == hashlib.sha256(embedded.read_bytes()).digest()
            assert (mount / 'INSTALL.txt').is_file()
        finally: run(['hdiutil', 'detach', mount])
    print('PASS: expanded PKG executes; verified DMG contains identical PKG and installation guide')
if __name__ == '__main__': main()
