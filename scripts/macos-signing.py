#!/usr/bin/env python3
"""Import credentials into a disposable runner keychain, then remove it in an always() step."""
import base64, json, os, pathlib, secrets, shlex, subprocess, sys

def call(args):
    result = subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if result.returncode: raise RuntimeError('macOS signing setup failed in ' + args[0])

def main():
    folder = pathlib.Path(os.environ['RUNNER_TEMP'])
    keychain = folder / 'loramesh-signing.keychain-db'
    original = folder / 'loramesh-keychains.json'
    certificate = folder / 'loramesh-signing.p12'
    if sys.argv[1] == 'cleanup':
        try:
            if original.exists(): call(['security', 'list-keychains', '-d', 'user', '-s', *json.loads(original.read_text())])
        finally:
            if keychain.exists(): call(['security', 'delete-keychain', str(keychain)])
            certificate.unlink(missing_ok=True); original.unlink(missing_ok=True)
        return
    required = ['MACOS_CERTIFICATE_P12_BASE64', 'MACOS_CERTIFICATE_PASSWORD', 'MACOS_SIGN_IDENTITY', 'MACOS_INSTALLER_IDENTITY', 'APPLE_ID', 'APPLE_TEAM_ID', 'APPLE_APP_PASSWORD']
    missing = [key for key in required if not os.environ.get(key)]
    if missing: raise ValueError('Signing enabled but missing secrets: ' + ', '.join(missing))
    previous = shlex.split(subprocess.check_output(['security', 'list-keychains', '-d', 'user'], text=True))
    original.write_text(json.dumps(previous))
    password = secrets.token_urlsafe(32)
    certificate.write_bytes(base64.b64decode(os.environ['MACOS_CERTIFICATE_P12_BASE64'], validate=True)); certificate.chmod(0o600)
    try:
        call(['security', 'create-keychain', '-p', password, str(keychain)])
        call(['security', 'set-keychain-settings', '-lut', '21600', str(keychain)])
        call(['security', 'unlock-keychain', '-p', password, str(keychain)])
        call(['security', 'import', str(certificate), '-k', str(keychain), '-P', os.environ['MACOS_CERTIFICATE_PASSWORD'], '-T', '/usr/bin/codesign', '-T', '/usr/bin/productsign'])
        call(['security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password, str(keychain)])
        call(['security', 'list-keychains', '-d', 'user', '-s', str(keychain), *previous])
        call(['xcrun', 'notarytool', 'store-credentials', 'loramesh-notary', '--keychain', str(keychain), '--apple-id', os.environ['APPLE_ID'], '--team-id', os.environ['APPLE_TEAM_ID'], '--password', os.environ['APPLE_APP_PASSWORD']])
        values = {'MACOS_KEYCHAIN': str(keychain), 'MACOS_NOTARY_PROFILE': 'loramesh-notary', 'MACOS_SIGN_IDENTITY': os.environ['MACOS_SIGN_IDENTITY'], 'MACOS_INSTALLER_IDENTITY': os.environ['MACOS_INSTALLER_IDENTITY']}
        if any('\n' in value or '\r' in value for value in values.values()): raise ValueError('Invalid newline in signing configuration')
        with open(os.environ['GITHUB_ENV'], 'a') as env:
            for key, value in values.items(): env.write(key + '=' + value + '\n')
    finally: certificate.unlink(missing_ok=True)
if __name__ == '__main__':
    try: main()
    except (ValueError, RuntimeError, OSError, KeyError, subprocess.CalledProcessError):
        print('macOS signing setup/cleanup failed; check required secrets and certificate identities. Credentials were not logged.', file=sys.stderr)
        sys.exit(1)
