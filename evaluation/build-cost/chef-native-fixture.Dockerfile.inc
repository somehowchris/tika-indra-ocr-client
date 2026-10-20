# Diagnostic-only tiny C archive at the same path; no network dependencies.
FROM chef-tool AS chef-native-archive-fixture
ARG NATIVE_FIXTURE_VALUE=1
RUN /bin/bash -euo pipefail -c '[[ "$NATIVE_FIXTURE_VALUE" = 1 || "$NATIVE_FIXTURE_VALUE" = 2 ]]; mkdir /native-fixture; printf "unsigned native_answer(void) { return %s; }\n" "$NATIVE_FIXTURE_VALUE" > /native-fixture/answer.c; gcc -c -O2 /native-fixture/answer.c -o /native-fixture/answer.o; ar rcsD /native-fixture/libnative_answer.a /native-fixture/answer.o'
COPY evaluation/build-cost/fixtures/chef-native-link /app/chef-fixture
WORKDIR /app/chef-fixture/app
RUN /bin/bash -euo pipefail -c 'source /native/cargo-env.sh; cargo chef prepare --recipe-path /tmp/fixture.json; cargo chef cook --release --target x86_64-unknown-linux-gnu --locked --jobs 2 --recipe-path /tmp/fixture.json; mkdir /fixture-evidence; python3 /build-tools/chef-artifact.py clean-workspace --output /fixture-evidence/dummy-cleanup.json; test ! -e target/x86_64-unknown-linux-gnu/release/chef-native-fixture'
# Restore only actual application source after every dummy artifact was removed.
COPY evaluation/build-cost/fixtures/chef-native-link/app /app/chef-fixture/app
ARG APP_FIXTURE_BONUS=0
RUN /bin/bash -euo pipefail -c '[[ "$APP_FIXTURE_BONUS" = 0 || "$APP_FIXTURE_BONUS" = 1 ]]; source /native/cargo-env.sh; sed -i "s/const BONUS: u32 = 0;/const BONUS: u32 = $APP_FIXTURE_BONUS;/" src/main.rs; touch -d @1 src/main.rs Cargo.toml Cargo.lock; cargo build --release --locked --target x86_64-unknown-linux-gnu --jobs 2; binary=target/x86_64-unknown-linux-gnu/release/chef-native-fixture; observed=$("$binary"); [[ "$observed" = "$((NATIVE_FIXTURE_VALUE+APP_FIXTURE_BONUS))" ]]; readelf -W -l "$binary" > /fixture-evidence/program-headers.txt; readelf -W -d "$binary" > /fixture-evidence/dynamic.txt; ! grep -q INTERP /fixture-evidence/program-headers.txt; ! grep -q NEEDED /fixture-evidence/dynamic.txt; sha256sum "$binary" /native-fixture/libnative_answer.a src/main.rs > /fixture-evidence/identities.sha256; printf "{\"native_value\":%s,\"source_bonus\":%s,\"observed\":%s,\"source_mtime\":1,\"native_archive_path\":\"/native-fixture/libnative_answer.a\"}\n" "$NATIVE_FIXTURE_VALUE" "$APP_FIXTURE_BONUS" "$observed" > /fixture-evidence/result.json'
COPY scripts/chef-native-fixture.py /build-tools/chef-native-fixture.py
RUN /bin/bash -euo pipefail -c 'if [[ "$NATIVE_FIXTURE_VALUE" = 1 && "$APP_FIXTURE_BONUS" = 0 ]]; then source /native/cargo-env.sh; python3 /build-tools/chef-native-fixture.py; fi'
FROM scratch AS chef-native-fixture-result
COPY --from=chef-native-archive-fixture /fixture-evidence /
