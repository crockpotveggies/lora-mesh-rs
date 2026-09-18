#!/usr/bin/env python3
"""Validate complete build artifacts, upload to a draft, publish only after all uploads succeed."""
import argparse, hashlib, json, os, pathlib, subprocess, sys
TARGETS = {'aarch64-apple-darwin': ['.dmg', '.pkg', '.tar.gz'], 'x86_64-apple-darwin': ['.dmg', '.pkg', '.tar.gz'], 'x86_64-pc-windows-msvc': ['-setup.exe', '.zip'], 'x86_64-unknown-linux-gnu': ['.deb', '.tar.gz'], 'aarch64-unknown-linux-gnu': ['.deb', '.tar.gz']}
def run(args, **kwargs): return subprocess.run(args, check=True, **kwargs)
def validate(directory, version, revision):
    expected = set()
    builds = []
    for target, extensions in TARGETS.items():
        name = f'loramesh-{version}-{target}'
        files = [name + ext for ext in extensions] + [name + '-build.json']
        for filename in files:
            path = directory / filename; sidecar = directory / (filename + '.sha256')
            expected.update([filename, filename + '.sha256'])
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            if sidecar.read_text().strip() != digest + '  ' + filename: raise ValueError('Invalid checksum: ' + filename)
        build = json.loads((directory / (name + '-build.json')).read_text())
        if build['version'] != version or build['target'] != target or build['revision'] != revision or build['dirty']:
            raise ValueError('Build provenance/version mismatch: ' + target)
        builds.append(build)
    source = directory / f'loramesh-{version}-source.tar.gz'
    sidecar = source.with_name(source.name + '.sha256')
    if sidecar.read_text().strip() != hashlib.sha256(source.read_bytes()).hexdigest() + '  ' + source.name:
        raise ValueError('Invalid source archive checksum')
    expected.update([source.name, sidecar.name])
    actual = {p.name for p in directory.iterdir() if p.is_file()}
    if actual != expected: raise ValueError('Missing/unexpected release files: ' + repr(actual ^ expected))
    return sorted(directory / filename for filename in expected), builds

def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--tag', required=True); parser.add_argument('--directory', type=pathlib.Path, required=True); parser.add_argument('--check-only', action='store_true'); args = parser.parse_args()
    # Reuse the same strict version/tag contract as the packager.
    import importlib.util
    spec = importlib.util.spec_from_file_location('package_release', pathlib.Path(__file__).with_name('package-release.py'))
    package = importlib.util.module_from_spec(spec); spec.loader.exec_module(package)
    version = package.version(args.tag)
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    if subprocess.check_output(['git', 'rev-parse', args.tag + '^{commit}'], text=True).strip() != revision:
        raise ValueError('Checked out revision differs from release tag')
    source = args.directory / f'loramesh-{version}-source.tar.gz'
    if not args.check_only:
        run(['git', 'archive', '--format=tar.gz', '--prefix=loramesh-' + version + '/', '-o', str(source), 'HEAD'])
        source.with_name(source.name + '.sha256').write_text(hashlib.sha256(source.read_bytes()).hexdigest() + '  ' + source.name + '\n')
    files, builds = validate(args.directory, version, revision)
    if args.check_only: print('Release artifact set verified'); return
    existing = subprocess.run(['gh', 'release', 'view', args.tag, '--json', 'isDraft'], capture_output=True, text=True)
    if existing.returncode == 0 and not json.loads(existing.stdout)['isDraft']:
        raise ValueError('Release is already published; refusing to replace its assets')
    notes = args.directory.parent / 'release-notes.md'
    lines = [f'LoRa Mesh {version}', '', f'Source commit: `{revision}`', '', '| Platform | Included capability | Signed / notarized installers |', '| --- | --- | --- |']
    for build in builds: lines.append(f"| {build['target']} | {build['networking']} | {build['signed']} / {build['notarized_installers']} |")
    lines += ['', 'DMGs contain command-line PKG installers; Windows EXEs are per-user tool installers.', 'No package creates identities, configures radios or starts a service automatically.', 'Keep deployment keys/state outside installation directories. See the included installation guide.', 'SHA-256 sidecars accompany every artifact. Unsigned builds may trigger operating-system trust warnings.']
    notes.write_text('\n'.join(lines) + '\n')
    if existing.returncode != 0:
        run(['gh', 'release', 'create', args.tag, '--verify-tag', '--draft', '--title', 'LoRa Mesh ' + version, '--notes-file', str(notes)])
    else: run(['gh', 'release', 'edit', args.tag, '--notes-file', str(notes)])
    run(['gh', 'release', 'upload', args.tag, *map(str, files), '--clobber'])
    uploaded = json.loads(subprocess.check_output(['gh', 'release', 'view', args.tag, '--json', 'assets'], text=True))['assets']
    if {a['name']: a['size'] for a in uploaded} != {p.name: p.stat().st_size for p in files}:
        raise ValueError('Remote draft assets differ from the validated artifact set; leaving draft unpublished')
    run(['gh', 'release', 'edit', args.tag, '--draft=false', '--prerelease=' + str('-' in version).lower()])
if __name__ == '__main__':
    try: main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print('Release failed: ' + str(error), file=sys.stderr); sys.exit(1)
