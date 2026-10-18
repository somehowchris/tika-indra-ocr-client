"""Chef cache identity/admission contracts; never executes compiled cook in CI."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('chef_layout',ROOT/'scripts/chef-layout.py')
chef=importlib.util.module_from_spec(spec)
spec.loader.exec_module(chef)


class ChefLayoutTests(unittest.TestCase):
    def identity(self,**changes):
        args={'root_recipe':'1'*64,'runtime_recipe':'2'*64,'external_inputs':[{'path':'shared/trace-context/src/lib.rs','mode':0o644,'sha256':'3'*64}],
              'sdk':{'recipe_sha256':'4'*64,'content_sha256':'5'*64,'platform_manifest_digest':'sha256:'+'6'*64},
              'chef':{'binary_sha256':'7'*64},'helper_sha256':'8'*64}
        args.update(changes)
        return chef.cache_identity(**args)

    def test_all_actual_compiler_content_and_source_inputs_change_key(self):
        original=self.identity()['compiled_dependency_key']
        cases=[{'root_recipe':'a'*64},{'runtime_recipe':'b'*64},{'external_inputs':[{'path':'shared/trace-context/src/lib.rs','mode':0o644,'sha256':'a'*64}]},
               {'sdk':{'recipe_sha256':'a'*64,'content_sha256':'5'*64,'platform_manifest_digest':'sha256:'+'6'*64}},
               {'sdk':{'recipe_sha256':'4'*64,'content_sha256':'a'*64,'platform_manifest_digest':'sha256:'+'6'*64}},
               {'chef':{'binary_sha256':'a'*64}},{'helper_sha256':'a'*64}]
        for changed in cases:
            with self.subTest(changed=changed):self.assertNotEqual(original,self.identity(**changed)['compiled_dependency_key'])

    def test_distribution_metadata_does_not_masquerade_as_compiler_content(self):
        different={'recipe_sha256':'4'*64,'content_sha256':'5'*64,'platform_manifest_digest':'sha256:'+'a'*64}
        self.assertEqual(self.identity()['compiled_dependency_key'],self.identity(sdk=different)['compiled_dependency_key'])

    def test_invalid_or_mutable_sdk_identity_rejected(self):
        for invalid in ({}, {'recipe_sha256':1,'content_sha256':'5'*64}, {'recipe_sha256':'4'*64,'content_sha256':'5'*64,'platform_manifest_digest':'latest'}):
            with self.subTest(invalid=invalid),self.assertRaises(ValueError):self.identity(sdk=invalid)

    def test_actual_receipt_has_no_compiled_cook_and_real_shared_identity(self):
        evidence=ROOT/'evaluation/build-cost/evidence/chef-layout-2026-10-18'
        admission=chef.read(evidence/'admission.json')
        self.assertIs(admission['native_or_rust_compilation_requested'],False)
        self.assertIs(admission['source_inputs_unchanged'],True)
        for command in admission['commands']:
            self.assertEqual(command['exit_code'],0)
            if 'cook' in command['command']:self.assertIn('--no-build',command['command'])
        external=chef.read(evidence/'cache-identity.json')['external_workspace_inputs']
        shared=[e for e in external if e['path'].startswith('shared/trace-context/')]
        self.assertGreater(len(shared),1)
        self.assertIs(chef.read(evidence/'layout-contract.json')['original_shared_version_preserved'],True)


if __name__=='__main__':unittest.main()
