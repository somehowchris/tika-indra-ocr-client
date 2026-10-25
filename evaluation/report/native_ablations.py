"""Render all descriptive native ablation pairs through pinned Seaborn.

No OCR, native build, fitted interval, request pooling or default promotion.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

import numpy as np
import pandas as pd
import seaborn as sns

from study import require

ROOT = Path(__file__).resolve().parents[2]
PACKET = ROOT / 'runtime/validation/native-ablations/2026-10-18'
FIGURES = ROOT / 'evaluation/figures'
COMPARISONS = ['fat-lto', 'spinning-off', 'batch-1', 'batch-8']
PAIRS = [0, 1, 2, 3]
COLORS = ['#345995', '#B65D20', '#3C8050', '#8556A7']
ORDERS = ['AB', 'BA', 'BA', 'AB']
LABELS = ['Fat LTO', 'Spinning off', 'Batch cap 1', 'Batch cap 8']


def load() -> pd.DataFrame:
    expected = dict(row.split('  ', 1)[::-1] for row in (PACKET / 'SHA256SUMS').read_text().splitlines())
    for name in ['paired.json', 'admission.json', 'protocol.json']:
        require(hashlib.sha256((PACKET / name).read_bytes()).hexdigest() == expected[name],
                f'native ablation packet identity differs: {name}')
    admission = json.loads((PACKET / 'admission.json').read_text())
    analysis = json.loads((PACKET / 'paired.json').read_text())
    require(admission['admitted'] is True and admission['measured_processes'] == 128
            and admission['measured_requests'] == 4096 and admission['process_pairs'] == 64,
            'incomplete native ablation admission')
    require(len(analysis['pairs']) == 64, 'incomplete native ablation pairs')
    require(all(d['decision'].startswith('retain baseline') for d in analysis['decisions']),
            'native ablation default decision changed')
    records = []
    identities = set()
    for pair in analysis['pairs']:
        key = (pair['cpus'], pair['fixture'], pair['comparison'], pair['block'])
        require(key not in identities and pair['order'] == ORDERS[pair['block']], 'duplicate/wrong-order native ablation pair')
        identities.add(key)
        a, b = pair['baseline'], pair['candidate']
        require(a['arm'] == 'A' and b['arm'] == 'B' and a['request_count'] == b['request_count'] == 32,
                'invalid native ablation arms/request count')
        for arm in [a, b]:
            require(all(arm[field] == pair[field] for field in ['cpus', 'fixture', 'comparison', 'block', 'order']),
                    'native ablation pair identity differs')
        ratio = b['p50_us'] / a['p50_us']
        require(ratio == pair['median_ratio'] and np.isfinite(ratio) and 0 < ratio < 1.4,
                'native ablation paired median ratio differs or exceeds frozen plot bounds')
        records.append({'cpus': pair['cpus'], 'fixture': pair['fixture'], 'comparison': pair['comparison'],
                        'block': pair['block'], 'median_ratio': ratio, 'baseline_id': a['id'], 'candidate_id': b['id']})
    require(identities == {(cpus, fixture, comparison, pair) for cpus in [1, 2] for fixture in ['tiff', 'math']
                          for comparison in COMPARISONS for pair in PAIRS}, 'native ablation factorial coverage differs')
    return pd.DataFrame(records)


def render() -> dict:
    data = load()
    sns.set_theme(style='whitegrid', context='notebook', font='DejaVu Sans',
                  rc={'svg.hashsalt': 'tika-indra-native-ablation-v1'})
    graph = sns.catplot(data=data, kind='strip', x='comparison', y='median_ratio', hue='block', row='cpus', col='fixture',
                        order=COMPARISONS, hue_order=PAIRS, row_order=[1, 2], col_order=['tiff', 'math'],
                        palette=COLORS, jitter=False, dodge=True, size=7, linewidth=0.7, edgecolor='white',
                        height=3.4, aspect=1.45, sharex=True, sharey=True, legend=False)
    graph.refline(y=1, color='#666666', linestyle='--', linewidth=1)
    graph.set(ylim=(0, 1.4))
    graph.set_axis_labels('', '')
    graph.figure.supylabel('Paired median latency ratio (candidate / baseline)', x=0.01, fontsize=12)
    graph.set_titles('{row_name} CPU ceiling · {col_name}')
    graph.set_xticklabels(LABELS)
    graph.figure.suptitle('Native synthetic ablations: every paired process median', y=1.04,
                          fontsize=15, fontweight='bold')
    verified = []
    legend_data = {}
    # Inspect the actual Seaborn strip artist positions/values/colors, not a separate plotted table.
    for row_number, cpus in enumerate([1, 2]):
        for col_number, fixture in enumerate(['tiff', 'math']):
            axis = graph.axes[row_number, col_number]
            axis.set_title(f'{cpus} CPU ceiling · {"TIFF" if fixture == "tiff" else "Math"}')
            collections = [c for c in axis.collections if len(c.get_offsets())]
            require(len(collections) == 16, 'wrong number of native ablation mark collections')
            for collection, (comparison, pair) in zip(collections,
                    [(comparison, pair) for comparison in COMPARISONS for pair in PAIRS], strict=True):
                expected = data[(data.cpus == cpus) & (data.fixture == fixture)
                                & (data.comparison == comparison) & (data.block == pair)]
                require(len(expected) == 1 and len(collection.get_offsets()) == 1,
                        'native ablation mark is missing/aggregated')
                x, y = collection.get_offsets()[0]
                # stripplot's 0.8 category width is divided evenly among the four hue levels.
                expected_x = COMPARISONS.index(comparison) + (pair - 1.5) * 0.2
                require(np.isclose(x, expected_x, rtol=0, atol=1e-12)
                        and np.isclose(y, expected.median_ratio.iloc[0], rtol=0, atol=1e-12),
                        'native ablation plotted mark position/value differs')
                if row_number == 0 and col_number == 0 and comparison == COMPARISONS[0]:
                    legend_data[f'{pair+1} ({ORDERS[pair]})'] = collection
                color = COLORS[pair].lstrip('#')
                expected_color = tuple(int(color[i:i+2], 16)/255 for i in [0, 2, 4])
                require(np.allclose(collection.get_facecolors()[0][:3], expected_color, rtol=0, atol=1e-12),
                        'native ablation plotted mark color differs')
                verified.append({'cpus': cpus, 'fixture': fixture, 'comparison': comparison, 'pair': pair,
                                 'ratio': float(y), 'baseline_id': expected.baseline_id.iloc[0],
                                 'candidate_id': expected.candidate_id.iloc[0]})
    require(len(verified) == 64, 'native ablation plot coverage differs')
    labels = [f'{pair+1} ({ORDERS[pair]})' for pair in PAIRS]
    graph.add_legend(legend_data=legend_data, title='Process pair', label_order=labels)
    require([text.get_text() for text in graph.legend.texts] == labels, 'ablation legend labels differ')
    for handle, color in zip(graph.legend.legend_handles, COLORS, strict=True):
        expected_color = tuple(int(color[i:i+2], 16)/255 for i in [1, 3, 5])
        require(np.allclose(handle.get_facecolors()[0][:3], expected_color, rtol=0, atol=1e-12)
                and np.all(handle.get_sizes() > 0), 'ablation legend mark is missing or miscolored')
    graph.figure.subplots_adjust(left=0.10, wspace=0.10, hspace=0.24)
    graph.figure.text(0.02, -0.025,
        '64 observed process pairs · 32 warmed recognitions per process · 3 GiB, zero swap\n'
        'Lower ratios mean lower latency. Dashed line: equal paired medians. Four descriptive pairs per cell; no intervals.\n'
        'Diagnostic binaries and two synthetic fixtures; no production/Tika improvement or default promotion established.',
        fontsize=9, color='#404040', va='top')
    FIGURES.mkdir(parents=True, exist_ok=True)
    for suffix in ['svg', 'png']:
        graph.figure.savefig(FIGURES / f'native-ablations-paired.{suffix}', dpi=180, bbox_inches='tight', facecolor='white',
                             metadata={'Date': None} if suffix == 'svg' else None)
    graph.figure.clear()
    result = {'scope': 'descriptive paired process medians, diagnostic native synthetic fixtures only',
              'verified_marks': verified, 'default_promoted': False, 'visual_review': 'pending'}
    (FIGURES / 'native-ablations-plot-validation.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


if __name__ == '__main__':
    result = render()
    print(json.dumps({'verified_marks': len(result['verified_marks']), 'visual_review': result['visual_review']}))
