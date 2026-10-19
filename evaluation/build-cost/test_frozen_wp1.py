"""Adversarial complete-material and archive admission; no build/equivalence claim."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

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


class PreparedInputBindingTests(unittest.TestCase):
    def test_changed_actual_audit_is_rejected_before_any_build_command(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);cost=root/'evaluation/build-cost';cost.mkdir(parents=True)
            audit=cost/'audit.py';audit.write_text('# original audit semantics\n')
            plan=cost/'evidence/seven-role-plan-2026-10-18.json';plan.parent.mkdir()
            plan.write_text(json.dumps({'sdk':{}}))
            output=root/'prepared';output.mkdir()
            (output/'plan.json').write_bytes(plan.read_bytes())
            plan_sha=hashlib.sha256(plan.read_bytes()).hexdigest()
            (output/'prepared.json').write_text(json.dumps({'sdk_layout':str(root/'sdk'),
                'plan_sha256':plan_sha,'source_archive_sha256':'0'*64}))
            (output/'controller-identity.json').write_text(json.dumps({
                'controller_source_sha':'1'*40,'controller_sha256':proof.sha(Path(proof.__file__)),
                'audit_sha256':proof.sha(audit),'plan_sha256':plan_sha}))
            audit.write_text('# actual audit semantics changed after preparation\n')
            # Exercise the real run entry point and an actual changed audit file.
            # A missing guard would reach the builder observation and fail this spy.
            with patch.object(proof,'ROOT',root),patch.object(proof,'PLAN',plan), \
                 patch('sys.argv',['prove-frozen-wp1.py','run','--output',str(output)]), \
                 patch.object(proof.subprocess,'run',side_effect=AssertionError('build/phase command reached')), \
                 patch.object(proof.subprocess,'check_output',side_effect=AssertionError('source lookup reached before audit guard')):
                with self.assertRaisesRegex(ValueError,'audit changed after preparation'):
                    proof.main()
            self.assertFalse((output/'execution.json').exists())

    def test_recorded_source_binding_rejects_dirty_tool_bytes(self):
        with patch.object(proof.subprocess,'check_output',return_value=b'committed tool bytes'):
            with self.assertRaisesRegex(ValueError,'does not match producing source'):
                proof.verify_source_binding('1'*40,{'scripts/tool.py':hashlib.sha256(b'dirty tool bytes').hexdigest()})

    def test_unrelated_later_commit_does_not_rebind_the_producer(self):
        source='1'*40;body=b'committed tool bytes'
        with patch.object(proof.subprocess,'check_output',return_value=body) as command:
            proof.verify_source_binding(source,{'scripts/tool.py':hashlib.sha256(body).hexdigest()})
        self.assertEqual(command.call_args.args[0],['git','show',source+':scripts/tool.py'])


if __name__=='__main__':unittest.main()
