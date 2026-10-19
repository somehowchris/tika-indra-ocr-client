"""Cheap typed contracts only; real Docker/Chef/fresh-hosted proof remains separate."""
import base64
import copy
import importlib.util
import json
import os
import hashlib
import shutil
import tarfile
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


index=module('build_index_test',ROOT/'scripts/native-build-index.py')
proof=module('build_proof_test',ROOT/'scripts/native-build-proof.py')
cost=module('build_cost_schedule_test',ROOT/'scripts/measure-native-build-cost.py')
chef=module('chef_clean_test',ROOT/'scripts/chef-artifact.py')
material_fixtures=module('current_material_fixtures',ROOT/'evaluation/build-cost/test_frozen_wp1.py')


class NativeBuildProofTests(unittest.TestCase):
    def test_existing_audit_api_is_callable_before_expensive_builds(self):
        self.assertTrue(callable(proof.audit.static_elf));self.assertTrue(callable(proof.audit.inventory));self.assertTrue(callable(cost.audit.parse_log))

    def test_only_same_trusted_vtag_release_invocation_is_admitted(self):
        env={'GITHUB_REPOSITORY':'somehowchris/tika-indra-ocr-client','GITHUB_SHA':'a'*40,'GITHUB_REF':'refs/tags/v-test',
             'GITHUB_WORKFLOW_REF':'somehowchris/tika-indra-ocr-client/.github/workflows/release.yml@refs/tags/v-test',
             'GITHUB_RUN_ID':'123','GITHUB_RUN_ATTEMPT':'1'}
        with patch.dict(os.environ,env,clear=True):
            identity=index.invocation()
            value={'schema_version':1,'kind':'native-build-inputs',**identity,'production_candidate_index_sha256':'a'*64,
                   'inputs':{role:{'reference':'ghcr.io/somehowchris/tika-indra-native-'+role+'@sha256:'+'b'*64,'digest':'sha256:'+'b'*64,**{k:'c'*64 for k in ('recipe_sha256','content_sha256','oci_receipt_sha256','spdx_sha256')}} for role in ('sdk','chef')},
                   'baseline_roles':{role:{'sha256':'d'*64,'bytes':1} for role in index.ROLES}}
            index.validate(value,identity)
            for key,bad in [('source_sha','f'*40),('source_ref','refs/heads/main'),('workflow_identity','https://example.invalid')]:
                changed=copy.deepcopy(value);changed[key]=bad
                with self.assertRaises(ValueError):index.validate(changed,identity)
            changed=copy.deepcopy(value);changed['baseline_roles'].pop('ocr')
            with self.assertRaises(ValueError):index.validate(changed,identity)
            with patch.dict(os.environ,{'GITHUB_REF':'refs/heads/completion/sdk-chef-proof'}):
                with self.assertRaises(ValueError):index.invocation()
            with patch.dict(os.environ,{'NATIVE_BUILD_RELEASE_RUN_ID':'999'}):
                with self.assertRaises(ValueError):index.invocation()

    def test_attestation_requires_exact_predicate_and_subject(self):
        predicate={'real':'input'};digest='sha256:'+'a'*64
        def envelope(value,subject):return json.dumps({'payloadType':'application/vnd.in-toto+json','payload':base64.b64encode(json.dumps({'predicate':value,'subject':[{'digest':{'sha256':subject}}]}).encode()).decode()})
        proof.require_attestation(envelope(predicate,'a'*64),predicate,digest)
        for bad in (envelope({'old':'input'},'a'*64),envelope(predicate,'b'*64)):
            with self.assertRaises(ValueError):proof.require_attestation(bad,predicate,digest)
        for empty in ('','[]',' \n[] \n'):proof.initial_cache_empty(empty)
        for nonempty in ('{}','[{}]','{"ID":"cache"}'):
            with self.assertRaises(ValueError):proof.initial_cache_empty(nonempty)

    def test_cleanup_requires_exact_name_owner_image_and_id(self):
        record={'Name':'/unique','Image':'sha256:'+'a'*64,'Id':'b'*64,'Config':{'Labels':{'org.tika-indra.native-build-proof':'token'}}}
        self.assertEqual(proof.owned_container(record,'unique','token','sha256:'+'a'*64),'b'*64)
        for key,value in [('Name','/other'),('Image','sha256:'+'c'*64),('Id','not-an-id')]:
            bad=copy.deepcopy(record);bad[key]=value
            with self.assertRaises(ValueError):proof.owned_container(bad,'unique','token','sha256:'+'a'*64)
        bad=copy.deepcopy(record);bad['Config']['Labels']['org.tika-indra.native-build-proof']='other'
        with self.assertRaises(ValueError):proof.owned_container(bad,'unique','token','sha256:'+'a'*64)

    def test_inner_transport_preserves_modes_without_links(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);packet=root/'packet';packet.mkdir();(packet/'binary').write_bytes(b'ELF fixture');(packet/'binary').chmod(0o755)
            nested=packet/'materials';nested.mkdir(mode=0o775);(nested/'source').write_bytes(b'source');(nested/'source').chmod(0o664)
            index.archive_packet(packet,root/'transport');index.unpack_packet(root/'transport',root/'restored')
            for name in ('binary','materials/source','materials'):
                self.assertEqual((packet/name).stat().st_mode&0o777,(root/'restored'/name).stat().st_mode&0o777)
            (packet/'link').symlink_to('/etc/passwd')
            with self.assertRaises(ValueError):index.archive_packet(packet,root/'bad')

    def test_explicit_clean_uses_actual_normalized_workspace_names(self):
        metadata={'workspace_members':['local#a@0.0.1','local#b@0.0.1'],'packages':[{'id':'local#a@0.0.1','name':'a'},{'id':'local#b@0.0.1','name':'b'},{'id':'registry#external','name':'external'}]}
        with tempfile.TemporaryDirectory() as d,patch.object(chef.subprocess,'check_output',return_value=json.dumps(metadata)),patch.object(chef.subprocess,'run') as command:
            chef.clean_workspace(Path(d)/'cleanup.json')
            self.assertEqual(command.call_args.args[0],['cargo','clean','--release','--target','x86_64-unknown-linux-gnu','--package','a','--package','b'])

    def test_cost_schedule_is_frozen_three_pairs_with_no_percentile_claim(self):
        schedule=cost.schedule();self.assertEqual(len(schedule),9)
        for scenario in ('warm','source-comment','material-doc'):
            rows=[v for v in schedule if v['scenario']==scenario]
            self.assertEqual([r['repetition'] for r in rows],[1,2,3]);self.assertEqual([r['order'] for r in rows],[['sdk','chef'],['chef','sdk'],['sdk','chef']])
        self.assertEqual(cost.summary([2,1,3]),{'n':3,'min_seconds':1,'median_seconds':2,'max_seconds':3})
        for bad in ([1,2],[1,2,float('nan')],[1,2,float('inf')],[1,2,-1]):
            with self.assertRaises(ValueError):cost.summary(bad)

    def test_opt_in_job_preserves_six_image_schema_and_read_only_consumer(self):
        workflow=(ROOT/'.github/workflows/release.yml').read_text()
        self.assertIn('run_sdk_proof:\n',workflow);self.assertIn('default: false',workflow.split('run_sdk_proof:',1)[1].split('run_evaluation:',1)[0])
        self.assertLess(workflow.index('bash scripts/sign-images.sh'),workflow.index('bash scripts/publish-native-build-inputs.sh'))
        job=workflow.split('  native-build-proof:\n',1)[1]
        self.assertIn('timeout-minutes: 360',job);self.assertNotIn('id-token:',job);self.assertNotIn('packages: write',job)
        self.assertNotIn('cache-from:',job);self.assertNotIn('cache-to:',job)
        self.assertIn("startsWith(github.ref, 'refs/tags/v')",job)
        self.assertIn('.images | length == 6',(ROOT/'scripts/sign-images.sh').read_text())
        fixture=(ROOT/'evaluation/build-cost/chef-native-fixture.Dockerfile.inc').read_text()
        self.assertIn('NATIVE_FIXTURE_VALUE=1',fixture);self.assertIn('chef-native-fixture.py',fixture);self.assertIn('touch -d @1',fixture)


