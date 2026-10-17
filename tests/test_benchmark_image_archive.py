"""Catch OCI manifest/config confusion in the Kubernetes image-import gate."""

from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from verify_kubernetes import archive_identity


def encoded(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode()


def descriptor(content: bytes, media_type: str) -> dict:
    return {"mediaType": media_type, "digest": "sha256:" + hashlib.sha256(content).hexdigest(),
            "size": len(content)}


class ArchiveIdentityTests(unittest.TestCase):
    def setUp(self) -> None:
        config = encoded({"os": "linux", "architecture": "amd64", "rootfs": {"type": "layers", "diff_ids": []}})
        config_descriptor = descriptor(config, "application/vnd.oci.image.config.v1+json")
        manifest = encoded({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json",
                            "config": config_descriptor, "layers": []})
        self.manifest_descriptor = descriptor(manifest, "application/vnd.oci.image.manifest.v1+json")
        self.config_digest = config_descriptor["digest"]
        self.config_path = "blobs/sha256/" + self.config_digest.removeprefix("sha256:")
        self.manifest_path = "blobs/sha256/" + self.manifest_descriptor["digest"].removeprefix("sha256:")
        self.members = {
            "oci-layout": encoded({"imageLayoutVersion": "1.0.0"}),
            "index.json": encoded({"schemaVersion": 2, "manifests": [self.manifest_descriptor]}),
            "manifest.json": encoded([{"Config": self.config_path, "RepoTags": ["identity:dev"], "Layers": []}]),
            self.config_path: config,
            self.manifest_path: manifest,
        }

    def check_archive(self, expected: dict | None = None, duplicate: str | None = None) -> dict:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "image.tar"
            with tarfile.open(path, "w") as archive:
                entries = list(self.members.items())
                if duplicate is not None:
                    entries.append((duplicate, self.members[duplicate]))
                for name, data in entries:
                    member = tarfile.TarInfo(name)
                    member.size = len(data)
                    archive.addfile(member, io.BytesIO(data))
            return archive_identity(path, expected or self.manifest_descriptor)

    def test_returns_config_digest_without_confusing_it_with_manifest(self) -> None:
        result = self.check_archive()
        self.assertEqual(result["config_digest"], self.config_digest)
        self.assertEqual(result["manifest_digest"], self.manifest_descriptor["digest"])
        self.assertNotEqual(result["config_digest"], result["manifest_digest"])

    def test_rejects_different_selected_image(self) -> None:
        expected = dict(self.manifest_descriptor, digest="sha256:" + "0" * 64)
        with self.assertRaisesRegex(RuntimeError, "differs from the selected"):
            self.check_archive(expected)

    def test_rejects_config_corruption(self) -> None:
        self.members[self.config_path] += b" "
        with self.assertRaisesRegex(RuntimeError, "digest/size mismatch"):
            self.check_archive()

    def test_rejects_manifest_corruption(self) -> None:
        self.members[self.manifest_path] += b" "
        with self.assertRaisesRegex(RuntimeError, "digest/size mismatch"):
            self.check_archive()

    def test_rejects_disagreement_between_archive_views(self) -> None:
        self.members["manifest.json"] = encoded([{"Config": "other-config.json"}])
        with self.assertRaisesRegex(RuntimeError, "archive views disagree"):
            self.check_archive()

    def test_rejects_an_index_instead_of_a_runnable_manifest(self) -> None:
        expected = dict(self.manifest_descriptor, mediaType="application/vnd.oci.image.index.v1+json")
        with self.assertRaisesRegex(RuntimeError, "select an image manifest"):
            self.check_archive(expected)

    def test_rejects_extra_platforms_or_missing_provenance_children(self) -> None:
        self.members["index.json"] = encoded({"schemaVersion": 2, "manifests": [self.manifest_descriptor,
                                                     {"digest": "sha256:" + "1" * 64}]})
        with self.assertRaisesRegex(RuntimeError, "exactly one exported platform"):
            self.check_archive()

    def test_rejects_duplicate_members(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Duplicate archive member"):
            self.check_archive(duplicate=self.config_path)


if __name__ == "__main__":
    unittest.main()
