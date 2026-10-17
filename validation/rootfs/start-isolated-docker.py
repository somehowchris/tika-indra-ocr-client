"""Start only the scratch validation daemon; no host Docker state is changed."""
import json
import os
from pathlib import Path
import sys

base = Path('/workspace/scratch/b641098e5d96/docker-validation')
base.mkdir(exist_ok=True)
runtime = base / 'rootless-state'
runtime.mkdir(exist_ok=True)
runtime.chmod(0o700)
bindir = Path('/workspace/scratch/b641098e5d96/docker-validation-tools/docker')
(base / 'daemon.json').write_text(json.dumps({
    'features': {'containerd-snapshotter': False},
    'runtimes': {'validation-runc': {'path': str(bindir / 'runc'), 'runtimeArgs': ['--rootless=true']}},
    'default-runtime': 'validation-runc',
}))
for file in bindir.iterdir():
    if file.is_file():
        file.chmod(0o755)
os.environ.update(PATH=str(bindir) + ':' + os.environ['PATH'], GOMAXPROCS='2',
                  XDG_RUNTIME_DIR=str(runtime))
os.environ.pop('ROOTLESSKIT_STATE_DIR', None)
log = os.open(base / 'daemon.log', os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
os.dup2(log, 1)
os.dup2(log, 2)
os.close(log)
args = [sys.executable, '/workspace/scratch/b641098e5d96/namespace-docker-validation.py',
        str(bindir / 'dockerd'), '--data-root=' + str(base / 'data'),
        '--exec-root=' + str(base / 'exec'), '--pidfile=' + str(base / 'daemon.pid'),
        '--host=unix://' + str(base / 'docker.sock'), '--storage-driver=vfs',
        '--iptables=false', '--ip6tables=false', '--bridge=none', '--ip-forward=false',
        '--ip-masq=false', '--userland-proxy=false', '--exec-opt=native.cgroupdriver=cgroupfs',
        '--config-file=' + str(base / 'daemon.json')]
os.execv(sys.executable, args)
