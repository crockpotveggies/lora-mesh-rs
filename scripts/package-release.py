#!/usr/bin/env python3
"""Native, versioned installers. No deployment keys/state or automatic service startup."""
import argparse
import gzip
import hashlib
import json
import os
import platform
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?')

def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], cwd=ROOT, check=True, **kwargs)

def output(*args):
    return run(args, capture_output=True, text=True).stdout.strip()

def version(tag=None):
    value = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    if not SEMVER.fullmatch(value):
        raise ValueError('Package version must be numeric semver, optionally with a prerelease suffix')
    if tag is not None and tag != 'v' + value:
        raise ValueError(f'Release tag must equal v{value}, got {tag!r}')
    return value

def identity(host):
    arch = host.split('-')[0]
    if arch not in ('x86_64', 'aarch64'):
        raise ValueError(f'Unsupported installer architecture: {host}')
    if sys.platform == 'win32' and host != 'x86_64-pc-windows-msvc':
        raise ValueError('Windows installer requires the x86_64-pc-windows-msvc toolchain')
    return arch


def third_party(stage, host):
    metadata = json.loads(output('cargo', 'metadata', '--locked', '--format-version', '1', '--filter-platform', host))
    resolved = {node['id'] for node in metadata['resolve']['nodes']}
    notices = stage / 'THIRD_PARTY'; notices.mkdir()
    index = []
    for package in metadata['packages']:
        if package['id'] not in resolved or package['source'] is None: continue
        source = Path(package['manifest_path']).parent
        folder = notices / (package['name'] + '-' + package['version']); folder.mkdir()
        candidates = set()
        for p in source.iterdir():
            if p.is_file() and p.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT')):
                candidates.add(p)
        if package.get('license_file'):
            candidates.add(source / package['license_file'])
        if not candidates:
            fallback = ROOT / 'packaging/licenses' / (package['name'] + '-' + package['version'] + '-LICENSE')
            if fallback.exists():
                record = json.loads((ROOT / 'packaging/licenses/sources.json').read_text())[fallback.name]
                if hashlib.sha256(fallback.read_bytes()).hexdigest() != record['sha256']: raise ValueError('Vendored notice checksum mismatch')
                candidates.add(fallback)
        if not candidates: raise ValueError('Missing dependency license notice: ' + package['name'] + ' ' + package['version'])
        for path in sorted(candidates): shutil.copy2(path, folder / path.name)
        index.append({'name': package['name'], 'version': package['version'], 'license': package.get('license'), 'repository': package.get('repository'), 'authors': package.get('authors', []), 'notices': [p.name for p in sorted(candidates)]})
    (notices / 'index.json').write_text(json.dumps(index, indent=2) + '\n')

def manifest(stage):
    hashes = {p.relative_to(stage).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in sorted(stage.rglob('*')) if p.is_file()}
    (stage / 'manifest.json').write_text(json.dumps(hashes, indent=2) + '\n')

def portable(stage, dist, name, epoch):
    if sys.platform == 'win32':
        dest = dist / (name + '.zip')
        # Match tar's release epoch instead of inherited Cargo registry mtimes.
        # ZIP stores only 1980..2107 and has two-second timestamp precision.
        date_time = time.gmtime(min(max(epoch, 315532800), 4354819198))[:6]
        with zipfile.ZipFile(dest, 'w', zipfile.ZIP_DEFLATED) as archive:
            for p in sorted(stage.rglob('*')):
                if not p.is_file(): continue
                info = zipfile.ZipInfo(name + '/' + p.relative_to(stage).as_posix(), date_time)
                info.compress_type = zipfile.ZIP_DEFLATED
                info.create_system = 3  # Preserve Unix mode bits consistently on every host.
                stat = p.stat()
                info.external_attr = (stat.st_mode & 0xffff) << 16
                info.file_size = stat.st_size
                with p.open('rb') as source, archive.open(info, 'w') as target:
                    shutil.copyfileobj(source, target)
    else:
        dest = dist / (name + '.tar.gz')
        def normalize(info):
            info.uid = info.gid = 0; info.uname = info.gname = ''; info.mtime = epoch
            return info
        with dest.open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode='w') as archive:
                archive.add(stage, arcname=name, filter=normalize)
    return dest

def notarize(path):
    profile = os.environ.get('MACOS_NOTARY_PROFILE')
    if profile:
        credentials = ['--keychain-profile', profile]
        if os.environ.get('MACOS_KEYCHAIN'): credentials += ['--keychain', os.environ['MACOS_KEYCHAIN']]
        run(['xcrun', 'notarytool', 'submit', path, *credentials, '--wait', '--timeout', '30m'])
        run(['xcrun', 'stapler', 'staple', path])
        run(['xcrun', 'stapler', 'validate', path])

