"""Build-only ownership/ref contracts. No cache, Docker or compiler execution."""
import importlib.util
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('cache_plan', ROOT/'scripts/cache-plan.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
REPO = 'somehowchris/tika-indra-ocr-client'


def context(owner, *, event='push', ref='refs/heads/main', head=REPO):
    workflow = cache.OWNERS[owner][0]
    return dict(repository=REPO, workflow_ref=f'{REPO}/.github/workflows/{workflow}@{ref}',
                event_name=event, ref=ref,
                event={'pull_request': {'base': {'repo': {'full_name': REPO}},
                                        'head': {'repo': {'full_name': head}}}})


class CacheContracts(unittest.TestCase):
    def test_each_purpose_has_one_distinct_v2_writer(self):
        scopes = [v[1] for v in cache.OWNERS.values()]
        self.assertEqual(len(scopes), len(set(scopes)))
        self.assertTrue(all(s.endswith('-v2') for s in scopes))
        for owner, (_, scope, dependencies) in cache.OWNERS.items():
            with self.subTest(owner=owner):
                result = cache.plan(owner, **context(owner))
                self.assertEqual(result['cache_to'], f'type=gha,scope={scope},mode=max')
                self.assertEqual(result['imports'], [scope, *dependencies])
                self.assertEqual(len(result['imports']),len(set(result['imports'])))
                self.assertTrue(any(s.endswith('-v1') for s in dependencies))
                self.assertNotIn('-v1',result['cache_to'])

    def test_fork_pr_reads_explicit_migration_scopes_without_writing(self):
        for owner in cache.OWNERS:
            result=cache.plan(owner, **context(owner,event='pull_request',ref='refs/pull/42/merge',head='fork/ocr'))
            self.assertEqual(result['cache_to'],'')
            self.assertFalse(result['writer_enabled'])
            self.assertIn('type=gha,scope=',result['cache_from'])

    def test_same_repo_pr_and_branch_tag_dispatch_have_own_writer(self):
        for event,ref in [('pull_request','refs/pull/42/merge'),('push','refs/heads/feature'),
                          ('push','refs/tags/v-test'),('workflow_dispatch','refs/heads/feature')]:
            result=cache.plan('engine',**context('engine',event=event,ref=ref))
            self.assertTrue(result['writer_enabled'])
            self.assertEqual(result['github_ref'],ref)

    def test_unknown_fresh_runner_state_does_not_become_hit_or_restore_proof(self):
        result=cache.plan('native',**context('native'))
        self.assertIn('unknown',result['availability'])
        self.assertIn('no release admission',result['signing_authority'])
        self.assertNotIn('hit',result)
        self.assertNotIn('restored',result)

    def test_wrong_workflow_and_unsupported_privileged_event_rejected(self):
        for field,value in [('workflow_ref',f'{REPO}/.github/workflows/release.yml@refs/heads/main'),
                            ('event_name','pull_request_target'),('ref','refs/pull/42/merge'),
                            ('repository','bad\nrepository')]:
            args=context('engine');args[field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):cache.plan('engine',**args)
        args=context('engine',event='pull_request',ref='refs/pull/42/merge')
        args['event']['pull_request']['base']['repo']['full_name']='other/base'
        with self.assertRaises(ValueError):cache.plan('engine',**args)

    def test_direct_workflow_builds_consume_exact_owner_outputs(self):
        direct=[o for o in cache.OWNERS if not o.startswith('release-') or o.endswith('-acceptance')]
        for owner in direct:
            workflow=cache.OWNERS[owner][0]
            text=(ROOT/'.github/workflows'/workflow).read_text()
            ident='cache_'+owner.replace('-','_')
            self.assertEqual(text.count(f'id: {ident}\n'),1)
            self.assertIn(f'run: python3 scripts/cache-plan.py {owner} >> "$GITHUB_OUTPUT"',text)
            self.assertEqual(text.count('cache-from: ${{ steps.'+ident+'.outputs.cache_from }}'),1)
            self.assertEqual(text.count('cache-to: ${{ steps.'+ident+'.outputs.cache_to }}'),1)
        for path in (ROOT/'.github').rglob('*.yml'):
            for line in path.read_text().splitlines():
                if re.match(r'\s*cache-to:',line):
                    self.assertIn('.outputs.cache_to }}',line,str(path))

    def test_release_composite_uses_role_plan_not_caller_mutable_scope(self):
        text=(ROOT/'.github/actions/publish-image/action.yml').read_text()
        self.assertNotIn('inputs.cache-scope',text)
        self.assertIn('python3 scripts/cache-plan.py "release-$ROLE"',text)
        self.assertIn('cache-to: ${{ steps.cache.outputs.cache_to }}',text)
        release=(ROOT/'.github/workflows/release.yml').read_text()
        self.assertNotIn('cache-scope:',release)
        for role in ('native','gateway','bridge','tika','tesseract','backend'):
            self.assertIn('release-'+role,cache.OWNERS)


if __name__ == '__main__':
    unittest.main()
