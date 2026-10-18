#!/usr/bin/env python3
"""Build-only GHA cache ownership plan; no cache/network/build operation."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import re

# Distinct purpose scopes; all leaves are read-only. Legacy imports are reads only.
OWNERS = {
    'native': ('native-runtime.yml', 'indra-native-sdk-gnu-v2', ('indra-native-gnu-v1',)),
    'engine': ('native-engine.yml', 'indra-engine-gnu-v2', ('indra-native-sdk-gnu-v2', 'indra-native-gnu-v1')),
    'server': ('native-service.yml', 'indra-server-gnu-v2', ('indra-native-sdk-gnu-v2', 'indra-native-gnu-v1')),
    'profile': ('native-profile.yml', 'indra-profile-gnu-v2', ('indra-native-sdk-gnu-v2', 'indra-native-profile-gnu-v1', 'indra-native-gnu-v1')),
    'stack-native': ('tika-stack.yml', 'indra-stack-native-gnu-v2', ('indra-native-sdk-gnu-v2', 'indra-server-gnu-v2', 'indra-native-gnu-v1')),
    'stack-gateway': ('tika-stack.yml', 'indra-stack-gateway-gnu-v2', ('indra-gateway-gnu-v1',)),
    'release-native-acceptance': ('release.yml', 'indra-release-native-acceptance-gnu-v2', ('indra-native-sdk-gnu-v2', 'indra-server-gnu-v2', 'indra-native-gnu-v1')),
    'release-gateway-acceptance': ('release.yml', 'indra-release-gateway-acceptance-gnu-v2', ('indra-gateway-gnu-v1',)),
    'release-native': ('release.yml', 'indra-release-native-gnu-v2', ('indra-release-native-acceptance-gnu-v2', 'indra-native-sdk-gnu-v2', 'indra-native-gnu-v1')),
    'release-gateway': ('release.yml', 'indra-release-gateway-gnu-v2', ('indra-release-gateway-acceptance-gnu-v2', 'indra-gateway-gnu-v1')),
    'release-bridge': ('release.yml', 'indra-release-bridge-gnu-v2', ('indra-bridge-gnu-v1',)),
    'release-tika': ('release.yml', 'indra-release-tika-v2', ('indra-tika-v1',)),
    'release-tesseract': ('release.yml', 'indra-release-tesseract-v2', ('indra-tesseract-v1',)),
    'release-backend': ('release.yml', 'indra-release-python-reference-v2', ('indra-python-reference-v1',)),
}


def plan(owner, *, repository, workflow_ref, event_name, ref, event):
    workflow, scope, dependencies = OWNERS[owner]
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository):
        raise ValueError('invalid repository')
    expected = f'{repository}/.github/workflows/{workflow}@'
    direct = workflow_ref.startswith(expected) and bool(workflow_ref[len(expected):])
    # A reusable workflow inherits the caller's github context. Stack consumers
    # remain read-only regardless of caller identity, event or supplied inputs.
    reusable_stack = owner in ('stack-native', 'stack-gateway') and re.fullmatch(
        re.escape(repository) + r'/\.github/workflows/[A-Za-z0-9_.-]+\.ya?ml@[^\s]+',
        workflow_ref) is not None
    if not direct and not reusable_stack:
        raise ValueError('cache owner belongs to a different workflow')
    if event_name == 'pull_request':
        if not re.fullmatch(r'refs/pull/[1-9][0-9]*/merge', ref):
            raise ValueError('invalid pull request ref')
        pr = event['pull_request']
        if pr['base']['repo']['full_name'] != repository:
            raise ValueError('pull request base repository mismatch')
        head = pr['head']['repo']['full_name']
        if not isinstance(head, str) or not head:
            raise ValueError('missing pull request head repository')
        writer_reason = 'pull requests import only, including same-repository merge refs'
    elif event_name in ('push', 'workflow_dispatch'):
        if not re.fullmatch(r'refs/(?:heads|tags)/[^\s]+', ref):
            raise ValueError('invalid push/dispatch ref')
        writer_reason = 'leaf imports only; producer requires explicit default-branch dispatch'
    else:
        # In particular, never grant pull_request_target a cache writer.
        raise ValueError('unsupported cache writer event')
    write = False
    requested = event.get('inputs', {}).get('export_native_cache', False)
    if not (isinstance(requested, bool) or
            isinstance(requested, str) and requested in ('', 'false', 'true')):
        raise ValueError('invalid native cache export opt-in')
    if owner == 'native' and requested in (True, 'true'):
        # Dispatch-only prevents enabling an automatic main native build.
        # The default ref is eligible for consumer reads; availability is unknown.
        default = event.get('repository', {}).get('default_branch')
        if not isinstance(default, str) or not re.fullmatch(r'[^\s]+', default):
            raise ValueError('native export requires an identified default branch')
        if event_name != 'workflow_dispatch' or ref != f'refs/heads/{default}':
            raise ValueError('native export requires explicit default-branch dispatch')
        write = True
        writer_reason = 'explicit default-branch producer dispatch; external acceptance required'
    return {'owner': owner, 'scope': scope, 'imports': [scope, *dependencies],
            'cache_from': '\n'.join(f'type=gha,scope={s}' for s in (scope, *dependencies)),
            'cache_to': f'type=gha,scope={scope},mode=max' if write else '',
            'writer_enabled': write, 'writer_reason': writer_reason, 'github_ref': ref,
            'workflow_context': 'direct owner' if direct else 'read-only reusable stack caller',
            'availability': 'unknown; plan does not attest a hit, blob availability or fresh-runner restoration',
            'signing_authority': 'unchanged; cache ownership grants no release admission'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('owner', choices=OWNERS)
    args = parser.parse_args()
    event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
    result = plan(args.owner, repository=os.environ['GITHUB_REPOSITORY'],
                  workflow_ref=os.environ['GITHUB_WORKFLOW_REF'],
                  event_name=os.environ['GITHUB_EVENT_NAME'], ref=os.environ['GITHUB_REF'], event=event)
    # Fixed owner scopes contain no untrusted multiline values.
    print('cache_from<<INDRA_CACHE_IMPORTS\n' + result['cache_from'] + '\nINDRA_CACHE_IMPORTS')
    print('cache_to=' + result['cache_to'])
    print('scope=' + result['scope'])
    print(json.dumps(result, sort_keys=True), file=__import__('sys').stderr)


if __name__ == '__main__':
    main()
