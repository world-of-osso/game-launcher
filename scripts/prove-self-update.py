#!/usr/bin/env python3
"""Owned loopback old->new executable proof; Windows optionally proves mounted UI/recheck."""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import threading
import time
import tomllib


def build_versions(root, proof, gui):
    source = proof / 'source'
    shutil.copytree(root, source, ignore=shutil.ignore_patterns('.git', 'target', '__pycache__'), dirs_exist_ok=True)
    # Fixture snapshots may live beneath another Cargo workspace (evidence tree).
    original_toml = (source / 'Cargo.toml').read_text() + '\n[workspace]\n'
    original_lock = (source / 'Cargo.lock').read_text()
    version = tomllib.loads(original_toml)['package']['version']
    extension = '.exe' if os.name == 'nt' else ''
    binaries = []
    for candidate in ['0.3.0', version]:
        (source / 'Cargo.toml').write_text(original_toml.replace(f'version = "{version}"', f'version = "{candidate}"', 1))
        (source / 'Cargo.lock').write_text(original_lock.replace(f'name = "game-launcher"\nversion = "{version}"', f'name = "game-launcher"\nversion = "{candidate}"', 1))
        command = ['cargo', 'build', '--locked', '--no-default-features', '--target-dir', str(proof / 'target'), '-j4']
        if gui:
            command += ['--features', 'desktop']
        subprocess.run(command, cwd=source, check=True)
        destination = proof / f'launcher-{candidate}{extension}'
        shutil.copy2(proof / 'target/debug' / f'game-launcher{extension}', destination)
        binaries.append(destination)
    return version, binaries


def owned_windows_pids(executable):
    escaped = str(executable).replace("'", "''")
    script = f"@(Get-Process game-launcher -ErrorAction SilentlyContinue | Where-Object {{$_.Path -eq '{escaped}'}} | ForEach-Object {{$_.Id}}) | ConvertTo-Json -Compress"
    result = subprocess.run(['powershell.exe', '-NoProfile', '-Command', script], check=True, capture_output=True, text=True)
    value = json.loads(result.stdout or '[]')
    return value if isinstance(value, list) else [value]


def wait_until(predicate, seconds, label):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise RuntimeError(f'Timed out: {label}')


def prove(root, proof, gui, periodic):
    version, (old, new) = build_versions(root, proof, gui)
    install = proof / 'install'
    install.mkdir(exist_ok=True)
    executable = install / ('game-launcher.exe' if os.name == 'nt' else 'game-launcher')
    shutil.copy2(old, executable)
    game = proof / 'game'
    counts = {'launcher': 0, 'game': 0}
    bodies = [b'first game file', b'second game file after periodic refresh']

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == '/launcher.json':
                counts['launcher'] += 1
                platform = 'windows-x86_64' if os.name == 'nt' else 'linux-x86_64'
                body = json.dumps({'version': version, 'platforms': {platform: {'version': version, 'url': f'http://127.0.0.1:{self.server.server_port}/new', 'size': new.stat().st_size, 'sha256': hashlib.sha256(new.read_bytes()).hexdigest(), 'file': new.name}}}).encode()
            elif self.path == '/new':
                body = new.read_bytes()
            elif self.path == '/manifest.json':
                counts['game'] += 1
                payload = bodies[min(counts['game'] - 1, 1)]
                body = json.dumps({'version': str(counts['game']), 'files': [{'path': 'proof.txt', 'size': len(payload), 'sha256': hashlib.sha256(payload).hexdigest()}]}).encode()
            elif self.path == '/proof.txt':
                body = bodies[min(counts['game'] - 1, 1)]
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    environment = dict(os.environ, GAME_LAUNCHER_MANIFEST_URL=f'http://127.0.0.1:{server.server_port}/launcher.json', GAME_LAUNCHER_BASE_URL=f'http://127.0.0.1:{server.server_port}', GAME_LAUNCHER_GAME_DIR=str(game))
    environment.pop('GAME_LAUNCHER_START_ACK', None)
    arguments = [] if gui else ['self-update']
    log_path = proof / 'process.log'
    process = None
    try:
        with log_path.open('w') as log:
            process = subprocess.Popen([str(executable), *arguments], env=environment, stdout=log, stderr=log)
            if process.wait(timeout=90) != 0:
                raise RuntimeError(f'Old launcher update failed: {log_path.read_text()}')
            ready = Path(str(executable) + '.ready')
            wait_until(lambda: ready.exists() and ready.read_text() == version, 30, 'new launcher startup receipt')
            assert executable.read_bytes() == new.read_bytes(), 'installed executable differs from new build'
            assert Path(str(executable) + '.old').read_bytes() == old.read_bytes(), 'old launcher backup not preserved'
            assert subprocess.check_output([str(executable), '--self-check'], text=True).strip() == version
            if gui:
                assert os.name == 'nt', 'GUI proof runner owns native Windows processes only'
                wait_until(lambda: (game / 'proof.txt').exists(), 30, 'mounted new UI game sync')
                assert (game / 'proof.txt').read_bytes() == bodies[0]
                assert owned_windows_pids(executable), 'no live restarted desktop process'
                if periodic:
                    wait_until(lambda: counts['game'] >= 2 and (game / 'proof.txt').read_bytes() == bodies[1], 330, 'periodic game manifest refresh/download')
                    assert counts['launcher'] >= 3, 'launcher manifest was not periodically rechecked'
                    assert not Path(str(executable) + '.old').exists(), 'confirmed backup was not cleaned on next check'
            else:
                wait_until(lambda: counts['launcher'] >= 2, 10, 'restarted CLI manifest check')
                subprocess.run([str(executable), 'self-update'], env=environment, check=True, timeout=30)
                assert not Path(str(executable) + '.old').exists(), 'confirmed backup was not cleaned on next start'
            print(json.dumps({'result': 'PASS', 'old_version': '0.3.0', 'new_version': version, 'gui': gui, 'periodic': periodic, 'requests': counts, 'installed_sha256': hashlib.sha256(executable.read_bytes()).hexdigest(), 'process_log': str(log_path)}, indent=2), flush=True)
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait()
        if gui and os.name == 'nt':
            for pid in owned_windows_pids(executable):
                os.kill(pid, signal.SIGTERM)
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--proof-dir', type=Path, required=True)
    parser.add_argument('--gui', action='store_true')
    parser.add_argument('--periodic', action='store_true')
    args = parser.parse_args()
    args.proof_dir.mkdir(parents=True, exist_ok=True)
    prove(Path(__file__).resolve().parents[1], args.proof_dir.resolve(), args.gui, args.periodic)
