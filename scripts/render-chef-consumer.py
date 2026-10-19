#!/usr/bin/env python3
"""Render opt-in real Chef cook/export or immutable cooked-layer consumer Dockerfile.

The original source Dockerfile and original four Cargo commands stay unchanged.
"""
from __future__ import annotations
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace


def render(source,output,mode):
    helper=Path(__file__).with_name('native-sdk.py')
    spec=importlib.util.spec_from_file_location('native_sdk',helper);sdk=importlib.util.module_from_spec(spec);spec.loader.exec_module(sdk)
    with tempfile.TemporaryDirectory(prefix='chef-render-') as temporary:
        intermediate=Path(temporary)/'consumer.Dockerfile'
        sdk.render(SimpleNamespace(action='render-consumer',source=source,output=intermediate))
        original=intermediate.read_text()
    lock=json.loads(Path(__file__).with_name('cargo-chef.lock.json').read_text())
    context=source.resolve().parent.parent
    planner_optional=''.join(f'COPY {name} /plan/{name}\n' for name in ('.cargo','rust-toolchain.toml') if (context/name).exists())
    cook_optional=''.join(f'COPY {name} /app/{name}\n' for name in ('runtime/.cargo',) if (context/name).exists())
    before,after=original.split(sdk.APP_MARKER,1)
    application,after=after.split('FROM application-build AS engine-build\n',1)
    application=sdk.APP_MARKER+application
    after='FROM application-build AS engine-build\n'+after
    common=application+f'''# Pinned build-only Chef; this never enters a production runtime image.
FROM application-build AS chef-tool
RUN curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 180 --retry 2 --retry-max-time 240 '{lock['archive_url']}' --output /tmp/chef.tar.xz \\
    && printf '%s  %s\\n' '{lock['archive_sha256']}' /tmp/chef.tar.xz | sha256sum --check --status \\
    && tar -xOf /tmp/chef.tar.xz cargo-chef > /usr/local/bin/cargo-chef \\
    && chmod 0755 /usr/local/bin/cargo-chef \\
    && printf '%s  %s\\n' '{lock['binary_sha256']}' /usr/local/bin/cargo-chef | sha256sum --check --status \\
    && rm /tmp/chef.tar.xz
# Force the same already-admitted compiler rather than installing recipe components.
ENV RUSTUP_TOOLCHAIN=1.99.1-x86_64-unknown-linux-gnu
COPY scripts/chef-artifact.py scripts/render-chef-consumer.py scripts/cargo-chef.lock.json /build-tools/
FROM chef-tool AS chef-planner
WORKDIR /plan
COPY Cargo.toml Cargo.lock /plan/
COPY src /plan/src
COPY tests /plan/tests
COPY benches /plan/benches
COPY runtime /plan/runtime
COPY shared /plan/shared
COPY docker/native.Dockerfile /plan/docker/native.Dockerfile
{planner_optional}RUN mkdir /recipes \\
    && python3 /build-tools/chef-artifact.py snapshot --source /plan --output /recipes/source-inputs.json \\
    && cargo-chef chef prepare --recipe-path /recipes/root.json \\
    && cd /plan/runtime && cargo-chef chef prepare --recipe-path /recipes/runtime.json \\
    && python3 /build-tools/chef-artifact.py key --source /plan --root-recipe /recipes/root.json --runtime-recipe /recipes/runtime.json --sdk-recipe /sdk-admission/recipe.json --sdk-content /sdk-admission/content.json --source-inputs /recipes/source-inputs.json --output /recipes/key.json
'''
    cook='''FROM chef-tool AS chef-dependencies
WORKDIR /app
COPY --from=chef-planner /recipes /recipes
# Root skeleton supplies metadata targets only. Restore the owning workspace and
# real external shared crate before runtime cook; never normalize shared versions.
RUN cargo-chef chef cook --no-build --recipe-path /recipes/root.json && rm -rf /app/shared
COPY Cargo.toml Cargo.lock /app/
COPY shared /app/shared
__COOK_OPTIONAL__WORKDIR /app/runtime
RUN /bin/bash -euo pipefail -c 'source /native/cargo-env.sh; \\
    mkdir -p /chef-evidence /usr/local/cargo/registry /usr/local/cargo/git; \\
    cargo-chef chef cook --release --target x86_64-unknown-linux-gnu --package indra-engine --package indra-server --tests --examples --locked --jobs 2 --recipe-path /recipes/runtime.json --timings 2>&1 | tee /chef-evidence/cook.log; \\
    python3 /build-tools/chef-artifact.py clean-workspace --output /chef-evidence/dummy-cleanup.json; \\
    python3 /build-tools/chef-artifact.py manifest --key /recipes/key.json --output /chef-evidence/content.json'
FROM scratch AS chef-cache
COPY --from=chef-dependencies /app/runtime/target /chef/target
COPY --from=chef-dependencies /usr/local/cargo/registry /chef/registry
COPY --from=chef-dependencies /usr/local/cargo/git /chef/git
COPY --from=chef-dependencies /recipes/key.json /chef/key.json
COPY --from=chef-dependencies /chef-evidence/content.json /chef/content.json
COPY --from=chef-dependencies /chef-evidence/cook.log /chef/cook.log
COPY --from=chef-dependencies /chef-evidence/dummy-cleanup.json /chef/dummy-cleanup.json
FROM scratch AS chef-cache-metadata
COPY --from=chef-dependencies /recipes/key.json /key.json
COPY --from=chef-dependencies /chef-evidence/content.json /content.json
COPY --from=chef-dependencies /chef-evidence/cook.log /cook.log
COPY --from=chef-dependencies /chef-evidence/dummy-cleanup.json /dummy-cleanup.json
'''
    cook=cook.replace('__COOK_OPTIONAL__',cook_optional)
    restore='''FROM chef-input AS cooked-dependency-input
FROM application-build AS chef-restored
ARG CHEF_DEPENDENCY_KEY_SHA256
ARG CHEF_CONTENT_SHA256
COPY scripts/chef-artifact.py /build-tools/chef-artifact.py
COPY --from=chef-planner /recipes/key.json /chef-admission/current-key.json
COPY --from=cooked-dependency-input /chef/key.json /chef/content.json /chef-admission/
COPY --from=cooked-dependency-input /chef/target /app/runtime/target
COPY --from=cooked-dependency-input /chef/registry /usr/local/cargo/registry
COPY --from=cooked-dependency-input /chef/git /usr/local/cargo/git
RUN python3 /build-tools/chef-artifact.py verify --manifest /chef-admission/content.json --key /chef-admission/key.json --current-key /chef-admission/current-key.json --expected-key "$CHEF_DEPENDENCY_KEY_SHA256" --expected-content "$CHEF_CONTENT_SHA256"
'''
    if mode=='producer':generated=before+common+cook
    else:
        # The actual source COPY instructions restore source; only compiled/cache
        # roots are imported. No dummy source enters application stages.
        after=after.replace('FROM application-build AS engine-build','FROM chef-restored AS engine-build').replace('FROM application-build AS server-build','FROM chef-restored AS server-build')
        generated=before+common+restore+after
    with output.open('x') as stream:stream.write(generated)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--source',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--mode',choices=('producer','consumer'),required=True);args=p.parse_args();render(args.source,args.output,args.mode)


if __name__=='__main__':main()
