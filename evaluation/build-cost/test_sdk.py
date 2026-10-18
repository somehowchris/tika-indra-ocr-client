"""Synthetic closure/OCI negatives; these do not attest a built native SDK."""
import contextlib
import gzip
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
from types import SimpleNamespace
import unittest

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('sdk',ROOT/'scripts/native-sdk.py')
sdk=importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk)


def encoded(value):
    return json.dumps(value,sort_keys=True).encode()


class SdkTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base=Path(self.temp.name).resolve()

    def fixture(self, platform='amd64', wrong_source=False, unsafe=False, changed_content=False, bad_diffid=False):
        source=self.base/'source-sdk'
        source.mkdir()
        root=source/'rootfs'
        for name in ('native/lib.a','native/opencv/include/header.h','native/evidence/config','native/cargo-env.sh','glibc-source/source.tar.xz'):
            path=root/name
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes(b'explicit synthetic SDK fixture; not a compiled SDK')
        for name in sdk.notice_paths(ROOT/'scripts/collect-runtime-materials.sh'):
            path=root/'native'/name
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_text('synthetic notice fixture')
        recipe={'source':sdk.source_recipe(ROOT),'platform':'linux/amd64','fixture_scope':'synthetic transport test only; not consumer-toolchain admission'}
        if wrong_source: recipe['source']['native_docker_prefix_sha256']='0'*64
        sdk.write(source/'recipe.json',recipe)
        sdk.write(source/'content.json',{'schema_version':1,'roots':['glibc-source','native'],'entries':sdk.entries(root)})
        sdk.write(source/'producer.json',{'recipe_sha256':sdk.sha(source/'recipe.json'),'content_sha256':sdk.sha(source/'content.json')})
        if changed_content: (root/'native/opencv/include/header.h').write_text('mutated after inventory')
        buffer=io.BytesIO()
        with tarfile.open(fileobj=buffer,mode='w') as archive:
            archive.add(source,arcname='sdk')
            if unsafe:
                item=tarfile.TarInfo('../escape')
                item.size=1
                archive.addfile(item,io.BytesIO(b'x'))
        raw=buffer.getvalue()
        layer=gzip.compress(raw,mtime=0)
        layout=self.base/'layout'
        blobs=layout/'blobs/sha256'
        blobs.mkdir(parents=True)
        def blob(data,media):
            digest=hashlib.sha256(data).hexdigest()
            (blobs/digest).write_bytes(data)
            return {'mediaType':media,'size':len(data),'digest':'sha256:'+digest}
        layer_desc=blob(layer,'application/vnd.oci.image.layer.v1.tar+gzip')
        config=blob(encoded({'os':'linux','architecture':platform,'rootfs':{'type':'layers','diff_ids':['sha256:'+('0'*64 if bad_diffid else hashlib.sha256(raw).hexdigest())]}}),'application/vnd.oci.image.config.v1+json')
        manifest=blob(encoded({'schemaVersion':2,'config':config,'layers':[layer_desc]}),sdk.OCI_MANIFEST)
        sdk.write(layout/'index.json',{'schemaVersion':2,'manifests':[manifest]})
        sdk.write(layout/'oci-layout',{'imageLayoutVersion':'1.0.0'})
        return layout,manifest['digest'],layer_desc

    def test_existing_notice_source_exact31_and_no_duplication(self):
        names=sdk.notice_paths(ROOT/'scripts/collect-runtime-materials.sh')
        self.assertEqual(len(names),31)
        self.assertIn('src/opencv/LICENSE',names)
        self.assertIn('build/opencv/3rdparty/ippicv/ippicv_lnx/EULA.txt',names)

    def test_generated_producer_and_consumer_keep_explicit_boundaries(self):
        source=ROOT/'docker/native.Dockerfile'
        producer=self.base/'producer.Dockerfile'
        consumer=self.base/'consumer.Dockerfile'
        sdk.render(SimpleNamespace(source=source,output=producer,action='render-producer'))
        sdk.render(SimpleNamespace(source=source,output=consumer,action='render-consumer'))
        self.assertTrue(producer.read_text().startswith(sdk.prefix(source)))
        text=consumer.read_text()
        self.assertIn('FROM sdk-input AS native-sdk-input',text)
        self.assertNotIn('RUN taskset --cpu-list "$BUILD_CPUSET" bash scripts/build-native-runtime.sh /native',text)
        self.assertIn('ARG NATIVE_SDK_RECIPE_SHA256',text)
        self.assertEqual(text.count('FROM native-tools AS native-build'),1)
        for body in __import__('re').findall(r"RUN /bin/bash -euo pipefail -c '(.*?)'\n",text,__import__('re').S):
            self.assertEqual(subprocess.run(['bash','-n'],input=body.replace('\\\n',' '),text=True,capture_output=True).returncode,0)

    def test_actual_all_blob_bytes_and_sdk_files_materialize(self):
        layout,digest,_=self.fixture()
        report=sdk.verify_oci(layout,digest)
        self.assertEqual(len(report['verified_blobs']),3)
        output=self.base/'materialized'
        with contextlib.redirect_stdout(io.StringIO()):
            sdk.materialize(SimpleNamespace(layout=layout,digest=digest,output=output,inputs=ROOT))
        receipt=sdk.read(output/'materialization.json')
        self.assertEqual(receipt['requested_digest'],digest)
        self.assertGreater(receipt['materialized_files'],31)
        self.assertEqual(sdk.entries(output/'sdk/rootfs'),sdk.read(output/'sdk/content.json')['entries'])

    def test_missing_corrupt_and_truncated_blob_rejected(self):
        layout,digest,layer=self.fixture()
        path=layout/'blobs/sha256'/layer['digest'].split(':')[1]
        original=path.read_bytes()
        for mutation in (original[:-1],b'x'*len(original)):
            path.write_bytes(mutation)
            with self.assertRaises(ValueError):sdk.verify_oci(layout,digest)
        path.unlink()
        with self.assertRaises(ValueError):sdk.verify_oci(layout,digest)

    def test_requested_empty_index_cannot_authorize_sibling_image(self):
        layout,digest,_=self.fixture()
        data=encoded({'schemaVersion':2,'manifests':[]})
        empty_digest=hashlib.sha256(data).hexdigest()
        (layout/'blobs/sha256'/empty_digest).write_bytes(data)
        index=sdk.read(layout/'index.json')
        index['manifests'].append({'mediaType':sdk.OCI_INDEX,'size':len(data),'digest':'sha256:'+empty_digest})
        (layout/'index.json').write_bytes(encoded(index))
        with self.assertRaisesRegex(ValueError,'uniquely authorize'):
            sdk.verify_oci(layout,'sha256:'+empty_digest)
        self.assertEqual(sdk.verify_oci(layout,digest)['platform_manifest_digest'],digest)

    def test_requested_parent_index_authorizes_its_actual_descendant(self):
        layout,digest,_=self.fixture()
        index=sdk.read(layout/'index.json')
        data=encoded(index)
        parent=hashlib.sha256(data).hexdigest()
        (layout/'blobs/sha256'/parent).write_bytes(data)
        (layout/'index.json').write_bytes(encoded({'schemaVersion':2,'manifests':[{'mediaType':sdk.OCI_INDEX,'size':len(data),'digest':'sha256:'+parent}]}))
        self.assertEqual(sdk.verify_oci(layout,'sha256:'+parent)['platform_manifest_digest'],digest)

    def test_wrong_platform_rejected(self):
        layout,digest,_=self.fixture(platform='arm64')
        with self.assertRaisesRegex(ValueError,'platform'):sdk.verify_oci(layout,digest)

    def test_mutable_or_wrong_digest_rejected(self):
        layout,digest,layer=self.fixture()
        for invalid in ('latest','sha256:'+'0'*64,'sha256:'+('A'*64),layer['digest']):
            with self.assertRaises(ValueError):sdk.verify_oci(layout,invalid)

    def test_correct_oci_wrong_recipe_never_admitted(self):
        layout,digest,_=self.fixture(wrong_source=True)
        with self.assertRaisesRegex(ValueError,'recipe inputs differ'):
            sdk.materialize(SimpleNamespace(layout=layout,digest=digest,output=self.base/'failed',inputs=ROOT))
        self.assertFalse((self.base/'failed/materialization.json').exists())

    def test_unsafe_layer_path_rejected(self):
        layout,digest,_=self.fixture(unsafe=True)
        with self.assertRaisesRegex(ValueError,'unsafe/non-SDK'):
            sdk.materialize(SimpleNamespace(layout=layout,digest=digest,output=self.base/'unsafe',inputs=ROOT))
        self.assertFalse((self.base/'escape').exists())

    def test_valid_oci_with_mutated_sdk_content_is_rejected(self):
        layout,digest,_=self.fixture(changed_content=True)
        with self.assertRaisesRegex(ValueError,'file/mode/hash inventory differs'):
            sdk.materialize(SimpleNamespace(layout=layout,digest=digest,output=self.base/'changed-content',inputs=ROOT))

    def test_valid_compressed_blob_with_wrong_diffid_is_rejected(self):
        layout,digest,_=self.fixture(bad_diffid=True)
        with self.assertRaisesRegex(ValueError,'diff-id differs'):
            sdk.materialize(SimpleNamespace(layout=layout,digest=digest,output=self.base/'wrong-diffid',inputs=ROOT))

    def test_inventory_binds_modes_and_rejects_symlinks(self):
        root=self.base/'inventory';root.mkdir()
        file=root/'header';file.write_text('header')
        original=sdk.entries(root)
        file.chmod(0o700)
        self.assertNotEqual(original,sdk.entries(root))
        (root/'symlink').symlink_to(file)
        with self.assertRaises(ValueError):sdk.entries(root)


if __name__=='__main__':unittest.main()
