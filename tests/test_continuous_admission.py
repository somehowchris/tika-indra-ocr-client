"""Arithmetic boundaries and malformed evidence rejection; no OCR execution."""
from fractions import Fraction
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    'continuous_admission', Path(__file__).resolve().parents[1] / 'scripts/admit-continuous-pilot.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Admission(unittest.TestCase):
    def test_exact_thresholds_and_single_memory_regression(self):
        ratios = {'throughput': [Fraction(105, 100)] * 10,
                  'p95': [Fraction(105, 100)] * 10,
                  'cpu_per_success': [Fraction(105, 100)] * 10,
                  'memory_peak': [Fraction(125, 100)] * 10}
        self.assertTrue(all(MODULE.screen(ratios, 2).values()))
        ratios['memory_peak'][9] += Fraction(1, 10**9)
        self.assertFalse(MODULE.screen(ratios, 2)['every_pair_memory_peak'])
        ratios['memory_peak'][9] -= Fraction(1, 10**9)
        ratios['throughput'][0] -= Fraction(1, 100)
        ratios['throughput'][1] -= Fraction(1, 100)
        ratios['throughput'][2] -= Fraction(1, 100)
        ratios['throughput'][3] -= Fraction(1, 100)
        ratios['throughput'][4] -= Fraction(1, 100)
        self.assertFalse(MODULE.screen(ratios, 2)['throughput'])
        self.assertTrue(MODULE.screen(ratios, 1)['throughput'])

    def test_median_uses_paired_ratios_not_ratio_of_unpaired_medians(self):
        ratios = [Fraction(a, b) for a, b in [(1, 10), (10, 1)] * 5]
        self.assertEqual(MODULE.describe(ratios)['median'], 5.05)
        with self.assertRaisesRegex(ValueError, 'ten paired'):
            MODULE.describe(ratios[:-1])

    def test_duplicate_and_malformed_counters_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'cpu.stat'
            for bad in ['usage_usec 2\nusage_usec 3\n', 'usage_usec NaN\n', 'usage_usec -1\n']:
                path.write_text(bad)
                with self.assertRaisesRegex(ValueError, 'counter'):
                    MODULE.counters(path)

    def test_manifest_rejects_unlisted_file_and_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = root / 'input'; file.write_bytes(b'actual evidence')
            manifest = root / 'SHA256SUMS'
            manifest.write_text(f'{MODULE.sha(file)}  ./input\n')
            self.assertEqual(MODULE.manifest(root, 'native'), 1)
            extra = root / 'unlisted'; extra.write_bytes(b'not inventoried')
            with self.assertRaisesRegex(ValueError, 'completeness'):
                MODULE.manifest(root, 'native')
            extra.unlink(); extra.symlink_to('/missing/private/path')
            with self.assertRaisesRegex(ValueError, 'alias'):
                MODULE.manifest(root, 'native')


if __name__ == '__main__':
    unittest.main()
