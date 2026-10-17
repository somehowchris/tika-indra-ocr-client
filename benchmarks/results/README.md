# Benchmark evidence and output attribution

The measured tables are in [benchmark-results.md](../../research/benchmark-results.md).
[index.json](index.json) lists the accepted experiments; [evidence-sha256.json](evidence-sha256.json)
hashes the retained files. Original rejected runs and the explicitly documented
thread-ablation startup recovery remain available for audit.

## Indra model output terms

The actual OCR predictions in the `indra*` and `tika-indra*` result directories
were produced with Datalab's **Indra OCR** models from
[datalab-to/indra-ocr, revision ed345c7f158eddbc7dee0c6a7269afb75a94f7e8](https://huggingface.co/datalab-to/indra-ocr/tree/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8).
Credit for those models belongs to Datalab and the model's contributors.
The model weights were used unchanged; the HTTP service, Tika adapter, and
benchmark instrumentation are separate code changes retained in this project.

The model's [AI Pubs Open RAIL-M License (Modified)](../../licenses/INDRA-MODEL-LICENSE.txt)
applies to these model-produced predictions, including its output attribution
and share-alike terms in sections 7 and 8. The complete pinned license is
provided at that link. Preserve this attribution and license when sharing the
predictions or derivatives covered by those terms. This attribution identifies
the model's origin and does not imply endorsement.

The Apache-2.0 license of the implementation code is separate from the model
and output license. The synthetic source fixtures and reference transcripts
were created for these tests. Public regression evidence here contains numeric
observations, content hashes, and configuration metadata; public scans,
reference article transcripts, and predicted article text are not included.

The frozen backend source is the version actually measured. Current production
recovery/health changes were validated separately. The native experiments use
shared process affinity, and do not establish Docker/Kubernetes resource-limit
or deployment-performance results.
