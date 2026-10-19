import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path('/evidence')
OUT = ROOT / 'fault-envelope-capture'
OUT.mkdir()
GATEWAY = ROOT / 'bin/sentry-only-c405064-a3-gateway'
HARNESS = ROOT / 'bin/sentry-only-c405064-a3-harness'
expected = json.loads((ROOT / 'fixed-checks-attempt3/sentry-only-c405064-a3-selection.json').read_text())
for role, path in [('gateway', GATEWAY), ('harness', HARNESS)]:
    assert hashlib.sha256(path.read_bytes()).hexdigest() == expected[role]['sha256']
limits = {name: (Path('/sys/fs/cgroup') / name).read_text() for name in
          ['cpu.max', 'cpuset.cpus.effective', 'memory.max', 'memory.swap.max', 'pids.max']}
(OUT / 'kernel-limits.json').write_text(json.dumps(limits, indent=2) + '\n')
assert limits['cpu.max'].strip() == '200000 100000'
assert limits['cpuset.cpus.effective'].strip() == '12-13'
assert limits['memory.max'].strip() == '3221225472'
assert limits['memory.swap.max'].strip() == '0'


def free_port():
    with socket.socket() as server:
        server.bind(('127.0.0.1', 0))
        return server.getsockname()[1]


def request(port, content):
    with socket.create_connection(('127.0.0.1', port), timeout=3) as peer:
        peer.settimeout(3)
        peer.sendall(content)
        data = bytearray()
        while True:
            chunk = peer.recv(8192)
            if not chunk:
                return bytes(data)
            data.extend(chunk)
            assert len(data) <= 65536


def collect(server):
    server.settimeout(8)
    peer, address = server.accept()
    with peer:
        assert address[0] == '127.0.0.1'
        peer.settimeout(3)
        wire = bytearray()
        while b'\r\n\r\n' not in wire:
            chunk = peer.recv(8192)
            assert chunk
            wire.extend(chunk)
            assert len(wire) <= 65536
        head, body = bytes(wire).split(b'\r\n\r\n', 1)
        assert head.startswith(b'POST /api/1/envelope/ HTTP/1.1\r\n')
        length = int(next(line.split(b':', 1)[1].strip() for line in head.split(b'\r\n')
                          if line.lower().startswith(b'content-length:')))
        assert 0 < length <= 32768
        while len(body) < length:
            chunk = peer.recv(length - len(body))
            assert chunk
            body += chunk
        assert len(body) == length
        for canary in [b'hello', b'DOCUMENT_SECRET', b'PRIVATE_COOKIE', b'baggage']:
            assert canary not in body
        envelope, item, payload = body.split(b'\n', 2)
        json.loads(envelope)
        assert json.loads(item)['type'] == 'event'
        event = json.loads(payload)
        assert event['tags']['failure.class'] == 'tika_failure'
        assert event['tags']['ocr.phase'] == 'gateway'
        assert 'trace_id' not in event['tags'] and 'span_id' not in event['tags']
        assert all(key not in event for key in ['request', 'user', 'server_name'])
        (OUT / 'sentry.envelope').write_bytes(body)
        (OUT / 'sentry-event.json').write_text(json.dumps(event, indent=2) + '\n')
        peer.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n')
        return event


with tempfile.TemporaryDirectory(prefix='indra-sentry-wire-') as temporary, socket.socket() as collector:
    directory = Path(temporary)
    collector.bind(('127.0.0.1', 0))
    collector.listen(1)
    public = free_port()
    backend = free_port()
    assert public != backend
    command = [str(GATEWAY), '--listen', f'127.0.0.1:{public}', '--backend', f'127.0.0.1:{backend}',
               '--java', str(HARNESS), '--jar', str(directory / 'fixture.jar'),
               '--tika-config', str(directory / 'fixture.xml'), '--max-active', '1',
               '--max-connections', '8', '--max-request-bytes', '1024', '--max-response-bytes', '1024',
               '--header-timeout-seconds', '1', '--upload-timeout-seconds', '2',
               '--request-timeout-seconds', '2', '--client-stall-seconds', '1',
               '--startup-timeout-seconds', '3', '--shutdown-timeout-seconds', '4',
               '--child-shutdown-seconds', '1', 'serve']
    environment = {'GATEWAY_TEST_DIR': str(directory), 'GATEWAY_TEST_WRONG_VERSION': 'false',
                   'TIKA__TELEMETRY__SENTRY_DSN': f'http://public@127.0.0.1:{collector.getsockname()[1]}/1'}
    (OUT / 'gateway-command.json').write_text(json.dumps({'command': command, 'environment': environment,
        'producing_source': 'c405064013b4fcd146f8992e90b49d3cef873a8d',
        'elf_sha256': {role: expected[role]['sha256'] for role in ['gateway', 'harness']}}, indent=2) + '\n')
    with (OUT / 'gateway.stderr').open('wb') as errors, concurrent.futures.ThreadPoolExecutor(max_workers=1) as threads:
        captured = threads.submit(collect, collector)
        gateway = subprocess.Popen(command, env=environment, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL, stderr=errors)
        try:
            due = time.monotonic() + 6
            while True:
                try:
                    ready = request(public, b'GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
                    if ready.startswith(b'HTTP/1.1 200'):
                        break
                except (OSError, TimeoutError):
                    pass
                assert gateway.poll() is None and time.monotonic() < due
                time.sleep(0.01)
            response = request(public, b'PUT /fail HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nAuthorization: DOCUMENT_SECRET\r\nCookie: PRIVATE_COOKIE\r\nConnection: close\r\n\r\nhello')
            assert response.startswith(b'HTTP/1.1 502')
            (OUT / 'gateway-response.http').write_bytes(response)
            header = response.split(b'\r\n\r\n', 1)[0]
            request_id = next(line.split(b':', 1)[1].strip().decode('ascii') for line in header.split(b'\r\n')
                              if line.lower().startswith(b'x-request-id:'))
            assert len(request_id) == 16 and all(character in '0123456789abcdef' for character in request_id)
            status = gateway.wait(timeout=8)
            assert status != 0
            event = captured.result(timeout=4)
            assert event['tags']['request_id'] == request_id
            reaped = {}
            for name in ['java.pid', 'orphan.pid']:
                pid = int((directory / name).read_text())
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    reaped[name] = True
                else:
                    raise AssertionError('live/zombie child remains: ' + name)
            (OUT / 'result.json').write_text(json.dumps({'passed': True, 'gateway_exit': status,
                'http_status': 502, 'returned_request_id': request_id,
                'envelope_request_id': event['tags']['request_id'], 'reaped': reaped,
                'otlp_compiled': False, 'sentry_compiled': True,
                'envelope_sha256': hashlib.sha256((OUT / 'sentry.envelope').read_bytes()).hexdigest()}, indent=2) + '\n')
        finally:
            if gateway.poll() is None:
                gateway.kill()
                gateway.wait()
print('Bounded real Sentry-only fault envelope capture passed')
