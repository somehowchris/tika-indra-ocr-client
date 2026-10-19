"""Cheap adversarial controller checks; actual signed SDK negatives require CI."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


controller=module('sdk_invalidation_control_test',ROOT/'scripts/verify-sdk-invalidation.py')
fixtures=module('sdk_invalidation_oci_fixture',ROOT/'evaluation/build-cost/test_sdk.py')


class SdkInvalidationTests(unittest.TestCase):
    def test_each_independent_blob_copy_fails_real_reader_and_preserves_original(self):
        fixture=fixtures.SdkTests();fixture.setUp();self.addCleanup(fixture.temp.cleanup)
        layout,digest,_=fixture.fixture();before=controller.audit.inventory(layout)
        for case in ('missing','truncated','corrupt'):
            copied=fixture.base/('copy-'+case);controller.shutil.copytree(layout,copied)
            mutation=controller.mutate_blob(copied,digest,case)
            self.assertNotEqual(mutation['before'],mutation['after'])
            if case=='corrupt':self.assertEqual(mutation['before']['bytes'],mutation['after']['bytes'])
            if case=='truncated':self.assertEqual(mutation['before']['bytes']-1,mutation['after']['bytes'])
            with self.subTest(case=case),self.assertRaises(ValueError):controller.sdk_api.verify_oci(copied,digest)
            self.assertEqual(controller.audit.inventory(layout),before)
            controller.sdk_api.verify_oci(layout,digest)

    def test_timeout_signal_success_and_unrelated_error_never_count_as_rejection(self):
        diagnostic='SDK admission manifest identity differs'
        controller.require_rejection(1,diagnostic,(diagnostic,))
        for status,body in ((0,diagnostic),(124,diagnostic),(-9,diagnostic),(1,'permission denied'),(1,'')):
            with self.subTest(status=status,body=body),self.assertRaisesRegex(ValueError,'typed SDK admission failure absent'):
                controller.require_rejection(status,body,(diagnostic,))

    def test_each_recipe_mutation_precedes_the_original_verifier(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'consumer.Dockerfile'
            controller.sdk_api.render(type('Arguments',(),{'source':ROOT/'docker/native.Dockerfile','output':path,'action':'render-consumer'})())
            original=path.read_text();marker='RUN python3 /sdk-input/scripts/native-sdk.py verify '
            for case,needle in (('compiler','cp -L "$(command -v gcc)"'),('source','>> /sdk-input/scripts/build-native-runtime.sh'),('closure','>> /native/cargo-env.sh')):
                changed=controller.diagnostic_recipe(original,case)
                self.assertEqual(changed.count(marker),1)
                self.assertLess(changed.index(needle),changed.index(marker))
                self.assertEqual(changed[changed.index(marker):],original[original.index(marker):])
            compiler=controller.diagnostic_recipe(original,'compiler')
            self.assertIn("printf '\\000' >> /sdk-negative-compiler/gcc",compiler)
            self.assertIn('ENV PATH=/sdk-negative-compiler:${PATH}',compiler)
            platform=controller.diagnostic_recipe(original,'platform')
            self.assertEqual(platform.replace('--platform "linux/arm64"','--platform "$TARGETPLATFORM"'),original)
            self.assertNotIn('RUN taskset --cpu-list "$BUILD_CPUSET" bash scripts/build-native-runtime.sh /native',original)
            with self.assertRaises(ValueError):controller.diagnostic_recipe(original+original,'compiler')
            with self.assertRaises(ValueError):controller.diagnostic_recipe(original,'unknown')


if __name__=='__main__':unittest.main()
