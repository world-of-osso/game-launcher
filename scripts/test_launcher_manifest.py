"""Publisher contract includes legacy download consumers and mixed platform versions."""
import hashlib
import importlib.util
import tempfile
import unittest
import shutil
import subprocess
from unittest.mock import patch
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


class PublishPermissionsTests(unittest.TestCase):
    def test_restrictive_uploads_are_readable_before_manifest_is_published(self):
        for platform_key in ('linux-x86_64', 'windows-x86_64'):
            with self.subTest(platform=platform_key), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / 'Cargo.toml').write_text('[package]\nversion = "0.4.0"\n')
                artifact = root / 'launcher'
                artifact.write_bytes(b'portable executable')
                artifact.chmod(0o700)
                remote = root / 'downloads'
                local_run = subprocess.run

                def execute_locally(command, **kwargs):
                    if command[0] == 'scp':
                        destination = command[2].split(':', 1)[1]
                        destination = Path(destination.replace(publisher.REMOTE, str(remote)))
                        shutil.copy2(command[1], destination)
                        destination.chmod(0o700)
                        return subprocess.CompletedProcess(command, 0)
                    if command[2].startswith('if test'):
                        return subprocess.CompletedProcess(command, 0, stdout='{}')
                    if command[2] == 'mv':
                        self.assertEqual((remote / 'launcher.json.next').stat().st_mode & 0o777, 0o644)
                        for uploaded in remote.iterdir():
                            self.assertEqual(uploaded.stat().st_mode & 0o777, 0o644)
                    local_command = [arg.replace(publisher.REMOTE, str(remote)) for arg in command[2:]]
                    return local_run(local_command, **kwargs)

                argv = ['publish-launcher.py', '--artifact', str(artifact), '--platform', platform_key]
                with patch.object(publisher, '__file__', str(root / 'scripts/publish-launcher.py')), patch('sys.argv', argv), patch.object(publisher.subprocess, 'run', side_effect=execute_locally):
                    publisher.main()
                self.assertEqual(artifact.stat().st_mode & 0o777, 0o700)
                for uploaded in remote.iterdir():
                    self.assertEqual(uploaded.stat().st_mode & 0o777, 0o644)


if __name__ == '__main__':
    unittest.main()
