#!/usr/bin/env python3
"""Smoke existing images in kind and verify both admitted and kernel limits.

Run after the Compose stacks stop. Application limits cover the two OCR
containers; the runner, Kubernetes control plane and test client are outside
that budget. This is a correctness/resource-configuration gate, not a benchmark.
Only native host executables and Debian/distroless kind images are introduced.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shlex
import subprocess
import sys
import tarfile
import tempfile
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
# Release pins verified against these primary sources on 2026-10-17:
# https://github.com/kubernetes-sigs/kind/releases/tag/v0.33.0
# https://github.com/kubernetes-sigs/kind/releases/download/v0.33.0/kind-linux-amd64.sha256sum
# https://dl.k8s.io/release/v1.36.4/bin/linux/amd64/kubectl.sha256
KIND_VERSION = "v0.33.0"
KUBERNETES_VERSION = "v1.36.4"
NODE_IMAGE = "kindest/node:v1.36.4@sha256:099e049362a1526b2db71494e1947aae99bd16290d7c895f2b7ea312e3cbfaed"
TOOLS = {
    "kind": (
        "https://github.com/kubernetes-sigs/kind/releases/download/v0.33.0/kind-linux-amd64",
        "aee6151561422756b764a4ae28e7f44cda5af5a9eead3cc9985112b1de8d8e0d",
    ),
    "kubectl": (
        "https://dl.k8s.io/release/v1.36.4/bin/linux/amd64/kubectl",
        "8b8f088da2dab964f853b38464033b1be15ede2839eca751482357c45abdd05a",
    ),
}
SOURCE_ROOT = "https://github.com/kubernetes-sigs/kind/blob/407a9675e6d9af1200b5f57f9ca52ec6cdacce74/"
DOCKER_SOURCE_ROOT = "https://github.com/moby/moby/blob/b26fb77d3e7b01f9d21b89539c1ab428f5ca431f/"
BASE_SOURCES = [
    SOURCE_ROOT + "images/base/Dockerfile",
    SOURCE_ROOT + "images/local-path-provisioner/README.md",
    SOURCE_ROOT + "images/local-path-provisioner/Dockerfile",
    SOURCE_ROOT + "images/local-path-helper/Dockerfile",
    SOURCE_ROOT + "images/kindnetd/Dockerfile",
    "https://github.com/kubernetes/kubernetes/blob/v1.36.4/build/pause/Makefile",
    DOCKER_SOURCE_ROOT + "daemon/containerd/image_inspect.go",
    DOCKER_SOURCE_ROOT + "daemon/containerd/image_exporter.go",
    DOCKER_SOURCE_ROOT + "vendor/github.com/containerd/containerd/v2/core/images/archive/exporter.go",
    DOCKER_SOURCE_ROOT + "vendor/github.com/containerd/containerd/v2/core/images/annotations.go",
    SOURCE_ROOT + "pkg/cluster/nodeutils/util.go",
]
IMAGES = {"tika": "tika-indra:dev", "indra": "indra-backend:dev"}
MEMORY = {"tika": 1024**3, "indra": 2 * 1024**3}
IMAGE_MANIFEST_TYPES = {
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
}
REFERRER_SUBJECT_ANNOTATION = "io.containerd.manifest.subject"


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def archive_identity(path: Path, descriptor: dict) -> dict:
    """Bind the saved runnable manifest to its exact CRI configuration digest.

    Docker's containerd-backed inspect Id names a manifest/index; CRI status.id
    names its configuration. A platform-specific inspect and save select the
    same original manifest, without dropping provenance from the registry image.
    Only small metadata blobs are read; no tar members are extracted.
    """
    manifest_digest = descriptor.get("digest", "")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", manifest_digest):
        raise RuntimeError("Invalid selected Docker manifest digest")
    if descriptor.get("mediaType") not in IMAGE_MANIFEST_TYPES:
        raise RuntimeError("Platform-specific Docker inspect must select an image manifest")
    with tarfile.open(path, "r:") as archive:
        members = {}
        for member in archive.getmembers():
            if member.name in members:
                raise RuntimeError("Duplicate archive member")
            members[member.name] = member

        def read_member(name: str) -> bytes:
            member = members.get(name)
            if member is None or not member.isfile() or member.size > 4 * 1024 * 1024:
                raise RuntimeError(f"Missing, nonregular or oversized archive metadata: {name}")
            stream = archive.extractfile(member)
            if stream is None:
                raise RuntimeError(f"Unreadable archive metadata: {name}")
            with stream:
                data = stream.read()
            if len(data) != member.size:
                raise RuntimeError(f"Truncated archive metadata: {name}")
            return data

        def read_blob(digest: str, size: int) -> tuple[bytes, str]:
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
                raise RuntimeError("Invalid archive metadata digest")
            name = "blobs/sha256/" + digest.removeprefix("sha256:")
            data = read_member(name)
            if len(data) != size or "sha256:" + hashlib.sha256(data).hexdigest() != digest:
                raise RuntimeError(f"Archive metadata digest/size mismatch: {name}")
            return data, name

        index = json.loads(read_member("index.json"))
        entries = index.get("manifests", [])
        if index.get("schemaVersion") != 2 or not isinstance(entries, list) or not entries:
            raise RuntimeError("Expected an exported OCI image index")
        # Docker 29 exports attached OCI artifacts even with --platform. The
        # containerd subject annotation prevents creating runnable image names
        # for those entries; verify its binding against the hashed OCI subject.
        image_entries = [entry for entry in entries
                         if not entry.get("annotations", {}).get(REFERRER_SUBJECT_ANNOTATION)]
        if len(image_entries) != 1:
            raise RuntimeError("Expected exactly one exported platform-specific image")
        entry = image_entries[0]
        if any(entry.get(key) != descriptor.get(key) for key in ("digest", "size", "mediaType")):
            raise RuntimeError("Archive index differs from the selected Docker manifest")
        manifest_bytes, _ = read_blob(manifest_digest, descriptor["size"])
        manifest = json.loads(manifest_bytes)
        if manifest.get("schemaVersion") != 2 or "config" not in manifest or "layers" not in manifest:
            raise RuntimeError("Invalid exported image manifest")
        config_descriptor = manifest["config"]
        config_digest = config_descriptor.get("digest", "")
        config_bytes, config_path = read_blob(config_digest, config_descriptor.get("size"))
        config = json.loads(config_bytes)
        if config.get("os") != "linux" or config.get("architecture") != "amd64":
            raise RuntimeError("Exported image config must select linux/amd64")
        referrers = []
        for referrer in entries:
            if referrer is entry:
                continue
            if (referrer.get("mediaType") != "application/vnd.oci.image.manifest.v1+json"
                    or referrer.get("annotations", {}).get(REFERRER_SUBJECT_ANNOTATION) != manifest_digest):
                raise RuntimeError("Archive referrer must bind to the selected image manifest")
            referrer_bytes, _ = read_blob(referrer.get("digest", ""), referrer.get("size"))
            artifact = json.loads(referrer_bytes)
            subject = artifact.get("subject", {})
            if (artifact.get("schemaVersion") != 2 or not artifact.get("artifactType")
                    or any(subject.get(key) != descriptor.get(key) for key in ("digest", "size", "mediaType"))):
                raise RuntimeError("Archive artifact subject differs from the selected image manifest")
            if referrer.get("artifactType", artifact["artifactType"]) != artifact["artifactType"]:
                raise RuntimeError("Archive referrer artifact type disagrees with its manifest")
            artifact_config = artifact.get("config", {})
            artifact_config_bytes, _ = read_blob(artifact_config.get("digest", ""), artifact_config.get("size"))
            if (artifact_config.get("mediaType") != "application/vnd.oci.empty.v1+json"
                    or json.loads(artifact_config_bytes) != {}):
                raise RuntimeError("Archive referrer must have a non-runnable empty OCI config")
            layers = artifact.get("layers", [])
            if not isinstance(layers, list) or not layers:
                raise RuntimeError("Archive referrer must contain artifact payloads")
            for layer in layers:
                read_blob(layer.get("digest", ""), layer.get("size"))
            referrers.append({"manifest_digest": referrer["digest"],
                              "artifact_type": artifact["artifactType"],
                              "subject_digest": subject["digest"],
                              "payload_digests": [layer["digest"] for layer in layers]})
        compatibility = json.loads(read_member("manifest.json"))
        if len(compatibility) != 1 or compatibility[0].get("Config") != config_path:
            raise RuntimeError("Docker and OCI archive views disagree about the image config")
    return {"manifest_digest": manifest_digest, "config_digest": config_digest,
            "platform": "linux/amd64", "archive_bytes": path.stat().st_size,
            "referrers": referrers}


def install_tools(directory: Path) -> dict[str, str]:
    """Download into the private work directory; execute only verified bytes."""
    paths = {}
    for name, (url, expected) in TOOLS.items():
        destination = directory / name
        digest = hashlib.sha256()
        length = 0
        with urllib.request.urlopen(url, timeout=45) as response, destination.open("xb") as output:
            while chunk := response.read(1024 * 1024):
                length += len(chunk)
                if length > 128 * 1024 * 1024:
                    raise RuntimeError(f"Unexpectedly large {name} download")
                digest.update(chunk)
                output.write(chunk)
        if digest.hexdigest() != expected:
            raise RuntimeError(f"SHA-256 mismatch for {name}")
        destination.chmod(0o755)
        paths[name] = str(destination)
    return paths


def cpu_quantity(value: str) -> Fraction:
    return Fraction(value[:-1]) / 1000 if value.endswith("m") else Fraction(value)


def memory_quantity(value: str) -> int:
    for suffix, multiplier in (("Gi", 1024**3), ("Mi", 1024**2), ("Ki", 1024)):
        if value.endswith(suffix):
            return int(Fraction(value[:-2]) * multiplier)
    return int(value)


def expected_cpu(budget: int) -> dict[str, Fraction]:
    return {"tika": Fraction(budget, 4), "indra": Fraction(3 * budget, 4)}


def verify_memory_events(raw: str, container: str) -> dict[str, int]:
    # Hierarchical counters include descendant workers; oom_kill also records
    # victims of a global OOM killer. https://docs.kernel.org/admin-guide/cgroup-v2.html
    events = {}
    for line in raw.splitlines():
        parts = line.split()
        if len(parts) != 2 or parts[0] in events or not parts[1].isdecimal():
            raise RuntimeError(f"Malformed memory.events for {container}")
        events[parts[0]] = int(parts[1])
    if not {"oom", "oom_kill"} <= events.keys():
        raise RuntimeError(f"Missing OOM counters for {container}")
    failures = {key: events[key] for key in ("oom", "oom_kill", "oom_group_kill") if events.get(key, 0)}
    if failures:
        raise RuntimeError(f"OOM event in {container}: {failures}")
    return events


def verify_pod(pod: dict, budget: int) -> None:
    containers = {entry["name"]: entry for entry in pod["spec"]["containers"]}
    if set(containers) != set(IMAGES) or pod["spec"].get("initContainers"):
        raise RuntimeError("Expected exactly the two application containers")
    if pod["status"].get("qosClass") != "Guaranteed":
        raise RuntimeError("Admitted Pod is not Guaranteed QoS")
    for name, container in containers.items():
        if container["image"] != IMAGES[name]:
            raise RuntimeError(f"Unexpected admitted image for {name}")
        for category in ("requests", "limits"):
            values = container["resources"][category]
            if cpu_quantity(values["cpu"]) != expected_cpu(budget)[name]:
                raise RuntimeError(f"Incorrect admitted {category} CPU for {name}")
            if memory_quantity(values["memory"]) != MEMORY[name]:
                raise RuntimeError(f"Incorrect admitted {category} memory for {name}")
        security = container["securityContext"]
        if not security.get("readOnlyRootFilesystem") or security.get("allowPrivilegeEscalation") is not False:
            raise RuntimeError(f"Incorrect admitted security context for {name}")
    environment = {item["name"]: item.get("value") for item in containers["indra"]["env"]}
    if environment.get("INDRA_WORKERS") != str(budget) or environment.get("INDRA_THREADS") != "1":
        raise RuntimeError("Incorrect admitted Indra worker/thread profile")
    statuses = pod["status"].get("containerStatuses", [])
    if len(statuses) != 2 or any(not item.get("ready") or item.get("restartCount") != 0 for item in statuses):
        raise RuntimeError("An application container is unready or restarted")


class Check:
    def __init__(self, output: Path, work: Path, tools: dict[str, str], name: str):
        self.output = output
        self.work = work
        self.tools = tools
        self.name = name
        self.node = ""
        self.env = dict(os.environ)
        self.env["KIND_EXPERIMENTAL_PROVIDER"] = "docker"
        self.env["KUBECONFIG"] = str(work / "kubeconfig")

    def run(self, command: list[str], timeout: int = 180, check: bool = True) -> str:
        print("+ " + shlex.join(command), flush=True)
        result = subprocess.run(command, cwd=ROOT, env=self.env, text=True, capture_output=True, timeout=timeout)
        with (self.output / "commands.log").open("a", encoding="utf-8") as log:
            log.write("+ " + shlex.join(command) + "\n" + result.stdout + result.stderr + "\n")
        if check and result.returncode:
            raise RuntimeError(f"Command exited {result.returncode}: {shlex.join(command)}\n{result.stderr[-4000:]}")
        return result.stdout

    def kubectl(self, *arguments: str, timeout: int = 180, check: bool = True) -> str:
        return self.run([self.tools["kubectl"], "--context", "kind-" + self.name, *arguments], timeout, check)

    def node_command(self, *arguments: str) -> str:
        return self.run(["docker", "exec", self.node, *arguments])

    def pod(self) -> dict:
        objects = json.loads(self.kubectl("get", "pods", "-l", "app=tika-indra", "-o", "json"))["items"]
        active = [pod for pod in objects if not pod["metadata"].get("deletionTimestamp")]
        if len(active) != 1:
            raise RuntimeError(f"Expected one active application Pod; found {len(active)}")
        return active[0]

    def cgroups(self, pod: dict, budget: int) -> dict:
        records = {}
        for container in pod["status"]["containerStatuses"]:
            name = container["name"]
            identity = container["containerID"].removeprefix("containerd://")
            if not re.fullmatch(r"[0-9a-f]{64}", identity):
                raise RuntimeError("Unexpected containerd container ID")
            runtime = json.loads(self.node_command("crictl", "inspect", identity))
            pid = int(runtime["info"]["pid"])
            if pid <= 0:
                raise RuntimeError("Invalid runtime PID")
            membership = self.node_command("cat", f"/proc/{pid}/cgroup")
            unified = [line[3:] for line in membership.splitlines() if line.startswith("0::")]
            if len(unified) != 1 or ".." in PurePosixPath(unified[0]).parts or not unified[0].startswith("/"):
                raise RuntimeError("Require a normal cgroup-v2 application path")
            path = "/sys/fs/cgroup" + unified[0]
            # This shell belongs to the Debian kind node, not an application image.
            raw = self.node_command("/bin/sh", "-ec", (
                'for name in cpu.max cpu.stat memory.max memory.peak memory.events pids.current; do '
                'printf "## %s\\n" "$name"; '
                'if [ -r "$1/$name" ]; then cat "$1/$name"; else printf "unavailable\\n"; fi; done; '
                'printf "## process_status\\n"; cat "/proc/$2/status"; '
                'printf "## mountinfo\\n"; cat "/proc/$2/mountinfo"'
            ), "read-application-cgroup", path, str(pid))
            fields = {}
            for block in raw.split("## ")[1:]:
                key, _, value = block.partition("\n")
                fields[key] = value.strip()
            quota, period = fields["cpu.max"].split()
            if quota == "max" or Fraction(int(quota), int(period)) != expected_cpu(budget)[name]:
                raise RuntimeError(f"Kernel CPU ceiling differs from the {budget}-CPU profile for {name}")
            if fields["memory.max"] != str(MEMORY[name]):
                raise RuntimeError(f"Kernel memory ceiling differs for {name}")
            # Each profile creates a fresh Pod. Requiring zero both before and
            # after smoke includes startup OOMs and recovered worker-only kills.
            events = verify_memory_events(fields["memory.events"], name)
            status = dict(line.split(":", 1) for line in fields["process_status"].splitlines() if ":" in line)
            if set(status["Uid"].split()) != {"65532"} or set(status["Gid"].split()) != {"65532"}:
                raise RuntimeError(f"Runtime identity differs for {name}")
            mounts = [line.split() for line in fields["mountinfo"].splitlines()]
            roots = [mount for mount in mounts if mount[4] == "/"]
            if len(roots) != 1 or "ro" not in roots[0][5].split(","):
                raise RuntimeError(f"Root filesystem is not read-only for {name}")
            temporary = [mount for mount in mounts if mount[4] == "/tmp"]
            if len(temporary) != 1 or temporary[0][temporary[0].index("-") + 1] != "tmpfs":
                raise RuntimeError(f"Expected a memory-backed /tmp for {name}")
            records[name] = {"container_id": identity, "pid": pid, "path": path, "memory_events": events, **fields}
        return records

    def profile(self, budget: int) -> dict:
        overlay = f"deploy/overlays/{budget}cpu"
        self.kubectl("apply", "-k", overlay)
        self.kubectl("rollout", "status", "deployment/tika-indra", "--timeout=300s", timeout=330)
        before = self.pod()
        verify_pod(before, budget)
        record = {"budget_cpus": budget, "pod_before": before, "cgroups_before": self.cgroups(before, budget)}
        log_path = self.output / f"port-forward-{budget}cpu.log"
        with log_path.open("w", encoding="utf-8") as log:
            forward = subprocess.Popen([
                self.tools["kubectl"], "--context", "kind-" + self.name,
                "port-forward", "--address", "127.0.0.1", "service/tika-indra", "19998:9998",
            ], cwd=ROOT, env=self.env, stdout=log, stderr=subprocess.STDOUT)
            try:
                result = self.run([
                    sys.executable, "scripts/smoke_stack.py", "--url", "http://127.0.0.1:19998", "--wait", "180",
                ], timeout=750)
                record["smoke"] = json.loads(result)
                if forward.poll() is not None:
                    raise RuntimeError("Kubernetes port-forward exited during the smoke check")
            finally:
                forward.terminate()
                try:
                    forward.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    forward.kill()
                    forward.wait(timeout=10)
        after = self.pod()
        verify_pod(after, budget)
        if before["metadata"]["uid"] != after["metadata"]["uid"]:
            raise RuntimeError("Application Pod changed during the check")
        record["pod_after"] = after
        record["cgroups_after"] = self.cgroups(after, budget)
        record["status"] = "passed"
        write_json(self.output / f"{budget}cpu.json", record)
        logs = self.kubectl("logs", "deployment/tika-indra", "--all-containers=true", "--prefix=true")
        (self.output / f"{budget}cpu-application.log").write_text(logs, encoding="utf-8")
        self.kubectl("delete", "-k", overlay, "--wait=true", "--timeout=180s", timeout=210)
        return {"budget_cpus": budget, "status": "passed", "evidence": f"{budget}cpu.json"}

    def diagnostics(self) -> None:
        for name, arguments in (
            ("pods.json", ("get", "pods", "-A", "-o", "json")),
            ("events.json", ("get", "events", "-A", "-o", "json")),
            ("application-final.log", ("logs", "deployment/tika-indra", "--all-containers=true", "--prefix=true")),
        ):
            try:
                value = self.kubectl(*arguments, timeout=30, check=False)
                (self.output / name).write_text(value, encoding="utf-8")
            except (OSError, subprocess.TimeoutExpired) as exc:
                print(f"Diagnostic {name}: {exc}", file=sys.stderr)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("This validation is pinned to native Linux/amd64")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / "summary.json").exists():
        raise RuntimeError("Use a fresh output directory; existing evidence is not overwritten")
    report = {
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "kind_version": KIND_VERSION,
        "kubernetes_version": KUBERNETES_VERSION,
        "node_image": NODE_IMAGE,
        "tools": {name: {"url": values[0], "sha256": values[1]} for name, values in TOOLS.items()},
        "base_source_references": BASE_SOURCES,
        "scope": "Application correctness and admitted/kernel limits; no performance claim; cluster overhead excluded",
        "profiles": [],
        "status": "failed",
    }
    try:
        with tempfile.TemporaryDirectory(prefix="tika-kind-") as temporary:
            work = Path(temporary)
            tools = install_tools(work)
            name = f"tika-indra-check-{os.getpid()}"
            check = Check(output, work, tools, name)
            report["kind_actual"] = check.run([tools["kind"], "version"]).strip()
            report["kubectl_actual"] = json.loads(check.run([tools["kubectl"], "version", "--client", "-o", "json"]))
            images = json.loads(check.run(["docker", "image", "inspect", *IMAGES.values()]))
            if any(image["Os"] != "linux" or image["Architecture"] != "amd64" for image in images):
                raise RuntimeError("Application images must be native Linux/amd64")
            report["docker_images"] = images
            selected = json.loads(check.run([
                "docker", "image", "inspect", "--platform", "linux/amd64", *IMAGES.values(),
            ]))
            if len(selected) != len(IMAGES):
                raise RuntimeError("Unexpected platform-specific Docker inspection result")
            report["docker_selected_images"] = selected
            archives = {}
            identities = {}
            for image_name, image in zip(IMAGES, selected):
                descriptor = image.get("Descriptor", {})
                if descriptor.get("digest") != image["Id"]:
                    raise RuntimeError("Require Docker's containerd image-store descriptor")
                archive = work / (image_name + ".tar")
                # An unfiltered docker save can reference absent provenance or
                # foreign-platform children, which kind imports with --all-platforms.
                check.run(["docker", "image", "save", "--platform", "linux/amd64",
                           "--output", str(archive), IMAGES[image_name]], timeout=300)
                identities[image_name] = archive_identity(archive, descriptor)
                archives[image_name] = archive
            report["archive_identities"] = identities
            if name in check.run([tools["kind"], "get", "clusters"]).splitlines():
                raise RuntimeError("Refusing to replace an existing kind cluster")
            cluster = {
                "kind": "Cluster", "apiVersion": "kind.x-k8s.io/v1alpha4",
                "networking": {"apiServerAddress": "127.0.0.1"},
                "nodes": [{"role": "control-plane"}],
            }
            config_path = output / "kind-config.json"
            write_json(config_path, cluster)
            try:
                check.run([tools["kind"], "create", "cluster", "--name", name, "--image", NODE_IMAGE,
                           "--config", str(config_path), "--kubeconfig", check.env["KUBECONFIG"], "--wait", "180s"], timeout=420)
                nodes = check.run([tools["kind"], "get", "nodes", "--name", name]).splitlines()
                if len(nodes) != 1:
                    raise RuntimeError("Expected one kind node")
                check.node = nodes[0]
                os_release = check.node_command("cat", "/etc/os-release")
                if not re.search(r'^ID=[\"]?debian[\"]?$', os_release, flags=re.MULTILINE):
                    raise RuntimeError("The pinned kind node is not Debian")
                report["node_os_release"] = os_release
                report["cluster_version"] = json.loads(check.kubectl("version", "-o", "json"))
                if report["cluster_version"]["serverVersion"]["gitVersion"] != KUBERNETES_VERSION:
                    raise RuntimeError("Unexpected Kubernetes server version")
                report["nodes"] = json.loads(check.kubectl("get", "nodes", "-o", "json"))
                for archive in archives.values():
                    check.run([tools["kind"], "load", "image-archive", "--name", name, str(archive)], timeout=300)
                imported = {}
                for image_name in IMAGES:
                    image = json.loads(check.node_command("crictl", "inspecti", IMAGES[image_name]))
                    if image["status"]["id"] != identities[image_name]["config_digest"]:
                        raise RuntimeError(f"Imported {image_name} config differs from the exact saved Docker image")
                    imported[image_name] = image["status"]
                report["imported_images"] = imported
                for budget in (1, 2):
                    report["profiles"].append(check.profile(budget))
                report["status"] = "passed"
            finally:
                check.diagnostics()
                # The unique cluster name was checked absent before this attempt.
                check.run([tools["kind"], "delete", "cluster", "--name", check.name], timeout=180)
                report["cluster_deleted"] = True
    except Exception as exc:
        report["status"] = "failed"
        report["error"] = str(exc)
        raise
    finally:
        report["finished_at_utc"] = datetime.now(timezone.utc).isoformat()
        write_json(output / "summary.json", report)


if __name__ == "__main__":
    main()
