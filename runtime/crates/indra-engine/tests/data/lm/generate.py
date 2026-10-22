"""Generate LM parity fixtures from the unmodified indra-ocr 1.1.2 package."""
import json, math, os, random, sys
for key in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ.setdefault(key, "1")
import numpy as np
import cv2
sys.path.insert(0, sys.argv[1])
from indra_ocr import lm as lmmod, pipeline
models_dir, out_dir = sys.argv[2], sys.argv[3]
images = sys.argv[4:]
random.seed(20261020)
# 1. logp fixtures: sampled real keys, perturbed keys and random keys per model.
for name in ["lm.npz", "lm_cjk.npz", "lm_world.npz"]:
    m = lmmod.NgramLM(os.path.join(models_dir, name))
    keys = []
    bits = m.bits
    hmask = (1 << (bits * (m.order - 1))) - 1
    for n in range(m.order):
        pool = m.k[n]
        for _ in range(400):
            k = int(pool[random.randrange(len(pool))])
            keys.append(k)                                    # seen pair
            keys.append((k & ~((1 << bits) - 1)) | random.randrange(1, m.vocab_size + 1))  # same history, random token
    for _ in range(600):
        keys.append(random.getrandbits(bits * m.order) & ((hmask << bits) | ((1 << bits) - 1)))
    vals = m.logp_keys(keys)
    json.dump({"model": name, "order": m.order, "bits": bits, "vocab_size": m.vocab_size,
               "max_logp": m.max_logp, "keys": [hex(k) for k in keys], "values": vals},
              open(os.path.join(out_dir, f"lm-logp-{name.replace('.npz','')}.json"), "w"))
    print(name, "order", m.order, "bits", bits, "keys", len(keys), "max_logp", m.max_logp)
# 2. beam fixtures: hook beam_search inside the real pipeline on real pages.
calls = []
real_beam = lmmod.beam_search
def hooked(blank, topv, topi, lm, alpha=0.5, beta=1.5, beam=8, blank_skip=0.999, cand_floor=1e-3):
    ids, steps = real_beam(blank, topv, topi, lm, alpha, beta, beam, blank_skip, cand_floor)
    name = [k for k, v in names.items() if v is lm][0]
    calls.append({"model": name, "alpha": alpha, "beta": beta, "beam": beam, "blank_skip": blank_skip,
                  "cand_floor": cand_floor, "blank": [float(v) for v in blank],
                  "topv": [[float(v) for v in row] for row in topv], "topi": [[int(v) for v in row] for row in topi],
                  "ids": ids, "steps": steps})
    return ids, steps
lmmod.beam_search = hooked
cv2.setNumThreads(1)
engine = pipeline.OrtOCR(models_dir, threads=1, rec_int8=False, adaptive=True, table_heads=False, math=True, providers=["CPUExecutionProvider"])
names = {}
for sid, (model, a, b, g) in engine.lms.items():
    for label, sess in [("base", engine.rec)] + [(k, s) for k, (s, _) in engine.groups.items()]:
        if id(sess) == sid:
            names[{"base": "lm.npz"}.get(label, f"lm_{label}.npz")] = model
texts = {}
for path in images:
    pages = []
    for gray in pipeline.read_images(path):
        before = len(calls)
        page = engine(gray)
        pages.append(page.text)
        for c in calls[before:]:
            c["image"] = os.path.basename(path)
    texts[os.path.basename(path)] = "\n\f\n".join(pages)
json.dump(calls, open(os.path.join(out_dir, "lm-beam-cases.json"), "w"))
json.dump(texts, open(os.path.join(out_dir, "reference-texts-math.json"), "w"), ensure_ascii=False, indent=1)
print("beam calls", len(calls), {k: sum(1 for c in calls if c["model"] == k) for k in names})
for k, v in texts.items():
    print("TEXT", k, repr(v[:200]))
