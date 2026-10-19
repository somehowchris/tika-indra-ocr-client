#!/usr/bin/env python3
"""Test the actual Rust offline CLI against mutated copies; never execute OCR."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('verifier', type=Path)
    parser.add_argument('extracted_study', type=Path)
    parser.add_argument('new_output', type=Path)
    args = parser.parse_args()
    if args.new_output.exists() or args.new_output.is_symlink():
        raise ValueError('output must be new')
    args.new_output.mkdir(mode=0o700, parents=True)
    run = args.extracted_study / 'cpu-1-r0-enabled/run'
    goldens = args.extracted_study / 'reference-cpu-1/run'
    identity = args.extracted_study / 'control/build-identity.json'
    results = []
    for kind in ['response_hash', 'duplicate_asset', 'private_sentry', 'arbitrary_context', 'wrong_parent']:
        destination = args.new_output / kind
        shutil.copytree(run, destination, symlinks=True)
        if kind == 'response_hash':
            path = destination / 'requests.jsonl'
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            rows[0]['output_sha256'][0] ^= 1
            path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
            expected = 'offline golden response hash differs'
        elif kind == 'duplicate_asset':
            path = destination / 'assets.jsonl'
            path.write_text(path.read_text() + path.read_text().splitlines()[0] + '\n')
            expected = 'offline duplicate/mismatched asset'
        elif kind in ['private_sentry', 'arbitrary_context']:
            path = next(destination.glob('sentry-*.envelope'))
            first, second, data = path.read_text().split('\n', 2)
            event = json.loads(data)
            if kind == 'private_sentry':
                event['exception']['values'][0]['value'] = 'SDK_PRIVATE_CANARY'
                expected = 'private Sentry payload'
            else:
                event['contexts']['trace']['document'] = 'unbounded arbitrary context'
                expected = 'unexpected Sentry context payload'
            payload = json.dumps(event)
            item = json.loads(second); item['length'] = len(payload.encode())
            path.write_text(first + '\n' + json.dumps(item) + '\n' + payload + '\n')
        else:
            path = destination / 'spans.json'
            rows = json.loads(path.read_text())
            for row in rows:
                if row['kind'] == 'gateway_http' and row['trace'][-1] == 1:
                    row['parent'] = [0] * 8
                    break
            else:
                raise ValueError('actual fixed-parent gateway span missing')
            path.write_text(json.dumps(rows))
            expected = 'scheduled gateway parent absent'
        result_file = args.new_output / (kind + '.json')
        result = subprocess.run([str(args.verifier.resolve()), 'verify', '--evidence', str(destination),
                                 '--goldens', str(goldens), '--identity', str(identity),
                                 '--output', str(result_file)], capture_output=True, text=True)
        (args.new_output / (kind + '.stdout')).write_text(result.stdout)
        (args.new_output / (kind + '.stderr')).write_text(result.stderr)
        if result.returncode == 0 or result_file.exists() or expected not in result.stderr:
            raise ValueError(f'{kind} did not reach the expected rejecting Rust gate: {result.stderr}')
        results.append({'case': kind, 'rust_exit': result.returncode, 'expected_gate': expected,
                        'no_success_receipt_written': True})
    (args.new_output / 'summary.json').write_text(json.dumps({'scope': 'Actual Rust offline CLI; only isolated evidence copies were mutated.',
                                                            'negative_cases_passed': len(results), 'cases': results}, indent=2) + '\n')
    print(json.dumps({'negative_cases_passed': len(results)}))


if __name__ == '__main__':
    main()
