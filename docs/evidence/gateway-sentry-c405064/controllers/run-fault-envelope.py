import hashlib
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent
command = ['docker', 'create', '--name', 'indra-sentry-c405064-safe-envelope', '--network', 'none',
           '--cpuset-cpus', '12,13', '--cpus', '2', '--memory', '3g', '--memory-swap', '3g',
           '--pids-limit', '128', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true',
           '--mount', 'type=bind,src=' + str(root) + ',dst=/evidence', '--entrypoint', 'python3',
           'sha256:56e865745a613045b95c3f9e478765dd2515baba4f611038b58179da1ce64239',
           '/evidence/capture-fault-envelope.py']
receipt = {'command': command, 'controller_sha256': hashlib.sha256((root / 'capture-fault-envelope.py').read_bytes()).hexdigest()}
(root / 'fault-envelope-container-command.json').write_text(json.dumps(receipt, indent=2) + '\n')
created = subprocess.run(command, capture_output=True, text=True)
(root / 'fault-envelope-create.stdout').write_text(created.stdout)
(root / 'fault-envelope-create.stderr').write_text(created.stderr)
assert created.returncode == 0
identifier = created.stdout.strip()
with (root / 'fault-envelope-inspect-before.json').open('wb') as stream:
    subprocess.run(['docker', 'inspect', identifier], stdout=stream, check=True)
with (root / 'fault-envelope-container.stdout').open('wb') as out, (root / 'fault-envelope-container.stderr').open('wb') as err:
    result = subprocess.run(['docker', 'start', '-a', identifier], stdout=out, stderr=err)
with (root / 'fault-envelope-inspect-after.json').open('wb') as stream:
    subprocess.run(['docker', 'inspect', identifier], stdout=stream, check=True)
receipt.update(container_id=identifier, status=result.returncode)
(root / 'fault-envelope-container-command.json').write_text(json.dumps(receipt, indent=2) + '\n')
assert result.returncode == 0, (root / 'fault-envelope-container.stderr').read_text()
print((root / 'fault-envelope-container.stdout').read_text())
