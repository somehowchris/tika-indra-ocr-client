"""Adversarial preparation only; none of these fixtures are OCR measurements."""
import copy
import itertools
import json
import shutil
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import replay as r


def matrix_distance(a, b):
    # Deliberately retain the full matrix, unlike Myers and the production row DP.
    cells = [[0] * (len(b) + 1) for _ in range(len(a) + 1)]
    for i in range(len(a) + 1):
        cells[i][0] = i
    for j in range(len(b) + 1):
        cells[0][j] = j
    for i, x in enumerate(a, 1):
        for j, y in enumerate(b, 1):
            cells[i][j] = min(cells[i - 1][j] + 1, cells[i][j - 1] + 1,
                              cells[i - 1][j - 1] + (x != y))
    return cells[-1][-1]


def case(cid="a", cohort="primary", cluster="book"):
    return dict(id=cid, original_page_id=cid, document_cluster=cluster, cohort=cohort,
                language="deu", script="Latn", domain="legal", appearance="print",
                reference_quality="audited", task="full_page", word_tokenizer="whitespace",
                english={"status": "absent", "reference_letters": 0}, input={"sha256": "a" * 64},
                reference={"sha256": r.sha(b"a"), "bytes": 1, "path": cid + ".txt"})


class RunFixture:
    def __init__(self, root, *, repeats=3):
        self.root = root
        self.cases = [case(), case("b", "challenge", "other")]
        self.study = {"manifest": {"cases": self.cases, "corpus_id": "synthetic", "selection_seed": 3},
                      "manifest_sha256": "b" * 64, "manifest_bytes": 123,
                      "protocol": {"execution": {"repetitions": repeats},
                                   "metrics": {"bootstrap_seed": 17, "bootstrap_resamples": 0}},
                      "texts": {"a": "a", "b": "a"}}
        self.run = dict(sequence=1, cohort="main", cpus=1, variant=r.VARIANTS[0], status="completed",
                        reason=None, raw_run="raw", driver_exit=0, score_exit=0)
        self.records = [dict(kind="environment", schema_version=1, corpus_id="synthetic",
                             manifest_sha256="b" * 64, manifest_bytes=123, selection_seed=3,
                             variant=self.run["variant"], repetitions=repeats,
                             budget=dict(cpus=1, quota_us=100000, period_us=100000, memory_limit_bytes=3*1024**3))]
        (root / "raw").mkdir()
        self.add(self.cases[0], 0, "a", kind="warmup")
        self.records.append(dict(kind="repetition_end", repetition=0, attempted_pages=1, wall_us=10))

    def add(self, c, rep, body, status=200, error=None, kind="attempt"):
        body = body.encode() if isinstance(body, str) else body
        name = f"{c['id']}-r{rep:02}.txt"
        (self.root / "raw" / name).write_bytes(body)
        self.records.append(dict(kind=kind, variant=self.run["variant"], cpus=1,
                                 **{k: c[k] for k in r.META if k != "original_page_id"},
                                 case_id=c["id"], repetition=rep, sequence=self.cases.index(c),
                                 elapsed_us=10 + rep, status=status, error=error,
                                 output_path=name, output_bytes=len(body), output_sha256=r.sha(body)))

    def finish(self, tail=b""):
        budget = self.records[0]["budget"]
        self.records.append(dict(kind="resource_end", budget=budget))
        return self.write(tail)

    def write(self, tail=b""):
        data = b"".join(json.dumps(x).encode() + b"\n" for x in self.records) + tail
        (self.root / "raw" / "requests.jsonl").write_bytes(data)
        return r.Files(self.root)

    def full(self):
        for rep in (1, 2, 3):
            self.add(self.cases[0], rep, "a")
            self.add(self.cases[1], rep, "ab")
            self.records.append(dict(kind="repetition_end", repetition=rep, attempted_pages=2, wall_us=100))
        return self.finish()


