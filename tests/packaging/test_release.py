import hashlib, importlib.util, json, pathlib, tempfile, unittest
ROOT = pathlib.Path(__file__).resolve().parents[2]
def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    result = importlib.util.module_from_spec(spec); spec.loader.exec_module(result); return result
package = module('package-release'); publish = module('publish-release'); verifier = module('verify-package')
class ReleaseTests(unittest.TestCase):
    def test_tag_must_match_exact_version(self):
        version = package.version()
        self.assertEqual(package.version('v' + version), version)
        for tag in ('main', version, 'v9.9.9', 'v' + version + ';echo unsafe'):
            with self.assertRaises(ValueError): package.version(tag)
    def test_platform_contract(self):
        platforms = json.loads((ROOT / 'packaging/platforms.json').read_text())
        self.assertNotIn('loramesh-mesh', platforms['win32']['binaries'])
        self.assertIn('loramesh-mesh', platforms['linux']['binaries'])
        self.assertIn('loramesh-mesh-sim', platforms['win32']['binaries'])
        self.assertEqual(set(publish.TARGETS), {'aarch64-apple-darwin', 'x86_64-apple-darwin', 'x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'})
    def fixtures(self, directory):
        for target, extensions in publish.TARGETS.items():
            name = 'loramesh-1.2.3-' + target
            for ext in extensions: (directory / (name + ext)).write_bytes(b'fake-installer')
            (directory / (name + '-build.json')).write_text(json.dumps({'version':'1.2.3','target':target,'revision':'abc','dirty':False}))
        (directory / 'loramesh-1.2.3-source.tar.gz').write_bytes(b'fake-source-archive')
        for path in list(directory.iterdir()):
            (directory / (path.name + '.sha256')).write_text(hashlib.sha256(path.read_bytes()).hexdigest() + '  ' + path.name + '\n')
    def test_complete_set_and_checksum_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = pathlib.Path(tmp); self.fixtures(directory)
            files, builds = publish.validate(directory, '1.2.3', 'abc')
            self.assertEqual(len(builds), 5); self.assertEqual(len(files), 36)
            artifact = directory / 'loramesh-1.2.3-x86_64-pc-windows-msvc-setup.exe'
            artifact.write_bytes(b'tampered')
            with self.assertRaises(ValueError): publish.validate(directory, '1.2.3', 'abc')
    def test_missing_extra_wrong_commit_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = pathlib.Path(tmp); self.fixtures(directory)
            with self.assertRaises(ValueError): publish.validate(directory, '1.2.3', 'wrong')
            extra = directory / 'secret.key'; extra.write_text('must not publish')
            with self.assertRaises(ValueError): publish.validate(directory, '1.2.3', 'abc')
            extra.unlink(); next(directory.glob('*.pkg')).unlink()
            with self.assertRaises(FileNotFoundError): publish.validate(directory, '1.2.3', 'abc')
    def test_dirty_build_cannot_publish_even_with_valid_checksums(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = pathlib.Path(tmp); self.fixtures(directory)
            path = next(directory.glob('*-build.json'))
            build = json.loads(path.read_text()); build['dirty'] = True
            path.write_text(json.dumps(build))
            path.with_name(path.name + '.sha256').write_text(hashlib.sha256(path.read_bytes()).hexdigest() + '  ' + path.name + '\n')
            with self.assertRaises(ValueError): publish.validate(directory, '1.2.3', 'abc')
    def test_manifest_detects_corruption_before_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); (root / 'tool').write_text('original'); package.manifest(root)
            (root / 'tool').write_text('changed')
            with self.assertRaises(ValueError): verifier.verify(root)
    def test_manifest_cannot_escape(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp); (root / 'manifest.json').write_text(json.dumps({'../outside':'0'*64}))
            with self.assertRaises(ValueError): verifier.verify(root)
if __name__ == '__main__': unittest.main()
