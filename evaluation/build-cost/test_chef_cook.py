"""Source/content contracts for real opt-in Chef cook. No compiler/Docker execution."""
import importlib.util
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


chef=module('chef_artifact',ROOT/'scripts/chef-artifact.py')
renderer=module('chef_renderer',ROOT/'scripts/render-chef-consumer.py')


class CookTests(unittest.TestCase):
    def test_real_cook_and_immutable_restore_are_separate_and_opt_in(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d)
            producer=root/'producer';consumer=root/'consumer'
            renderer.render(ROOT/'docker/native.Dockerfile',producer,'producer')
            renderer.render(ROOT/'docker/native.Dockerfile',consumer,'consumer')
            text=producer.read_text();restored=consumer.read_text()
            self.assertIn('cargo-chef chef cook --release --target x86_64-unknown-linux-gnu',text)
            self.assertIn('--package indra-engine --package indra-server --tests --examples --locked --jobs 2',text)
            self.assertIn('source /native/cargo-env.sh',text)
            self.assertIn('COPY shared /app/shared',text)
            self.assertIn('FROM scratch AS chef-cache',text)
            self.assertIn('FROM chef-input AS cooked-dependency-input',restored)
            self.assertIn('--expected-key "$CHEF_DEPENDENCY_KEY_SHA256" --expected-content "$CHEF_CONTENT_SHA256"',restored)
            for stage in ('engine-build','server-build'):
                original=(ROOT/'docker/native.Dockerfile').read_text().split('FROM native-build AS '+stage+'\n',1)[1].split('\nFROM ',1)[0]
                actual=restored.split('FROM chef-restored AS '+stage+'\n',1)[1].split('\nFROM ',1)[0]
                self.assertEqual(original,actual)
            self.assertNotIn('RUN taskset --cpu-list "$BUILD_CPUSET" bash scripts/build-native-runtime.sh /native',restored)
            self.assertNotIn('type=cache',text+restored)

    def test_workspace_lock_feature_profile_sdk_and_external_source_invalidate_key(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);source=root/'source';source.mkdir();tools=root/'tools';tools.mkdir()
            files=['Cargo.toml','Cargo.lock','runtime/Cargo.toml','runtime/Cargo.lock','runtime/rust-toolchain.toml','runtime/.cargo/config.toml','runtime/crates/example/Cargo.toml','shared/example/src/lib.rs']
            for name in files:
                path=source/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(name)
            for name in ('chef-artifact.py','render-chef-consumer.py','cargo-chef.lock.json'):(tools/name).write_text(name)
            inputs=[]
            for name in ('root-recipe','runtime-recipe','sdk-recipe','sdk-content'):
                path=root/name;path.write_text(name);inputs.append(path)
            def key():return chef.key(source,*inputs,tools)['dependency_key_sha256']
            baseline=key()
            for path in [source/name for name in files]+inputs+[tools/'cargo-chef.lock.json']:
                before=path.read_text();path.write_text(before+' changed')
                self.assertNotEqual(key(),baseline,str(path));path.write_text(before)
            runtime_source=source/'runtime/crates/example/src/lib.rs';runtime_source.parent.mkdir();runtime_source.write_text('application body')
            self.assertEqual(key(),baseline,'runtime application body alone should not alter dependency key')
            # SDK content identity, not the filesystem pathname, invalidates a cook.
            original=inputs[-1].read_text();inputs[-1].write_text('same /native/lib.a path; different archive hash')
            self.assertNotEqual(key(),baseline);inputs[-1].write_text(original)

    def test_changed_restored_bytes_modes_key_and_content_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);roots={name:root/name for name in ('target','registry','git')}
            for path in roots.values():path.mkdir()
            artifact=roots['target']/'libnative.rlib';artifact.write_bytes(b'compiled bytes')
            key=root/'key';current=root/'current';manifest=root/'manifest'
            identity='a'*64;value={'schema_version':1,'dependency_key_sha256':identity,'inputs':{'sdk_content':'b'*64}}
            key.write_text(json.dumps(value));current.write_bytes(key.read_bytes())
            manifest.write_text(json.dumps({'schema_version':1,'dependency_key_sha256':identity,'entries':chef.inventory(roots)}))
            expected=hashlib.sha256(manifest.read_bytes()).hexdigest()
            chef.verify(manifest,key,identity,expected,current,roots)
            artifact.write_bytes(b'changed at identical path')
            with self.assertRaises(ValueError):chef.verify(manifest,key,identity,expected,current,roots)
            artifact.write_bytes(b'compiled bytes');artifact.chmod(0o700)
            with self.assertRaises(ValueError):chef.verify(manifest,key,identity,expected,current,roots)
            with self.assertRaises(ValueError):chef.verify(manifest,key,'c'*64,expected,current,roots)
            with self.assertRaises(ValueError):chef.verify(manifest,key,identity,'d'*64,current,roots)
            artifact.unlink();artifact.symlink_to('/etc/passwd')
            with self.assertRaises(ValueError):chef.inventory(roots)


if __name__=='__main__':unittest.main()
