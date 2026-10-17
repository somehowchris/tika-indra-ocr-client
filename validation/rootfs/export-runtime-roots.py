"""Export pinned image roots for ABI validation, without starting containers."""
import json
from pathlib import Path
import subprocess

base = Path('/workspace/scratch/b641098e5d96/docker-validation')
docker = ['/workspace/scratch/b641098e5d96/docker-validation-tools/docker/docker',
          '--host', 'unix:///workspace/scratch/b641098e5d96/docker-validation/docker.sock']
images = {
    'java': 'gcr.io/distroless/java21-debian13:nonroot@sha256:51c242e7ddaba3666d47115a76a19cfa5076b2ad3ed6077b30d7bac689d98fa1',
    'python': 'gcr.io/distroless/python3-debian13:nonroot@sha256:83aa8d4f74a4d7f7cf2d472054139bef71a927b76c680c0f2e1021d6b1d6d732',
    'debian': 'debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f',
}
for label, image in images.items():
    name = 'validation-export-' + label
    root = base / (label + '-root')
    root.mkdir(exist_ok=True)
    metadata = subprocess.check_output(docker + ['image', 'inspect', image])
    (base / (label + '-image.json')).write_bytes(metadata)
    subprocess.run(docker + ['create', '--name', name, '--network', 'none',
                            '--entrypoint', '/bin/true', image], check=True)
    archive = base / (label + '-root.tar')
    subprocess.run(docker + ['export', '--output', str(archive), name], check=True)
    subprocess.run(['tar', '--extract', '--file', str(archive), '--directory', str(root),
                    '--no-same-owner'], check=True)
    subprocess.run(docker + ['rm', name], check=True)
    print(json.dumps({'root': str(root), 'source_image': image, 'archive_bytes': archive.stat().st_size}), flush=True)
