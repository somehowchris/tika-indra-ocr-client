"""Adversarial complete-material and archive admission; no build/equivalence claim."""
import hashlib
import csv
import importlib.util
import io
import json
import shutil
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('frozen_wp1',ROOT/'scripts/prove-frozen-wp1.py');proof=importlib.util.module_from_spec(spec);spec.loader.exec_module(proof)


class ApplicationClockAmendmentTests(unittest.TestCase):
    def test_common_injection_preserves_native_prefix_and_original_cargo_commands(self):
        import subprocess
        originals=[]
        for revision in (proof.BASELINE,proof.CANDIDATE):
            original=subprocess.check_output(['git','show',revision+':docker/native.Dockerfile'],cwd=ROOT,text=True)
            amended=proof.amend_application_clock(original)
            marker='# Application compilation is cached independently'
            self.assertEqual(amended.split(marker,1)[0],original.split(marker,1)[0])
            cargo=lambda text:[line for line in text.splitlines() if 'cargo test ' in line or 'cargo build ' in line or 'cargo bench ' in line]
            self.assertEqual(cargo(amended),cargo(original))
            self.assertEqual(amended.count('ENV SOURCE_DATE_EPOCH=0'),1)
            for stage in ('engine-build','server-build','engine-profile-build'):
                before=original.split('FROM native-build AS '+stage+'\n',1)[1].split('\nFROM ',1)[0]
                after=amended.split('FROM application-build AS '+stage+'\n',1)[1].split('\nFROM ',1)[0]
                self.assertEqual(before,after)
            originals.append(amended.split('FROM native-build AS application-build\n',1)[1].split('\nFROM ',1)[0])
        self.assertEqual(*originals)
        for bad in (original.replace('AS engine-build','AS renamed'),amended,original.replace('# Application compilation is cached independently','# renamed delimiter')):
            with self.assertRaises(ValueError):proof.amend_application_clock(bad)

    def test_one_baseline_replica_role_mismatch_blocks_material_collection(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            def fake_static(path):
                mismatch='baseline-replicate' in str(path) and path.name=='ocr'
                return {'sha256':'b'*64 if mismatch else 'a'*64}
            fake=type('Audit',(),{'static_elf':staticmethod(fake_static)})()
            with patch.object(proof,'audit_module',return_value=fake):
                with self.assertRaisesRegex(ValueError,'same-source baseline seven-role byte identity differs'):
                    proof.compare_baseline_replicate(root)
            self.assertTrue((root/'baseline-replicate-seven-roles.json').exists())
            self.assertFalse((root/'baseline-replicate-comparison.json').exists())


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


class RequiredSourceCompletenessTests(unittest.TestCase):
    def make_context(self,root):
        files={'Cargo.toml','Cargo.lock','backend/models.lock.json','LICENSE','THIRD_PARTY_NOTICES.md'}
        for name in proof.SOURCE_ROOTS:
            target=root/name
            if name in files:
                target.parent.mkdir(parents=True,exist_ok=True);target.write_text('frozen original '+name)
            else:
                target.mkdir(parents=True,exist_ok=True);(target/'preserved.rs').write_text('frozen source '+name)
        # These are outside the declared collector roots and must not be required.
        (root/'README.md').write_text('not a release source operand')
        (root/'.github').mkdir();(root/'.github/workflow.yml').write_text('not collected')
        (root/'runtime/proofs').mkdir();(root/'runtime/proofs/proof.rs').write_text('explicitly excluded')
        (root/'runtime/target').mkdir();(root/'runtime/target/stale').write_text('must be excluded')

    def make_materials(self,context,source):
        source.mkdir()
        for name in proof.SOURCE_ROOTS:
            original=context/name;target=source/name
            target.parent.mkdir(parents=True,exist_ok=True)
            if original.is_dir():shutil.copytree(original,target,ignore=shutil.ignore_patterns('target','proofs'))
            else:shutil.copyfile(original,target)
        (source/'.cargo').mkdir();(source/'.cargo/config.toml').write_text('[source.vendored-sources]\ndirectory="vendor"\n')
        (source/'vendor').mkdir();(source/'vendor/source.rs').write_text('generated dependency source')

    def test_identical_model_lock_omission_in_both_arms_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context=root/'context';context.mkdir();self.make_context(context)
            for arm in ('baseline','candidate'):
                source=root/arm;self.make_materials(context,source)
                (source/'backend/models.lock.json').unlink()
                with self.subTest(arm=arm),self.assertRaisesRegex(ValueError,'required source material missing or changed: backend/models.lock.json'):
                    proof.verify_source_completeness(context,source)

    def test_frozen_roots_match_and_generated_paths_are_reported_separately(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context=root/'context';context.mkdir();self.make_context(context)
            source=root/'source';self.make_materials(context,source)
            result=proof.verify_source_completeness(context,source)
            self.assertTrue(result['required_source_files_match_context'])
            self.assertEqual({r['path'] for r in result['additional_generated_files']},{'.cargo/config.toml','vendor/source.rs'})
            self.assertNotIn('README.md',{r['path'] for r in result['required_context_files']})
            (source/'.cargo/config.toml').unlink()
            with self.assertRaisesRegex(ValueError,'vendor configuration missing'):proof.verify_source_completeness(context,source)


class NativeNoticeCompletenessTests(unittest.TestCase):
    def make_notices(self,root):
        sdk=root/'sdk';sdk.mkdir();collector=root/'collector.sh'
        names=[f'src/component{i}/LICENSE' for i in range(31)]
        collector.write_text("done <<'NATIVE_NOTICES'\n"+'\n'.join(f'component{i}|{name}' for i,name in enumerate(names))+'\nNATIVE_NOTICES\n')
        recipe={'notice_paths':names,'source':{'source_files':{'scripts/collect-runtime-materials.sh':proof.sha(collector)}}}
        (sdk/'recipe.json').write_text(json.dumps(recipe))
        materials=root/'materials';notices=materials/'licenses/native';notices.mkdir(parents=True)
        (notices/'README.txt').write_text('notice scope')
        rows=[]
        for i,name in enumerate(names):
            body=f'original SDK notice {i}'.encode();original=sdk/'rootfs/native'/name
            original.parent.mkdir(parents=True);original.write_bytes(body)
            destination=f'component{i}/LICENSE';copied=notices/destination
            copied.parent.mkdir();copied.write_bytes(body)
            rows.append({'component':f'component{i}','source_relative_path':name,'notice_relative_path':destination,
                         'bytes':str(len(body)),'sha256':hashlib.sha256(body).hexdigest()})
        self.write_rows(notices,rows)
        return materials,sdk,collector,rows

    def write_rows(self,notices,rows):
        with (notices/'inventory.tsv').open('w',newline='') as stream:
            writer=csv.DictWriter(stream,fieldnames=['component','source_relative_path','notice_relative_path','bytes','sha256'],delimiter='\t')
            writer.writeheader();writer.writerows(rows)

    def test_all_31_sdk_sources_and_destinations_are_verified(self):
        with tempfile.TemporaryDirectory() as temporary:
            materials,sdk,collector,rows=self.make_notices(Path(temporary))
            result=proof.verify_native_notices(materials,sdk,collector,proof.sha(sdk/'recipe.json'))
            self.assertEqual(len(result['verified_notices']),31)
            self.assertTrue(result['source_paths_and_actual_sdk_bytes_and_destinations_verified'])

    def test_same_notice_count_cannot_hide_wrong_source_destination_or_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            materials,sdk,collector,rows=self.make_notices(Path(temporary));notices=materials/'licenses/native'
            for field,value in [('source_relative_path',rows[1]['source_relative_path']),
                                ('notice_relative_path','component0/wrong-file'),('sha256','0'*64),('bytes','1')]:
                changed=[dict(row) for row in rows];changed[0][field]=value;self.write_rows(notices,changed)
                with self.subTest(field=field),self.assertRaises(ValueError):
                    proof.verify_native_notices(materials,sdk,collector,proof.sha(sdk/'recipe.json'))
            self.write_rows(notices,rows)
            (notices/rows[0]['notice_relative_path']).write_bytes(b'changed copy with unchanged inventory')
            with self.assertRaisesRegex(ValueError,'bytes differ from verified SDK source'):
                proof.verify_native_notices(materials,sdk,collector,proof.sha(sdk/'recipe.json'))


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
