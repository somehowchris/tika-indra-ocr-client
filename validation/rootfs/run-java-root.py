"""Run the assembled Tika final root as UID65532, inside private mounts."""
import json
import os
from pathlib import Path
import subprocess
import sys

base = Path('/workspace/scratch/b641098e5d96/docker-validation')
arguments = sys.argv[1:]
label = 'java-root'
if arguments[:1] == ['--root']:
    label = arguments[1]
    arguments = arguments[2:]
if label not in ('java-root', 'tesseract-root'):
    raise ValueError('unexpected validation root')
root = base / label
subprocess.run(['mount', '--bind', str(root), str(root)], check=True)
subprocess.run(['mount', '-o', 'remount,bind,ro', str(root)], check=True)
for name in ('proc', 'dev', 'sys'):
    subprocess.run(['mount', '--rbind', '/' + name, str(root / name)], check=True)
subprocess.run(['mount', '-t', 'tmpfs', '-o', 'size=256m,mode=1777,nosuid,noexec',
                'tmpfs', str(root / 'tmp')], check=True)
environment = dict(item.split('=', 1) for item in json.loads((base / 'java-image.json').read_text())[0]['Config']['Env'])
environment.update(INDRA_ADDR='127.0.0.1:18081', INDRA_TIMEOUT_SECONDS='70',
                   INDRA_CONNECT_TIMEOUT_MS='1000', XDG_CACHE_HOME='/tmp/.cache')
if label == 'tesseract-root':
    environment.update(OMP_THREAD_LIMIT='1', OMP_NUM_THREADS='1', TESSDATA_PREFIX='/opt/tessdata-best')
os.chroot(root)
os.chdir('/tmp')
os.setgid(65532)
os.setuid(65532)
command = arguments
os.execve(command[0], command, environment)
