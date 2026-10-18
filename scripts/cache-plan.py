#!/usr/bin/env python3
"""Build-only GHA cache ownership plan; no cache/network/build operation."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import re

# One writer per build purpose. Legacy v1 imports are migration reads only.
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
    if not workflow_ref.startswith(expected) or not workflow_ref[len(expected):]:
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
        write = head == repository
    elif event_name in ('push', 'workflow_dispatch'):
        if not re.fullmatch(r'refs/(?:heads|tags)/[^\s]+', ref):
            raise ValueError('invalid push/dispatch ref')
        write = True
    else:
        # In particular, never grant pull_request_target a cache writer.
        raise ValueError('unsupported cache writer event')
    return {'owner': owner, 'scope': scope, 'imports': [scope, *dependencies],
            'cache_from': '\n'.join(f'type=gha,scope={s}' for s in (scope, *dependencies)),
            'cache_to': f'type=gha,scope={scope},mode=max' if write else '',
            'writer_enabled': write, 'github_ref': ref,
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
