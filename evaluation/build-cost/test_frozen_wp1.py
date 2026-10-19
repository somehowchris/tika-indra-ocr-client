"""Adversarial complete-material and archive admission; no build/equivalence claim."""
import hashlib
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('frozen_wp1',ROOT/'scripts/prove-frozen-wp1.py');proof=importlib.util.module_from_spec(spec);spec.loader.exec_module(proof)


class FrozenMaterialTests(unittest.TestCase):
    def test_integrity_of_listed_files_does_not_establish_completeness(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'listed').write_bytes(b'present material')
            digest=hashlib.sha256(b'present material').hexdigest()
            (root/'SHA256SUMS').write_text(digest+'  ./listed\n')
            proof.verify_manifest(root)
            (root/'omitted').write_bytes(b'unlisted but required closure input')
            with self.assertRaisesRegex(ValueError,'inventory incomplete'):proof.verify_manifest(root)
            (root/'omitted').unlink();(root/'listed').write_bytes(b'corrupted material')
            with self.assertRaisesRegex(ValueError,'checksum differs'):proof.verify_manifest(root)

    def test_manifest_rejects_duplicate_alias_and_parent_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'file').write_bytes(b'x');digest=hashlib.sha256(b'x').hexdigest()
            for names in (['file','file'],['../file'],['./file'],['directory/../file']):
                (root/'SHA256SUMS').write_text(''.join(digest+'  ./'+name+'\n' for name in names))
                with self.assertRaises(ValueError):proof.verify_manifest(root)

    def test_full_tar_admission_precedes_any_extraction(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);archive=root/'source.tar'
            for bad in ('../outside','/absolute','./first','first'):
                with tarfile.open(archive,'w') as tar:
                    member=tarfile.TarInfo('first');member.size=1;tar.addfile(member,io.BytesIO(b'a'))
                    member=tarfile.TarInfo(bad);member.size=1;tar.addfile(member,io.BytesIO(b'b'))
                output=root/'output'
                with self.assertRaises(ValueError):proof.extract(archive,output)
                self.assertFalse(output.exists(),'unsafe archive must not leave extracted earlier members')

    def test_tar_links_are_rejected_and_ordinary_modes_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);archive=root/'source.tar'
            with tarfile.open(archive,'w') as tar:
                member=tarfile.TarInfo('binary');member.mode=0o755;member.size=1;tar.addfile(member,io.BytesIO(b'x'))
            proof.extract(archive,root/'valid')
            self.assertEqual((root/'valid/binary').stat().st_mode&0o777,0o755)
            with tarfile.open(archive,'w') as tar:
                member=tarfile.TarInfo('link');member.type=tarfile.SYMTYPE;member.linkname='/etc/passwd';tar.addfile(member)
            with self.assertRaises(ValueError):proof.extract(archive,root/'invalid')
            self.assertFalse((root/'invalid').exists())


if __name__=='__main__':unittest.main()
