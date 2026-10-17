import random
import unittest

from benchmarks.metrics import aggregate, distance, normalize, paired_cer_interval, score


def simple_distance(a, b):
    rows = list(range(len(b) + 1))
    for i, left in enumerate(a, 1):
        current = [i]
        for j, right in enumerate(b, 1):
            current.append(min(rows[j] + 1, current[j - 1] + 1, rows[j - 1] + (left != right)))
        rows = current
    return rows[-1]


class MetricTests(unittest.TestCase):
    def test_unicode_canonical_equivalence_but_not_case_or_spelling(self):
        self.assertEqual(score("café", "cafe\u0301")["cer"], 0)
        self.assertGreater(score("Straße", "Strasse")["cer"], 0)
        self.assertGreater(score("TOTAL", "total")["wer"], 0)
        self.assertGreater(score("CHF 12.50", "CHF 1250")["wer"], 0)

    def test_explicit_whitespace_policy(self):
        text = "  alpha\r\n beta\t\f\n"
        self.assertEqual(normalize(text), "alpha beta")
        self.assertEqual(normalize(text, "preserve"), "  alpha\n beta\t\f\n")
        with self.assertRaises(ValueError):
            normalize(text, "mystery")

    def test_insertions_can_exceed_one(self):
        self.assertEqual(score("a", "aaaaaaaa")["cer"], 7)

    def test_empty_reference_is_not_reported_as_perfect(self):
        result = score("", "invented text")
        self.assertIsNone(result["cer"])
        self.assertTrue(result["blank_false_positive"])
        self.assertEqual(result["character_errors"], 13)

    def test_micro_average_uses_reference_units(self):
        result = aggregate([score("x" * 100, "x" * 100), score("a", "b")])
        self.assertAlmostEqual(result["micro_cer"], 1 / 101)

    def test_reading_order_is_not_bag_of_words(self):
        self.assertGreater(score("left column then right column", "right column then left column")["wer"], 0)

    def test_bit_parallel_matches_independent_dynamic_programming(self):
        rng = random.Random(17)
        alphabet = "abcé中"
        for _ in range(500):
            a = "".join(rng.choices(alphabet, k=rng.randrange(18)))
            b = "".join(rng.choices(alphabet, k=rng.randrange(18)))
            self.assertEqual(distance(a, b), simple_distance(a, b), (a, b))
        self.assertEqual(distance(["one", "two"], ["one", "three"]), 1)

    def test_bootstrap_clusters_renditions_of_same_document(self):
        baseline, candidate = [], []
        for case_id, document_id in (("page1", "doc-a"), ("page1-noisy", "doc-a"), ("page2", "doc-b")):
            baseline.append(dict(score("abc", "xbc"), case_id=case_id, document_id=document_id))
            candidate.append(dict(score("abc", "abc"), case_id=case_id, document_id=document_id))
        result = paired_cer_interval(baseline, candidate, seed=5, resamples=100)
        self.assertEqual(result["paired_cases"], 3)
        self.assertEqual(result["document_clusters"], 2)
        self.assertAlmostEqual(result["difference"], -1 / 3)
        self.assertAlmostEqual(result["low"], -1 / 3)


if __name__ == "__main__":
    unittest.main()
