#!/usr/bin/env python3
"""Check portable payload hashes and run the shipped simulator without any hardware."""
import argparse, hashlib, json, pathlib, subprocess, tarfile, tempfile, zipfile

def verify(root):
    manifest = json.loads((root / 'manifest.json').read_text())
    for name, expected in manifest.items():
        path = root / name
        if path.resolve().is_relative_to(root.resolve()) is False: raise ValueError('unsafe manifest path')
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected: raise ValueError('payload hash mismatch: ' + name)
    files = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
    assert files == set(manifest) | {'manifest.json'}, 'unlisted files in payload'
    assert not any(p.endswith('.key') or p.endswith('/state.json') for p in files)
    build = json.loads((root / 'build.json').read_text())
    extension = '.exe' if 'windows' in build['target'] else ''
    for binary in build['binaries']:
        result = subprocess.check_output([root / 'bin' / (binary + extension), '--version'], text=True).strip()
        assert result == binary + ' ' + build['version'], result
    with tempfile.TemporaryDirectory(prefix='loramesh-installed-test-') as tmp:
        report, replay = pathlib.Path(tmp) / 'report.json', pathlib.Path(tmp) / 'replay.json'
        binary = root / 'bin' / ('loramesh-mesh-sim' + extension)
        subprocess.run([binary, root / 'scenarios/mesh/line.json', '--output', report], check=True, timeout=30)
        subprocess.run([binary, report, '--replay', '--output', replay], check=True, timeout=30)
        assert report.read_bytes() == replay.read_bytes()
    print('PASS: manifest, versions, simulator and deterministic replay')

def main():
    parser = argparse.ArgumentParser(); parser.add_argument('archive', type=pathlib.Path); args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='loramesh-verify-') as tmp:
        root = pathlib.Path(tmp)
        if args.archive.suffix == '.zip':
            with zipfile.ZipFile(args.archive) as archive:
                for name in archive.namelist():
                    assert (root / name).resolve().is_relative_to(root.resolve()), 'unsafe archive path'
                archive.extractall(root)
        else:
            with tarfile.open(args.archive) as archive:
                for member in archive.getmembers():
                    path = root / member.name
                    if not path.resolve().is_relative_to(root.resolve()) or not (member.isfile() or member.isdir()):
                        raise ValueError('unsafe archive entry')
                    if member.isdir(): path.mkdir(parents=True, exist_ok=True)
                    else:
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_bytes(archive.extractfile(member).read())
                        path.chmod(member.mode & 0o777)
        contents = list(root.iterdir()); assert len(contents) == 1
        verify(contents[0])
if __name__ == '__main__': main()
