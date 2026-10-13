"""Publisher contract includes legacy download consumers and mixed platform versions."""
import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('publisher', Path(__file__).with_name('publish-launcher.py'))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class ManifestTests(unittest.TestCase):
    def test_platform_versions_and_legacy_links_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory) / 'launcher.exe'
            artifact.write_bytes(b'new executable')
            existing = {'version': '0.3.0', 'platforms': {'linux-x86_64': {'file': 'game-launcher-0.3.0-linux-x86_64', 'sha256': 'a' * 64}}, 'assets': [{'name': 'existing installer'}]}
            manifest = publisher.merge_manifest(existing, '0.4.0', 'windows-x86_64', artifact, {'linux-x86_64': 123})
            linux = manifest['platforms']['linux-x86_64']
            windows = manifest['platforms']['windows-x86_64']
            self.assertEqual(linux['version'], '0.3.0')
            self.assertEqual(linux['size'], 123)
            self.assertEqual(linux['file'], existing['platforms']['linux-x86_64']['file'])
            self.assertEqual(windows, {'version': '0.4.0', 'file': 'game-launcher-0.4.0-windows-x86_64.exe', 'url': 'https://worldofosso.com/downloads/game-launcher-0.4.0-windows-x86_64.exe', 'sha256': hashlib.sha256(b'new executable').hexdigest(), 'size': 14})
            self.assertEqual(manifest['version'], '0.4.0')
            self.assertEqual(manifest['assets'], existing['assets'])
            self.assertNotIn('version', existing['platforms']['linux-x86_64'])

    def test_existing_platform_version_does_not_follow_global_version(self):
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory) / 'launcher'
            artifact.write_bytes(b'linux')
            entry = {'version': '0.3.1', 'url': 'https://example.com/windows.exe', 'sha256': 'b' * 64, 'size': 10, 'file': 'game-launcher-0.3.1-windows-x86_64.exe'}
            manifest = publisher.merge_manifest({'version': '0.4.0', 'platforms': {'windows-x86_64': entry}}, '0.4.1', 'linux-x86_64', artifact, {})
            self.assertEqual(manifest['platforms']['windows-x86_64'], entry)


if __name__ == '__main__':
    unittest.main()
