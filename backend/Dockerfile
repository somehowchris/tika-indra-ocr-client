# Debian builders and a matching Debian 13 / Python 3.13 distroless runtime.
FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f AS build

ARG DEBIAN_SNAPSHOT=20261017T000000Z
RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s/ trixie main\n' "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get update \
    && apt-get install --yes --no-install-recommends python3 python3-venv libstdc++6 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /venv

WORKDIR /app
COPY backend/requirements.lock /app/backend/requirements.lock
RUN /venv/bin/pip install --no-cache-dir --require-hashes --only-binary=:all: -r /app/backend/requirements.lock
COPY backend/models.lock.json backend/model_files.py backend/fetch_models.py /app/backend/
RUN /venv/bin/python /app/backend/fetch_models.py --output /models \
    && mkdir /native \
    && cp -L /usr/lib/*/libstdc++.so.6 /native/ \
    && cp -L /usr/lib/*/libgcc_s.so.1 /native/

FROM gcr.io/distroless/python3-debian13:nonroot@sha256:f47f5807043391605425f2bd162b25f50cebdc90640bacc804e0177608fe5c65
ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    HF_HUB_OFFLINE=1 \
    INDRA_MODEL_DIR=/models \
    INDRA_MODEL_MANIFEST=/app/backend/models.lock.json \
    INDRA_HOST=127.0.0.1 \
    INDRA_WORKERS=1 \
    INDRA_THREADS=1 \
    OMP_NUM_THREADS=1 \
    OMP_THREAD_LIMIT=1 \
    OPENBLAS_NUM_THREADS=1 \
    MKL_NUM_THREADS=1 \
    LD_LIBRARY_PATH=/opt/native
COPY --from=build /venv /venv
COPY --from=build /models /models
COPY --from=build /native /opt/native
COPY backend/server.py backend/model_files.py backend/models.lock.json backend/image_smoke.py /app/backend/
WORKDIR /app
USER 65532:65532
# These commands execute the actual final-image Python; no shell is introduced.
# Build fails if the Python ABI, native library closure, or either model profile fails.
RUN ["/venv/bin/python", "/app/backend/image_smoke.py"]
RUN ["/venv/bin/python", "/app/backend/image_smoke.py", "--math"]
EXPOSE 8081
ENTRYPOINT ["/venv/bin/python", "/app/backend/server.py"]