def macos(stage, dist, name, ver, temporary):
    root = temporary / 'pkg-root'
    bindir = root / 'usr/local/bin'; bindir.mkdir(parents=True)
    share = root / 'usr/local/share/loramesh'; share.mkdir(parents=True)
    for p in (stage / 'bin').iterdir(): shutil.copy2(p, bindir / p.name)
    for p in stage.iterdir():
        if p.name != 'bin':
            if p.is_dir(): shutil.copytree(p, share / p.name)
            else: shutil.copy2(p, share / p.name)
    shutil.copy2(ROOT / 'packaging/macos/uninstall.sh', share / 'uninstall.sh')
    (share / 'uninstall.sh').chmod(0o755)
    pkg = dist / (name + '.pkg')
    sign = ['--sign', os.environ['MACOS_INSTALLER_IDENTITY']] if os.environ.get('MACOS_INSTALLER_IDENTITY') else []
    run(['pkgbuild', '--root', root, '--identifier', 'org.loramesh.tools', '--version', ver.split('-')[0], '--install-location', '/', '--ownership', 'recommended', *sign, pkg])
    notarize(pkg)
    contents = temporary / 'dmg'; contents.mkdir()
    shutil.copy2(pkg, contents / pkg.name)
    shutil.copy2(stage / 'INSTALL.txt', contents / 'INSTALL.txt')
    dmg = dist / (name + '.dmg')
    run(['hdiutil', 'create', '-ov', '-format', 'UDZO', '-volname', 'LoRa Mesh ' + ver, '-srcfolder', contents, dmg])
    if os.environ.get('MACOS_SIGN_IDENTITY'):
        run(['codesign', '--force', '--timestamp', '--sign', os.environ['MACOS_SIGN_IDENTITY'], dmg])
    notarize(dmg)
    return [pkg, dmg]

def linux(stage, dist, name, ver, arch, temporary):
    root = temporary / 'deb-root'
    bindir = root / 'usr/bin'; bindir.mkdir(parents=True)
    share = root / 'usr/share/loramesh'; share.mkdir(parents=True)
    for p in (stage / 'bin').iterdir(): shutil.copy2(p, bindir / p.name)
    for p in stage.iterdir():
        if p.name != 'bin':
            if p.is_dir(): shutil.copytree(p, share / p.name)
            else: shutil.copy2(p, share / p.name)
    control = root / 'DEBIAN'; control.mkdir()
    deb_version = ver.replace('-', '~', 1)
    (control / 'control').write_text(f'''Package: loramesh
Version: {deb_version}
Architecture: {'amd64' if arch == 'x86_64' else 'arm64'}
Maintainer: LoRa Mesh contributors <crockpotveggies@users.noreply.github.com>
Depends: libc6 (>= {platform.libc_ver()[1]}), libudev1, libgcc-s1, iproute2
Section: net
Priority: optional
Description: Secure experimental IPv4 mesh over LoStik LoRa radios
 Includes the daemon, provisioning and deterministic simulation tools.
 No daemon is enabled automatically. Configure identities before use.
''')
    dest = dist / (name + '.deb')
    run(['dpkg-deb', '--root-owner-group', '--build', root, dest])
    return [dest]

