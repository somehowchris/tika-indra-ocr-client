"""Reproduce Tesseract native stage contents using the pinned Debian snapshot."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

base = Path('/workspace/scratch/b641098e5d96')
builder = base / 'docker-validation/debian-root'
repository = base / 'tika-indra-ocr-client'
for name in ('proc', 'dev', 'sys'):
    subprocess.run(['mount', '--rbind', '/' + name, str(builder / name)], check=True)
environment = dict(os.environ)
environment.update(DEBIAN_FRONTEND='noninteractive', PATH='/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin')
environment.pop('LD_LIBRARY_PATH', None)
def run(arguments):
    print(json.dumps({'builder_command': arguments}), flush=True)
    subprocess.run(['chroot', str(builder), *arguments], check=True, env=environment)

run(['apt-get', 'update'])
run(['apt-get', 'install', '--yes', '--no-install-recommends', 'ca-certificates', 'curl', 'tesseract-ocr'])
target = builder / 'usr/local/bin/copy-elf-dependencies'
target.parent.mkdir(parents=True, exist_ok=True)
shutil.copyfile(repository / 'docker/copy-elf-dependencies.sh', target)
target.chmod(0o555)
for name in ('usr/bin', 'opt/tessdata-best/configs'):
    (builder / 'out' / name).mkdir(parents=True, exist_ok=True)
shutil.copy2(builder / 'usr/bin/tesseract', builder / 'out/usr/bin/tesseract')
run(['/usr/local/bin/copy-elf-dependencies', '/usr/bin/tesseract', '/out'])
shutil.copy2(builder / 'usr/share/tesseract-ocr/5/tessdata/configs/txt', builder / 'out/opt/tessdata-best/configs/txt')
model = base / 'tessdata-best-research/eng.traineddata'
expected = '8280aed0782fe27257a68ea10fe7ef324ca0f8d85bd2fd145d1c2b560bcb66ba'
if hashlib.sha256(model.read_bytes()).hexdigest() != expected:
    raise RuntimeError('Tesseract best English model checksum mismatch')
shutil.copyfile(model, builder / 'out/opt/tessdata-best/eng.traineddata')
with (builder / 'out/opt/tessdata-best/engine-version.txt').open('wb') as stream:
    subprocess.run(['chroot', str(builder), '/usr/bin/tesseract', '--version'], check=True, env=environment, stdout=stream, stderr=subprocess.STDOUT)
print(json.dumps({'native_stage_reproduced': str(builder / 'out'),
                  'tessdata_best_sha256': expected, 'affinity': sorted(os.sched_getaffinity(0)),
                  'scope': 'rootfs native-stage reproduction, not Docker build/run'}), flush=True)
