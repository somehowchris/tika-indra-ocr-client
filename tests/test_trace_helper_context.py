#!/usr/bin/env python3
"""Synthetic transport/schema controls; actual BuildKit/ELF proof is separate."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
IMAGE = "sha256:" + "a" * 64
CID = "c" * 64
MOCK_DOCKER = r'''#!/usr/bin/env python3
import hashlib,json,os,sys
from pathlib import Path
args=sys.argv[1:]; mode=os.environ["TRACE_MOCK_MODE"]; state=Path(os.environ["TRACE_MOCK_STATE"])
cid="c"*64; image="sha256:"+"a"*64
if args[0]=="create":
 label=args[args.index("--label")+1].split("=",1)[1]
 state.write_text(json.dumps({"label":label,"image":args[-1]}))
 Path(args[args.index("--cidfile")+1]).write_text(cid+"\n")
 print(cid)
elif args[0]=="inspect":
 saved=json.loads(state.read_text())
 label="foreign" if mode=="wrong-owner" else saved["label"]
 print(json.dumps([{"Id":cid,"Image":image,"State":{"Running":False},
 "Config":{"Labels":{"org.tika-indra.trace-helper":label}},
 "HostConfig":{"NetworkMode":"none","ReadonlyRootfs":True}}]))
elif args[0]=="cp":
 target=Path(args[2])
 if args[1].endswith(":/indra-trace-acceptance"):
  if mode=="symlink":
   payload=target.parent/"payload";payload.write_bytes(b"mock ELF");target.symlink_to(payload)
  else:target.write_bytes(b"mock ELF")
  target.chmod(0o755)
 else:
  digest=hashlib.sha256(b"mock ELF").hexdigest()
  if mode=="wrong-hash":digest="0"*64
  row=digest+"  /out/indra-trace-acceptance\n"
  if mode=="duplicate-hash":row+=row
  target.write_text(row)
elif args[0]=="rm":
 if mode=="cleanup-failure":sys.exit(1)
 state.with_suffix(".removed").write_text(cid);print(cid)
else:sys.exit(2)
'''
MOCK_READELF = r'''#!/usr/bin/env python3
import os,sys
if "-h" in sys.argv:print("Class:                             ELF64\nMachine:                           Advanced Micro Devices X86-64")
elif "-l" in sys.argv:print("program headers")
elif "-d" in sys.argv:print("(NEEDED) libc.so" if os.environ["TRACE_MOCK_MODE"]=="dynamic" else "There is no dynamic section")
else:sys.exit(1)
'''
MOCK_BIND = '''#!/usr/bin/env bash
set -euo pipefail
[[ $# = 4 && $1 = bind ]]
mkdir "$4"
root=$(jq -er .root_digest "$2/result.json")
if [[ $TRACE_MOCK_MODE = wrong-binding ]]; then root=sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb; fi
jq -n --arg root "$root" --arg id cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc \\
 '{passed:true,root_digest:$root,container_id:$id,platform_digest:"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",config_digest:"sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"}' > "$4/result.json"
'''

class HelperControls(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="trace-helper-control-")
        self.base = Path(self.temporary.name)
        scripts = self.base / "project/scripts"
        scripts.mkdir(parents=True)
        shutil.copyfile(ROOT / "scripts/prepare-trace-helper.sh", scripts / "prepare-trace-helper.sh")
        (scripts / "verify-daemon-image.sh").write_text(MOCK_BIND)
        self.script = scripts / "prepare-trace-helper.sh"
        self.mocks = self.base / "mocks"
        self.mocks.mkdir()
        for name, body in [("docker", MOCK_DOCKER), ("readelf", MOCK_READELF)]:
            p = self.mocks / name
            p.write_text(body)
            p.chmod(0o755)
        self.resolution = self.base / "resolution"
        self.resolution.mkdir()
        self.identity = dict(passed=True, store="containerd", os="linux", architecture="amd64",
                             image_id=IMAGE, root_digest=IMAGE)
        (self.resolution / "result.json").write_text(json.dumps(self.identity))
        self.output = self.base / "output"
        self.state = self.base / "mock-state"

    def tearDown(self):
        self.temporary.cleanup()

    def invoke(self, mode, expected):
        env = dict(os.environ, PATH=str(self.mocks) + os.pathsep + os.environ["PATH"],
                   TRACE_MOCK_MODE=mode, TRACE_MOCK_STATE=str(self.state))
        p = subprocess.run(["bash", str(self.script), str(self.resolution), str(self.output)],
                           env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
        self.assertEqual(p.returncode, expected, p.stderr.decode())
        return p

    def test_positive_exact_context_and_cleanup(self):
        self.invoke("positive", 0)
        self.assertEqual([p.name for p in (self.output / "context").iterdir()], ["indra-trace-acceptance"])
        result = json.loads((self.output / "result.json").read_text())
        self.assertEqual(result["helper_sha256"], hashlib.sha256(b"mock ELF").hexdigest())
        self.assertTrue(self.state.with_suffix(".removed").is_file())
        self.assertEqual((self.output / "exit-status.txt").read_text(), "0\n")

    def test_rejects_wrong_hash(self):
        self.invoke("wrong-hash", 1)

    def test_rejects_duplicate_build_hash(self):
        self.invoke("duplicate-hash", 1)

    def test_rejects_symlink_helper(self):
        self.invoke("symlink", 1)

    def test_rejects_dynamic_helper(self):
        self.invoke("dynamic", 1)

    def test_rejects_foreign_owner_without_removal(self):
        self.invoke("wrong-owner", 1)
        self.assertFalse(self.state.with_suffix(".removed").exists())

    def test_rejects_wrong_bound_root(self):
        self.invoke("wrong-binding", 1)

    def test_cleanup_failure_rejects_even_after_copy(self):
        self.invoke("cleanup-failure", 1)
        self.assertEqual((self.output / "exit-status.txt").read_text(), "1\n")

    def test_existing_output_preserved(self):
        self.output.mkdir()
        marker = self.output / "marker"
        marker.write_bytes(b"existing")
        self.invoke("positive", 1)
        self.assertEqual(marker.read_bytes(), b"existing")
        self.assertFalse(self.state.exists())

    def test_invalid_root_never_creates_container(self):
        self.identity["image_id"] = "sha256:" + "f" * 64
        (self.resolution / "result.json").write_text(json.dumps(self.identity))
        self.invoke("positive", 1)
        self.assertFalse(self.state.exists())


    def test_release_mounts_fetched_revision_leaf_and_old_parent_fails(self):
        # Execute the actual release shell step against an inert revision tree.
        # The stub models the unchanged gate's direct /models mount; no Docker,
        # model parsing or OCR runs in this filesystem caller regression.
        import re
        import textwrap
        lock = json.loads((ROOT / "backend/models.lock.json").read_text())
        configuration = (ROOT / "runtime/crates/indra-server/src/configuration.rs").read_text()
        default = re.search(r'DEFAULT_MODEL_DIRECTORY: &str = "([^"]+)"', configuration).group(1)
        self.assertEqual(default, "/models/" + lock["revision"])
        project = self.base / "caller"
        scripts = project / "scripts"
        scripts.mkdir(parents=True)
        (scripts / "prepare-trace-helper.sh").write_text('mkdir -p "$2/context"\n')
        (scripts / "verify-trace-native.sh").write_text(
            'set -euo pipefail\n'
            '[[ -f "$2/config.json" && -f "$2/det.onnx" ]]\n'
            'printf "%s\\n" "$2" > "$TRACE_MODELS_CAPTURE"\n')
        leaf = project / "release-evidence/models" / lock["revision"]
        leaf.mkdir(parents=True)
        for record in lock["files"]:
            path = leaf / record["name"]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"inert synthetic cache-path fixture")
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        marker = "      - name: Verify actual gateway, forked Java, bridge and native OTLP graph before signing\n"
        self.assertEqual(workflow.count(marker), 1)
        section = workflow.split(marker, 1)[1].split("\n      - ", 1)[0]
        command = textwrap.dedent(section.split("        run: |\n", 1)[1])
        fake_bin = self.base / "caller-mocks"
        fake_bin.mkdir()
        (fake_bin / "docker").write_text("#!/bin/sh\nexit 0\n")
        (fake_bin / "docker").chmod(0o755)
        capture = self.base / "models-capture"
        env = dict(os.environ, PATH=str(fake_bin) + os.pathsep + os.environ["PATH"],
                   TIKA_REF="fixture:tika", NATIVE_REF="fixture:native",
                   TRACE_MODELS_CAPTURE=str(capture))
        valid = subprocess.run(["bash", "-e", "-c", command], cwd=project, env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        self.assertEqual(valid.returncode, 0, valid.stderr.decode())
        self.assertEqual(Path(capture.read_text().strip()), leaf)
        capture.unlink()
        needle = "/models/" + lock["revision"]
        self.assertEqual(command.count(needle), 1)
        old_parent = command.replace(needle, "/models")
        rejected = subprocess.run(["bash", "-e", "-c", old_parent], cwd=project, env=env,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertFalse(capture.exists())


if __name__ == "__main__":
    unittest.main()
