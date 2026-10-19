"""Integrity checks for real local SDK receipts; no fresh-builder or speed claim."""
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest

ROOT=Path(__file__).resolve().parents[2]
EVIDENCE=ROOT/'evaluation/build-cost/evidence/native-sdk-2026-10-18'
spec=importlib.util.spec_from_file_location('sdk',ROOT/'scripts/native-sdk.py')
sdk=importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk)


class ActualSdkEvidenceTests(unittest.TestCase):
    def test_every_retained_receipt_matches_its_hash_and_length(self):
        for record in sdk.read(EVIDENCE/'receipts.sha256.json'):
            path=EVIDENCE/record['path']
            self.assertTrue(path.resolve().is_relative_to(EVIDENCE.resolve()))
            self.assertEqual(path.stat().st_size,record['bytes'])
            self.assertEqual(sdk.sha(path),record['sha256'])

    def test_recipe_content_source_and_oci_identities_are_distinct_and_bound(self):
        producer=sdk.read(EVIDENCE/'producer.json')
        material=sdk.read(EVIDENCE/'materialization.json')
        for name in ('recipe','content'):
            self.assertEqual(sdk.sha(EVIDENCE/(name+'.json')),producer[name+'_sha256'])
            self.assertEqual(producer[name+'_sha256'],material[name+'_sha256'])
        self.assertEqual(producer['source_commit'],(EVIDENCE/'source-commit.txt').read_text().strip())
        manifest=sdk.read(EVIDENCE/'oci-manifest.json')
        self.assertEqual('sha256:'+sdk.sha(EVIDENCE/'oci-manifest.json'),material['platform_manifest_digest'])
        self.assertEqual('sha256:'+sdk.sha(EVIDENCE/'oci-config.json'),manifest['config']['digest'])
        self.assertEqual(sdk.sha(EVIDENCE/'oci-index.json'),material['index_sha256'])
        self.assertEqual(material['requested_digest'],material['platform_manifest_digest'])
        self.assertEqual(len(manifest['layers']),1)

    def test_exact_archive_and_required_notice_content_is_preserved(self):
        content=sdk.read(EVIDENCE/'content.json')
        files={entry['path']:entry for entry in content['entries'] if entry['type']=='file'}
        names=sdk.notice_paths(ROOT/'scripts/collect-runtime-materials.sh')
        self.assertEqual(len(names),31)
        for name in names:self.assertIn('native/'+name,files)
        archives={}
        for line in (EVIDENCE/'native-build-evidence/static-archives.sha256').read_text().splitlines():
            digest,path=line.split('  ',1)
            archives[path.lstrip('/')]=digest
        self.assertGreater(len(archives),20)
        for path,digest in archives.items():self.assertEqual(files[path]['sha256'],digest,path)
        actual={path for path in files if path.startswith('native/') and path.endswith('.a')}
        self.assertEqual(actual,set(archives))
        self.assertIn('native/cargo-env.sh',files)
        self.assertEqual(len(content['entries']),557)

    def test_actual_success_is_shared_builder_only_and_failures_are_retained(self):
        summary=sdk.read(EVIDENCE/'summary.json')
        self.assertIs(summary['fresh_builder_claim'],False)
        self.assertIs(summary['seven_role_equivalence_claim'],False)
        self.assertIs(summary['speedup_claim'],False)
        self.assertEqual(sdk.read(EVIDENCE/'producer-phase-full-source/phase.json')['exit_code'],0)
        self.assertEqual(sdk.read(EVIDENCE/'consumer-admission-phase/phase.json')['exit_code'],0)
        self.assertEqual(sdk.read(EVIDENCE/'original-archive-mode-cancelled/phase.json')['exit_code'],130)
        self.assertNotEqual(sdk.read(EVIDENCE/'producer-phase/phase.json')['exit_code'],0)
        self.assertIn('restored_entries', (EVIDENCE/'consumer-admission-phase/stderr').read_text())


if __name__=='__main__':unittest.main()
