"""Cheap typed contracts only; real Docker/Chef/fresh-hosted proof remains separate."""
import base64
import copy
import importlib.util
import json
import os
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


if __name__=='__main__':unittest.main()