class Algorithms(unittest.TestCase):
    def test_exhaustive_bit_vectors_against_full_matrix(self):
        # 14,641 pairs, including empty, insert/delete/substitute and repeats.
        words = ["".join(x) for n in range(5) for x in itertools.product("aé中", repeat=n)]
        for a in words:
            for b in words:
                self.assertEqual(r.distance(a, b), matrix_distance(a, b), (a, b))

    def test_token_sequence_bit_vectors_and_long_bit_boundaries(self):
        for n in (0, 1, 31, 32, 63, 64, 65, 127, 128, 129):
            a = tuple(str(i % 7) for i in range(n))
            b = a[:n//2] + ("insert",) + a[n//2:]
            self.assertEqual(r.distance(a, b), matrix_distance(a, b))
            self.assertEqual(r.distance(b, a), 1)

    def test_nfc_scalars_not_compatibility_or_graphemes(self):
        for a, b in (("a\u0308", "ä"), ("가", "가"), ("q\u0301\u0327", "q\u0327\u0301")):
            self.assertEqual(r.measure(a, b, "whitespace")["cer"]["errors"], 0)
        for a, b in (("👩\u200d🔬", "👩🔬"), ("مَ", "م"), ("①", "1"), ("Α", "A"), ("x\u200by", "xy")):
            self.assertEqual(r.measure(a, b, "whitespace")["cer"]["errors"], 1)

    def test_rust_whitespace_exactly_excludes_python_extra_controls(self):
        self.assertEqual(len(r.WHITE_SPACE), 25)
        for char in r.WHITE_SPACE:
            self.assertEqual(r.collapse(char + "a" + char * 2 + "b" + char), "a b")
        for value in range(0x1c, 0x20):
            text = "a" + chr(value) + "b"
            self.assertEqual(r.collapse(text), text)
            self.assertEqual(r.tokens(text), [text])
            self.assertTrue(chr(value).isspace())  # Catch accidentally using Python semantics.

    def test_empty_reference_over_100_percent_and_punctuation(self):
        self.assertEqual(r.metric("", "abc"), dict(reference_units=0, hypothesis_units=3, errors=3))
        self.assertEqual(r.measure("a", "aaaa", "whitespace")["cer"]["errors"], 3)
        self.assertEqual(r.measure("a-b", "ab", "whitespace")["cer"]["errors"], 1)
        self.assertIsNone(r.measure("中文", "中文", "none")["wer"])

    def test_metric_limits_are_after_nfc_and_never_repair_http_status(self):
        self.assertIsNotNone(r.measure("a" * 32768, "", "whitespace"))
        self.assertIsNone(r.measure("a" * 32769, "", "whitespace"))
        self.assertIsNotNone(r.measure("a\u0308" * 32768, "", "whitespace"))
        # Strict metric rejects before secondary whitespace collapse can shrink it.
        self.assertIsNone(r.measure(" " * 32769, "", "whitespace"))
        self.assertIsNotNone(r.measure("a " * 16384, "", "whitespace"))

    def test_type7_latency_known_values_and_undefined_empty(self):
        self.assertEqual(r.distribution([])["p95_us"], None)
        values = [30, 0, 20, 10]
        self.assertEqual(r.quantile(values, 50), 15.0)
        self.assertEqual(r.quantile(values, 95), 28.5)
        self.assertAlmostEqual(r.quantile(values, 99), 29.7)
        self.assertEqual(r.distribution([17])["p99_us"], 17.0)

    def test_splitmix_known_external_vectors_and_rejection_branch(self):
        rng = r.SplitMix(0)
        self.assertEqual([rng.next() for _ in range(3)],
                         [0xe220a8397b1dcdaf, 0x6e789e6aa1b965f4, 0x06c45d188009454f])
        rng = r.SplitMix(9)
        with patch.object(rng, "next", side_effect=[0, 4]) as mocked:
            self.assertEqual(rng.sample(3), 1)  # 0 rejected: 2**64 % 3 == 1.
            self.assertEqual(mocked.call_count, 2)

    def test_stratum_seed_has_length_and_encoding_boundaries(self):
        self.assertNotEqual(r.stratum_seed(1, "ab", "c", ""), r.stratum_seed(1, "a", "bc", ""))
        self.assertNotEqual(r.stratum_seed(1, "a", "x", "ä"), r.stratum_seed(2, "a", "x", "ä"))
        self.assertEqual(r.stratum_seed(1, "a", "x", "ä"), r.stratum_seed(1, "a", "x", "ä"))

    def test_analytic_whole_cluster_distribution_not_page_bootstrap(self):
        result = r.estimate([(2, 0, 0, 2), (1, 0, 1, 1)], 20261017, 1000)
        self.assertEqual(result["paired_cases"], 3)
        self.assertEqual(result["document_clusters"], 2)
        self.assertEqual(result["right_minus_left"], 1/3)
        self.assertEqual((result["ci_95_low"], result["ci_95_high"]), (0.0, 1.0))
        self.assertEqual(result["bootstrap_samples"], 1000)

    def test_undefined_cluster_draws_never_get_conditioned_interval(self):
        value = r.estimate([(0, 2, 3, 1), (1, 0, 0, 1)], 1, 1000)
        self.assertGreater(value["undefined_bootstrap_samples"], 0)
        self.assertIsNone(value["ci_95_low"])
        self.assertIsNone(value["ci_95_high"])
        for clusters, n in (([(1, 0, 1, 1)], 1000), ([(1, 0, 1, 1), (2, 0, 0, 1)], 0), ([(0, 1, 2, 1)], 1000)):
            value = r.estimate(clusters, 3, n)
            self.assertEqual(value["bootstrap_samples"], 0)
            self.assertIsNone(value["ci_95_low"])


class Journal(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.fixture = RunFixture(Path(self.temp.name))

    def test_complete_journal_and_performance_arithmetic(self):
        f = self.fixture
        score, attempts, ends = r.rescore(f.full(), f.run, f.study)
        self.assertTrue(score["journal_complete"])
        self.assertEqual(score["cases"][0]["metrics"]["cer"]["errors"], 0)
        self.assertEqual(score["cases"][1]["metrics"]["cer"]["errors"], 1)
        p = r.performance(score["cases"], attempts, ends, True, 3)
        self.assertEqual(p["successful_attempts"], 6)
        self.assertEqual(p["successful_case_responses_per_complete_workload_wall_second"], 20000.0)
        self.assertEqual(p["cohorts"][0]["successful_complete_body_latency"]["p95_us"], 12.9)

    def test_first_failed_binary_body_is_retained_later_success_never_repairs_it(self):
        f = self.fixture
        f.add(f.cases[0], 1, b"\xff", status=500)
        f.add(f.cases[0], 2, "a")
        f.add(f.cases[0], 3, "ab")
        score, attempts, ends = r.rescore(f.write(), f.run, f.study)
        first = score["cases"][0]
        self.assertEqual(first["outcome"], "service_failure")
        self.assertIsNone(first["metrics"])
        self.assertEqual(first["first_error"], "non_success_http")
        self.assertEqual(first["successful_outputs_differing_from_first_success"], 1)
        self.assertTrue(first["mixed_success_and_failure"])
        self.assertEqual(score["cases"][1]["outcome"], "missing_attempt")
        self.assertIsNone(r.performance(score["cases"], attempts, ends, False, 3)["successful_case_responses_per_complete_workload_wall_second"])

    def test_successful_invalid_utf8_is_hard_failure(self):
        f = self.fixture
        f.add(f.cases[0], 1, b"\xff", status=200)
        with self.assertRaises(UnicodeDecodeError):
            r.rescore(f.write(), f.run, f.study)

    def test_metric_limit_does_not_become_http_failure(self):
        f = self.fixture
        f.add(f.cases[0], 1, "a" * 32769)
        score, attempts, ends = r.rescore(f.write(), f.run, f.study)
        self.assertEqual(score["cases"][0]["outcome"], "metric_limit")
        self.assertEqual(score["cases"][0]["first_error"], "metric_codepoint_limit")
        self.assertEqual(r.performance(score["cases"], attempts, ends, False, 3)["successful_attempts"], 1)

    def test_truncated_last_record_missing_cases_and_raw_hash_mismatch(self):
        f = self.fixture
        f.add(f.cases[0], 1, "a")
        score, _, _ = r.rescore(f.write(b'{"kind":"attempt"'), f.run, f.study)
        self.assertFalse(score["journal_complete"])
        self.assertEqual(score["truncated_final_record_bytes"], 17)
        self.assertEqual(score["cases"][1]["outcome"], "missing_attempt")
        (f.root / "raw" / "a-r01.txt").write_text("x")
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            r.rescore(f.write(), f.run, f.study)

    def test_duplicate_attempt_bad_order_end_and_records_after_resource(self):
        f = self.fixture
        f.add(f.cases[0], 1, "a")
        row = copy.deepcopy(f.records[-1])
        f.records.append(row)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            r.rescore(f.write(), f.run, f.study)
        f.records.pop()
        f.records.append(dict(kind="repetition_end", repetition=1, attempted_pages=2, wall_us=1))
        with self.assertRaisesRegex(ValueError, "end disagrees"):
            r.rescore(f.write(), f.run, f.study)
        f.records.pop()
        f.finish()
        f.records.append(row)
        with self.assertRaisesRegex(ValueError, "records after"):
            r.rescore(f.write(), f.run, f.study)

    def test_dynamic_budget_counters_can_change_but_limits_cannot(self):
        f = self.fixture
        f.full()
        f.records[0]["budget"]["cpu_stat"] = "usage_usec 100\n"
        f.records[-1]["budget"] = dict(f.records[-1]["budget"], cpu_stat="usage_usec 200\n")
        self.assertTrue(r.rescore(f.write(), f.run, f.study)[0]["journal_complete"])
        f.records[-1]["budget"]["quota_us"] += 1
        with self.assertRaisesRegex(ValueError, "budget changed"):
            r.rescore(f.write(), f.run, f.study)

    def test_unterminated_tail_after_resource_end_is_rejected(self):
        f = self.fixture
        f.full()
        with self.assertRaisesRegex(ValueError, "bytes after resource_end"):
            r.rescore(f.write(b"junk"), f.run, f.study)

    def test_explicit_empty_error_label_is_still_a_failure(self):
        f = self.fixture
        f.add(f.cases[0], 1, b"\xff", status=200, error="")
        value = r.rescore(f.write(), f.run, f.study)[0]["cases"][0]
        self.assertEqual(value["outcome"], "service_failure")
        self.assertEqual(value["first_error"], "")

    def test_metric_failure_missing_strata_and_empty_reference_macro(self):
        f = self.fixture
        f.add(f.cases[0], 1, "ab")
        score, _, _ = r.rescore(f.write(), f.run, f.study)
        all_primary = score["summaries"][0]
        self.assertEqual(all_primary["cer"]["micro_rate"], 1)
        challenge = next(s for s in score["summaries"] if s["cohort"] == "challenge" and s["axis"] == "overall")
        self.assertEqual(challenge["missing_first_attempts"], 1)
        self.assertEqual(r.aggregate([r.metric("", "abc"), r.metric("a", "ab")]),
                         dict(cases=2, reference_units=1, hypothesis_units=5, errors=4,
                              micro_rate=4.0, macro_case_rate=1.0, empty_reference_cases=1))

    def test_consumed_path_symlink_escape_duplicate_json_and_optimized_checks(self):
        root = self.fixture.root
        (root / "link").symlink_to(root / "raw")
        files = r.Files(root)
        for name in ("../x", "/tmp/x", "raw//a", "raw/./a", "link/a"):
            with self.assertRaises(ValueError):
                files.read(name)
        with self.assertRaises(ValueError):
            r.decode(b'{"a":1,"a":2}')
        with self.assertRaises(ValueError):
            r.decode(b'{"a":NaN}')
        checks = r.Checks()
        checks.equal({"x": None, "n": 1}, {"x": 0.0, "n": True}, "mutated")
        self.assertEqual(len(checks.differences), 2)


class AssignmentReplay(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = "f" * 40
        self.index = {"source_sha": self.source, "assigned_cpu_budget": None,
                      "runs": [dict(cohort=c, cpus=cpu, variant=v, sequence=i + 1,
                                    status="failed" if i == 0 else "planned", reason="synthetic retained status",
                                    scores=None, raw_run=None, driver_exit=9 if i == 0 else None,
                                    score_exit=None) for i, (c, cpu, v) in enumerate(itertools.product(("main", "laboratory"), (1, 2), r.VARIANTS))],
                      "comparisons": [dict(cohort=c, cpus=cpu, left=a, right=b, status="skipped", reason="synthetic unavailable",
                                           exit_code=None, path=None, sha256=None)
                                      for c in ("main", "laboratory") for cpu in (1, 2)
                                      for a, b in itertools.combinations(r.VARIANTS, 2)]}
        self.studies = {}
        for cohort in ("main", "laboratory"):
            folder = self.root / cohort
            folder.mkdir()
            f = RunFixture(folder)
            f.full()
            self.studies[cohort] = f.study

    def save(self):
        (self.root / "study-index.json").write_text(json.dumps(self.index))
        return r.Files(self.root)

    def test_all_32_cells_and_112_missing_pair_statuses_remain_explicit(self):
        result = r.replay(self.save(), self.studies, self.source, r.Checks())
        self.assertEqual(len(result["cells"]), 32)
        self.assertEqual(len(result["comparisons"]), 112)
        self.assertEqual(result["cells"][0]["status"], "failed")
        self.assertEqual(result["cells"][0]["driver_exit"], 9)
        self.assertEqual({p["status"] for p in result["comparisons"]}, {"skipped"})
        self.assertEqual(result["bootstrap_cluster_draws_requested"], 0)
        for mutate in (lambda: self.index["runs"].pop(), lambda: self.index["comparisons"].pop()):
            before = copy.deepcopy(self.index)
            mutate()
            with self.assertRaisesRegex(ValueError, "assignment differs"):
                r.replay(self.save(), self.studies, self.source, r.Checks())
            self.index = before

    def test_hard_failure_still_retains_every_assigned_status(self):
        self.index["runs"][0].update(scores="missing-score.json", sha256="0"*64)
        progress = {}
        with self.assertRaisesRegex(ValueError, "missing/unsafe"):
            r.replay(self.save(), self.studies, self.source, r.Checks(), progress)
        self.assertEqual(len(progress["cells"]), 32)
        self.assertEqual(len(progress["comparisons"]), 112)
        self.assertEqual(progress["cells"][0]["status"], "failed")

    def test_available_score_is_reconstructed_and_summary_corruption_retained(self):
        run = self.index["runs"][0]
        run.update(raw_run="main/raw", scores="main/scores.json", performance="main/performance.json")
        # Fully specified analytical fixture: two distinct cohorts, one a→a and
        # one a→ab, exactly three successful attempts per case and 300 us wall.
        cases = []
        for cid, cohort, cluster, error, units in (("a", "primary", "book", 0, 1), ("b", "challenge", "other", 1, 2)):
            frozen = next(c for c in self.studies["main"]["manifest"]["cases"] if c["id"] == cid)
            body = b"a" if cid == "a" else b"ab"
            cases.append({**{k: frozen[k] for k in r.META}, "case_id": cid, "writer_cluster": None,
                          "word_tokenizer": "whitespace", "input_sha256": "a"*64, "reference_sha256": r.sha(b"a"),
                          "outcome": "success", "first_http_status": 200, "first_error": "",
                          "first_output_sha256": r.sha(body), "first_output_bytes": units, "first_elapsed_us": 11,
                          "attempts": 3, "successful_attempts": 3, "failed_attempts": 0, "missing_attempts": 0,
                          "successful_outputs_differing_from_first_success": 0, "mixed_success_and_failure": False,
                          "metrics": {"cer": {"reference_units": 1, "hypothesis_units": units, "errors": error},
                                      "cer_whitespace_collapsed": {"reference_units": 1, "hypothesis_units": units, "errors": error},
                                      "wer": {"reference_units": 1, "hypothesis_units": 1, "errors": error}}})
        summaries = []
        for case in cases:
            for axis in r.AXES:
                summary = dict(cohort=case["cohort"], axis=axis, level="all" if axis == "overall" else case[axis],
                               selected_cases=1, scored_cases=1, service_failures=0, metric_limits=0,
                               missing_first_attempts=0, document_clusters=1, scored_document_clusters=1)
                for metric, d in case["metrics"].items():
                    summary[metric] = dict(cases=1, **d, micro_rate=float(d["errors"]),
                                           macro_case_rate=float(d["errors"]), empty_reference_cases=0)
                summaries.append(summary)
        journal = (self.root / "main/raw/requests.jsonl").read_bytes()
        score = dict(cases=cases, summaries=summaries, journal_sha256=r.sha(journal), journal_bytes=len(journal),
                     journal_complete=True, truncated_final_record_bytes=0, warmup_attempts=1,
                     nondeterministic_successful_output_cases=0, mixed_success_failure_cases=0,
                     manifest_sha256="b"*64, manifest_bytes=123, variant=r.VARIANTS[0], cpus=1,
                     selected_accuracy_repetition=1, expected_repetitions=3,
                     corpus_id="synthetic", metric_policy="nfc-unicode-scalar-unit-cost-levenshtein-v1", nfc_unicode_version=[16,0,0], rust_unicode_version=[17,0,0])
        score_path = self.root / "main/scores.json"
        score_path.write_text(json.dumps(score))
        run["sha256"] = r.sha(score_path.read_bytes())
        # This test covers score arithmetic; timing is separately analytical above.
        run["performance"] = None
        checks = r.Checks()
        result = r.replay(self.save(), self.studies, self.source, checks)
        self.assertFalse(checks.differences)
        self.assertEqual(result["cells"][0]["numeric_replay"], "checked")
        self.assertEqual(result["cells"][0]["status"], "failed")  # Numeric match never promotes it.
        for field in ("reference_units", "errors", "macro_case_rate"):
            original = copy.deepcopy(score)
            score["summaries"][0]["cer"][field] += 1
            score_path.write_text(json.dumps(score))
            run["sha256"] = r.sha(score_path.read_bytes())  # Rehashed mutation, not a trivial integrity failure.
            checks = r.Checks()
            r.replay(self.save(), self.studies, self.source, checks)
            self.assertEqual(len(checks.differences), 1)
            self.assertIn(field, checks.differences[0]["field"])
            score = original
        score_path.write_text(json.dumps(score))
        run["sha256"] = r.sha(score_path.read_bytes())
        right = self.index["runs"][1]
        right.update(raw_run="right/raw", scores="right/scores.json", performance=None)
        shutil.copytree(self.root / "main/raw", self.root / "right/raw")
        right_journal = self.root / "right/raw/requests.jsonl"
        records = [json.loads(line) for line in right_journal.read_bytes().splitlines()]
        for record in records:
            if "variant" in record:
                record["variant"] = right["variant"]
        right_journal.write_bytes(b"".join(json.dumps(record).encode() + b"\n" for record in records))
        right_score = dict(score, variant=right["variant"], journal_sha256=r.sha(right_journal.read_bytes()),
                           journal_bytes=right_journal.stat().st_size)
        right_path = self.root / right["scores"]
        right_path.write_text(json.dumps(right_score))
        right["sha256"] = r.sha(right_path.read_bytes())
        pair = self.index["comparisons"][0]
        pair.update(path="pair.json", status="completed", exit_code=0)
        paired_strata = []
        for summary in summaries:
            header = {k: summary[k] for k in ("cohort", "axis", "level")}
            row = dict(header, seed=r.stratum_seed(17, **header), selected_cases=1, joint_success_cases=1,
                       left_service_failures=0, right_service_failures=0, left_unscoreable=0, right_unscoreable=0)
            for metric in r.METRICS:
                error = summary[metric]["errors"]
                row[metric] = dict(paired_cases=1, document_clusters=1, reference_units=1,
                                   left_errors=error, right_errors=error, left_rate=float(error), right_rate=float(error),
                                   right_minus_left=0.0, ci_95_low=None, ci_95_high=None,
                                   bootstrap_samples=0, undefined_bootstrap_samples=0)
            paired_strata.append(row)
        report = dict(bootstrap_seed=17, requested_bootstrap_samples=0, manifest_sha256="b"*64,
                      left_variant=run["variant"], right_variant=right["variant"], left_cpus=1, right_cpus=1,
                      left_scores_sha256=run["sha256"], right_scores_sha256=right["sha256"],
                      left_scores_bytes=score_path.stat().st_size, right_scores_bytes=right_path.stat().st_size,
                      strata=paired_strata)
        pair_path = self.root / "pair.json"
        pair_path.write_text(json.dumps(report))
        pair["sha256"] = r.sha(pair_path.read_bytes())
        checks = r.Checks()
        result = r.replay(self.save(), self.studies, self.source, checks)
        self.assertFalse(checks.differences)
        self.assertEqual(result["comparisons"][0]["numeric_replay"], "checked")
        report["strata"][0]["cer"]["ci_95_high"] = 0.0
        pair_path.write_text(json.dumps(report))
        pair["sha256"] = r.sha(pair_path.read_bytes())
        checks = r.Checks()
        r.replay(self.save(), self.studies, self.source, checks)
        self.assertEqual(len(checks.differences), 1)
        self.assertIn("ci_95_high", checks.differences[0]["field"])

    def test_paired_metrics_group_clusters_and_retain_failures(self):
        def scored(cid, cluster, error, *, outcome="success", eligible=True):
            c = case(cid, "primary", cluster)
            c["case_id"] = c.pop("id")
            c["outcome"] = outcome
            c["metrics"] = {m: dict(reference_units=2, hypothesis_units=2, errors=error)
                            if m != "wer" or eligible else None for m in r.METRICS} if outcome == "success" else None
            return c
        left = [scored("a", "z", 0), scored("b", "z", 0), scored("c", "a", 0, eligible=False),
                scored("d", "lost", 0, outcome="service_failure")]
        right = [scored("a", "z", 1), scored("b", "z", 1), scored("c", "a", 0, eligible=False),
                 scored("d", "lost", 0, outcome="metric_limit")]
        values, draws = r.compare(left, right, 17, 1000)
        overall = values[0]
        self.assertEqual(overall["selected_cases"], 4)
        self.assertEqual(overall["joint_success_cases"], 3)
        self.assertEqual(overall["left_service_failures"], 1)
        self.assertEqual(overall["right_unscoreable"], 1)
        self.assertEqual(overall["cer"]["document_clusters"], 2)
        self.assertEqual(overall["cer"]["paired_cases"], 3)
        self.assertEqual(overall["cer"]["right_minus_left"], 1/3)
        self.assertEqual(overall["wer"]["paired_cases"], 2)
        self.assertEqual(overall["wer"]["document_clusters"], 1)
        self.assertIsNone(overall["wer"]["ci_95_low"])
        self.assertEqual(draws, 6 * 2 * 2 * 1000)  # Six identical axes, two eligible CER metrics.


class ReferenceDenominators(unittest.TestCase):
    def test_nfc_letter_categories_and_unknown_english_are_counted_separately(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base = root / "evaluation"
            base.mkdir()
            prepared = root / "prepared"
            prepared.mkdir()
            cases = []
            for index in range(11):
                c = case(str(index), cluster=str(index))
                body = "A\u0301 ①² \u0345".encode()
                (prepared / c["reference"]["path"]).write_bytes(body)
                c["reference"].update(sha256=r.sha(body), bytes=len(body))
                if index == 0:
                    c["english"] = {"status": "unknown", "reference_letters": 0}
                cases.append(c)
            manifest = dict(cases=cases, expected_primary_pages=11)
            data = json.dumps(manifest).encode()
            (base / "corpus-frozen.json").write_bytes(data)
            protocol = dict(corpus_sha256=r.sha(data), primary_variants=[{"id": v} for v in r.VARIANTS])
            (base / "protocol.json").write_text(json.dumps(protocol))
            result = r.references(r.Files(root), "main", prepared)["denominators"]
            self.assertEqual(result["reference_letters"], 11)  # Marks, spaces and numbers are not Letters.
            self.assertEqual(result["english_reference_letters"], 1)  # Unknown counts all NFC Letters.
            self.assertEqual(result["english_pages_conservative"], 1)
            self.assertEqual(result["english_clusters_conservative"], 1)
            self.assertEqual(result["per_case_nfc_letters"], {str(i): 1 for i in range(11)})
            cases[0]["english"] = {"status": "present", "reference_letters": 2}
            data = json.dumps(manifest).encode()
            (base / "corpus-frozen.json").write_bytes(data)
            protocol["corpus_sha256"] = r.sha(data)
            (base / "protocol.json").write_text(json.dumps(protocol))
            with self.assertRaisesRegex(ValueError, "English count exceeds"):
                r.references(r.Files(root), "main", prepared)


if __name__ == "__main__":
    unittest.main()