class CurrentMaterialAdmissionTests(unittest.TestCase):
    def fixture(self,root):
        context=root/'context';context.mkdir()
        material_fixtures.RequiredSourceCompletenessTests().make_context(context)
        for name in ('docs/SETUP.md','docs/runtime-relinking.md','docs/relinking.md','licenses/INDRA-MODEL-LICENSE.txt','licenses/INDRA-CODE-LICENSE.txt'):
            (context/name).write_text('original '+name)
        engine=context/'runtime/crates/indra-engine/src/lib.rs';engine.parent.mkdir(parents=True);engine.write_text('// original engine source\n')
        native=root/'native-fixture';native.mkdir()
        materials,sdk,collector,rows=material_fixtures.NativeNoticeCompletenessTests().make_notices(native)
        (context/'scripts/collect-runtime-materials.sh').write_bytes(collector.read_bytes())
        source=root/'released-source'
        material_fixtures.RequiredSourceCompletenessTests().make_materials(context,source)
        for destination,original in (('README.md','docs/runtime-relinking.md'),('relinking.md','docs/relinking.md'),('LICENSE','LICENSE'),('THIRD_PARTY_NOTICES.md','THIRD_PARTY_NOTICES.md'),('licenses/INDRA-MODEL-LICENSE.txt','licenses/INDRA-MODEL-LICENSE.txt'),('licenses/INDRA-CODE-LICENSE.txt','licenses/INDRA-CODE-LICENSE.txt')):
            shutil.copyfile(context/original,materials/destination)
        self.archive_and_manifest(materials,source)
        return context,materials,sdk,source,rows

    def archive_and_manifest(self,materials,source):
        with tarfile.open(materials/'rust-source.tar.gz','w:gz') as archive:
            for path in sorted(source.iterdir()):archive.add(path,arcname=path.name)
        (materials/'SHA256SUMS').write_text(''.join(
            hashlib.sha256(path.read_bytes()).hexdigest()+'  ./'+path.relative_to(materials).as_posix()+'\n'
            for path in sorted(materials.rglob('*')) if path.is_file() and path.name!='SHA256SUMS'))

    def verify(self,root,context,materials,sdk):
        return proof.verify_materials(materials,context,sdk,proof.index_api.sha(sdk/'recipe.json'),root/'receipt.json')

    def test_actual_source_and_31_sdk_notices_pass_with_bound_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,materials,sdk,source,_=self.fixture(root)
            result=self.verify(root,context,materials,sdk)
            self.assertTrue(result['source_completeness']['required_source_files_match_context'])
            self.assertEqual(len(result['native_notices']['verified_notices']),31)
            self.assertEqual(result['source_archive_sha256'],proof.index_api.sha(materials/'rust-source.tar.gz'))
            self.assertEqual(list(root.glob('material-source-*')),[])

    def test_identical_rehashed_omission_cannot_pass_either_arm(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,materials,sdk,source,_=self.fixture(root)
            (source/'backend/models.lock.json').unlink();self.archive_and_manifest(materials,source)
            for arm in ('baseline','sdk','chef'):
                destination=root/arm;destination.mkdir()
                with self.subTest(arm=arm),self.assertRaisesRegex(ValueError,'required source material missing or changed: backend/models.lock.json'):
                    self.verify(destination,context,materials,sdk)
                self.assertIs(json.loads((destination/'receipt.json').read_text())['passed'],False)

    def test_stale_self_consistent_bundle_rejects_actual_source_or_doc_edit(self):
        for name in ('runtime/crates/indra-engine/src/lib.rs','docs/SETUP.md'):
            with self.subTest(path=name),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary);context,materials,sdk,source,_=self.fixture(root)
                (context/name).write_text('actual current mutated benchmark input')
                proof.materials_api.verify_manifest(materials)
                with self.assertRaisesRegex(ValueError,'required source material missing or changed: '+name):
                    self.verify(root,context,materials,sdk)
                self.assertIs(json.loads((root/'receipt.json').read_text())['passed'],False)

    def test_current_mutated_source_and_doc_bytes_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,materials,sdk,source,_=self.fixture(root)
            for name in ('runtime/crates/indra-engine/src/lib.rs','docs/SETUP.md'):
                (context/name).write_text('actual current mutated benchmark input')
                (source/name).write_bytes((context/name).read_bytes())
            self.archive_and_manifest(materials,source)
            result=self.verify(root,context,materials,sdk)
            required={row['path']:row for row in result['source_completeness']['required_context_files']}
            for name in ('runtime/crates/indra-engine/src/lib.rs','docs/SETUP.md'):
                self.assertEqual(required[name]['sha256'],proof.index_api.sha(context/name))

    def test_rehashed_notice_forgery_cannot_replace_sdk_source_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,materials,sdk,source,rows=self.fixture(root)
            notice=materials/'licenses/native'/rows[0]['notice_relative_path'];notice.write_bytes(b'rehashed forged notice')
            rows[0]['sha256']=proof.index_api.sha(notice);rows[0]['bytes']=str(notice.stat().st_size)
            material_fixtures.NativeNoticeCompletenessTests().write_rows(materials/'licenses/native',rows)
            self.archive_and_manifest(materials,source)
            proof.materials_api.verify_manifest(materials)
            with self.assertRaisesRegex(ValueError,'native notice bytes differ from verified SDK source'):
                self.verify(root,context,materials,sdk)

    def test_rehashed_stale_outer_relink_instructions_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,materials,sdk,source,_=self.fixture(root)
            (materials/'README.md').write_text('stale but self-consistent outer instructions')
            self.archive_and_manifest(materials,source)
            proof.materials_api.verify_manifest(materials)
            with self.assertRaisesRegex(ValueError,'copied source material missing or stale: README.md'):
                self.verify(root,context,materials,sdk)

    def test_same_count_wrong_notice_source_and_unlisted_material_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);context,materials,sdk,source,rows=self.fixture(root)
            rows[0]['source_relative_path']=rows[1]['source_relative_path']
            material_fixtures.NativeNoticeCompletenessTests().write_rows(materials/'licenses/native',rows)
            self.archive_and_manifest(materials,source)
            with self.assertRaisesRegex(ValueError,'native notice source path set differs'):self.verify(root,context,materials,sdk)
            (materials/'unlisted-required-file').write_text('must not hide behind listed checksums')
            second=root/'second';second.mkdir()
            with self.assertRaisesRegex(ValueError,'material checksum inventory incomplete or extra'):self.verify(second,context,materials,sdk)


if __name__=='__main__':unittest.main()