def windows(stage, dist, name, ver):
    compiler = os.environ.get('ISCC') or shutil.which('ISCC.exe')
    if not compiler:
        candidate = Path(os.environ.get('ProgramFiles(x86)', 'C:/Program Files (x86)')) / 'Inno Setup 6/ISCC.exe'
        if candidate.exists(): compiler = str(candidate)
    if not compiler: raise ValueError('Install Inno Setup 6 or set ISCC to ISCC.exe')
    run([compiler, '/DVersion=' + ver, '/DNumericVersion=' + ver.split('-')[0] + '.0', '/DSourceDir=' + str(stage), '/DOutputDir=' + str(dist), '/DOutputName=' + name + '-setup', ROOT / 'packaging/windows/installer.iss'])
    return [dist / (name + '-setup.exe')]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', help='Require vVERSION to match Cargo.toml; never creates a tag')
    parser.add_argument('--check-version', action='store_true')
    parser.add_argument('--skip-build', action='store_true', help='Package existing native release binaries')
    parser.add_argument('--portable-only', action='store_true')
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    args = parser.parse_args()
    ver = version(args.tag)
    if args.check_version: print(ver); return
    platforms = json.loads((ROOT / 'packaging/platforms.json').read_text())
    spec = platforms[sys.platform]
    compiler = output('rustc', '-Vv')
    host = next(line.split(': ', 1)[1] for line in compiler.splitlines() if line.startswith('host: '))
    arch = identity(host)
    if sys.platform == 'darwin' and bool(os.environ.get('MACOS_SIGN_IDENTITY')) != bool(os.environ.get('MACOS_INSTALLER_IDENTITY')):
        raise ValueError('Provide both Application and Installer signing identities, or neither')
    if sys.platform == 'darwin' and os.environ.get('MACOS_NOTARY_PROFILE') and not all(os.environ.get(k) for k in ('MACOS_SIGN_IDENTITY', 'MACOS_INSTALLER_IDENTITY')):
        raise ValueError('Notarization requires both Developer ID Application and Installer identities')
    if not args.skip_build:
        build_args = ['cargo', 'build', '--locked', '--release']
        environment = os.environ.copy()
        if sys.platform == 'win32':
            build_args += ['--target', host]
            environment['RUSTFLAGS'] = environment.get('RUSTFLAGS', '') + ' -C target-feature=+crt-static'
        run([*build_args, *[a for b in spec['binaries'] for a in ('--bin', b)]], env=environment)
    dist = args.output.resolve(); dist.mkdir(parents=True, exist_ok=True)
    name = 'loramesh-' + ver + '-' + host
    epoch = int(os.environ.get('SOURCE_DATE_EPOCH') or output('git', 'show', '-s', '--format=%ct', 'HEAD'))
    with tempfile.TemporaryDirectory(prefix='loramesh-package-') as directory:
        temporary = Path(directory); stage = temporary / name; (stage / 'bin').mkdir(parents=True)
        target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
        if not target.is_absolute(): target = ROOT / target
        for binary in spec['binaries']:
            filename = binary + ('.exe' if sys.platform == 'win32' else '')
            source = (target / host if sys.platform == 'win32' else target) / 'release' / filename
            # Refuse stale --skip-build binaries from another version.
            actual = output(str(source.resolve()), '--version')
            if actual != binary + ' ' + ver: raise ValueError(f'Binary version mismatch: {actual}')
            dest = stage / 'bin' / filename; shutil.copy2(source, dest); dest.chmod(0o755)
            if sys.platform == 'darwin' and os.environ.get('MACOS_SIGN_IDENTITY'):
                run(['codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', os.environ['MACOS_SIGN_IDENTITY'], dest])
                run(['codesign', '--verify', '--strict', dest])
        for filename in ['LICENSE', 'README.md', 'Cargo.lock']:
            shutil.copy2(ROOT / filename, stage / filename)
        shutil.copytree(ROOT / 'docs', stage / 'docs')
        shutil.copytree(ROOT / 'scenarios', stage / 'scenarios')
        shutil.copy2(ROOT / 'packaging/INSTALL.txt', stage / 'INSTALL.txt')
        metadata = {'version': ver, 'revision': output('git', 'rev-parse', 'HEAD'), 'dirty': bool(output('git', 'status', '--porcelain')), 'compiler': compiler, 'target': host, 'binaries': spec['binaries'], 'networking': spec['networking'], 'signed': bool(sys.platform == 'darwin' and os.environ.get('MACOS_SIGN_IDENTITY')), 'notarized_installers': bool(sys.platform == 'darwin' and os.environ.get('MACOS_NOTARY_PROFILE'))}
        (stage / 'build.json').write_text(json.dumps(metadata, indent=2) + '\n')
        third_party(stage, host)
        manifest(stage)
        artifacts = []
        if not args.portable_only:
            if sys.platform == 'darwin': artifacts += macos(stage, dist, name, ver, temporary)
            elif sys.platform == 'linux': artifacts += linux(stage, dist, name, ver, arch, temporary)
            elif sys.platform == 'win32': artifacts += windows(stage, dist, name, ver)
        artifacts.append(portable(stage, dist, name, epoch))
        # Sidecar build metadata makes platform/signing limits visible without installing.
        metadata_path = dist / (name + '-build.json'); shutil.copy2(stage / 'build.json', metadata_path); artifacts.append(metadata_path)
    for artifact in artifacts:
        digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
        artifact.with_name(artifact.name + '.sha256').write_text(digest + '  ' + artifact.name + '\n')
        print(artifact)

if __name__ == '__main__':
    try: main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        # Do not echo complete subprocess arguments: signing commands may use credentials.
        print('Packaging failed: ' + (f'{error.cmd[0]} exited {error.returncode}' if isinstance(error, subprocess.CalledProcessError) else str(error)), file=sys.stderr)
        sys.exit(1)
