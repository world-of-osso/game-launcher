#!/usr/bin/env python3
"""Build/publish one launcher platform, then atomically publish its manifest."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import platform
import re
import shlex
import subprocess
import tomllib

REMOTE = '/var/lib/osso-website/downloads'
BASE_URL = 'https://worldofosso.com/downloads'


def merge_manifest(existing, version, platform_key, artifact, legacy_sizes):
    manifest = copy.deepcopy(existing)
    platforms = manifest.setdefault('platforms', {})
    for old_key, entry in platforms.items():
        if old_key == platform_key:
            continue
        entry.setdefault('version', existing['version'])
        entry.setdefault('url', f"{BASE_URL}/{entry['file']}")
        if 'size' not in entry:
            entry['size'] = legacy_sizes[old_key]
    extension = '.exe' if platform_key.startswith('windows') else ''
    filename = f'game-launcher-{version}-{platform_key}{extension}'
    with artifact.open('rb') as binary:
        digest = hashlib.file_digest(binary, 'sha256').hexdigest()
    platforms[platform_key] = {
        'version': version, 'url': f'{BASE_URL}/{filename}',
        'sha256': digest, 'size': artifact.stat().st_size, 'file': filename,
    }
    manifest['version'] = version
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifact', type=Path, help='Already-built portable executable (not installer)')
    parser.add_argument('--platform', choices=('linux-x86_64', 'windows-x86_64'))
    parser.add_argument('--dry-run', action='store_true', help='Read existing manifest and stage locally, but do not upload')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / 'Cargo.toml').read_text())['package']['version']
    key = args.platform or {'Linux': 'linux-x86_64', 'Windows': 'windows-x86_64'}.get(platform.system())
    if key is None:
        parser.error('Unsupported launcher platform')
    if args.artifact is None:
        subprocess.run(['cargo', 'build', '--locked', '--release'], cwd=root, check=True)
    artifact = args.artifact or root / 'target/release' / ('game-launcher.exe' if key.startswith('windows') else 'game-launcher')
    existing_result = subprocess.run(['ssh', 'sakuin', f'if test -f {REMOTE}/launcher.json; then cat {REMOTE}/launcher.json; else printf "{{}}"; fi'], check=True, capture_output=True, text=True)
    existing = json.loads(existing_result.stdout)
    legacy_sizes = {}
    for old_key, entry in existing.get('platforms', {}).items():
        if old_key == key or 'size' in entry:
            continue
        filename = entry['file']
        if not re.fullmatch(r'game-launcher-[A-Za-z0-9._-]+', filename):
            raise ValueError(f'Unsafe published launcher filename: {filename}')
        result = subprocess.run(['ssh', 'sakuin', 'stat', '-c', '%s', f'{REMOTE}/{filename}'], check=True, capture_output=True, text=True)
        legacy_sizes[old_key] = int(result.stdout)
    manifest = merge_manifest(existing, version, key, artifact, legacy_sizes)
    stage = root / 'target/launcher-release'
    stage.mkdir(parents=True, exist_ok=True)
    manifest_path = stage / 'launcher.json'
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    filename = manifest['platforms'][key]['file']
    commands = [
        ['ssh', 'sakuin', 'mkdir', '-p', REMOTE],
        ['scp', str(artifact), f'sakuin:{REMOTE}/{filename}'],
        ['scp', str(manifest_path), f'sakuin:{REMOTE}/launcher.json.next'],
        ['ssh', 'sakuin', 'mv', f'{REMOTE}/launcher.json.next', f'{REMOTE}/launcher.json'],
    ]
    for command in commands:
        print(shlex.join(command), flush=True)
        if not args.dry_run:
            subprocess.run(command, check=True)


if __name__ == '__main__':
    main()
