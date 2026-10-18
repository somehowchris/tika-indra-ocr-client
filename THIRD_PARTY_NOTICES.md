# Third-party notices

The adapter, backend integration, deployment configuration and project documentation are provided under the repository's [Apache License 2.0](LICENSE). External software, model weights, source documents and model-generated text retain their own applicable terms. This notice identifies the material carried by this implementation; it is not a claim that every component or output is Apache-licensed.

## Indra code, weights and generated text

Indra OCR 0.3.2's Python code is Apache-2.0. Its wheel's license is retained in the installed environment at `indra_ocr-0.3.2.dist-info/licenses/LICENSE`.

The model is published by **Datalab**, at [datalab-to/indra-ocr](https://huggingface.co/datalab-to/indra-ocr). This project uses the immutable revision [`ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`](https://huggingface.co/datalab-to/indra-ocr/tree/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8). The weight files are unmodified and hash-verified; runtime settings select the CPU provider, batching and optional math profile. No endorsement by Datalab is implied.

The weights use **AI Pubs Open RAIL-M License (Modified)**. Its exact text is provided in [licenses/INDRA-MODEL-LICENSE.txt](licenses/INDRA-MODEL-LICENSE.txt) and in the backend image at `/models/LICENSE`. Sections 7 and 8 state attribution and share-alike conditions extending to model output. Preserve that license, model link, credit and modification information when distributing extracted prediction text or its derivatives. Benchmark prediction fields are model output; they are not relicensed as Apache adapter source merely because they appear in a JSON or text artifact. Input-document and ground-truth rights remain separate; the model license excludes the input data and complementary code from its own grant.

The exact model license also contains revenue/funding thresholds, a competing-service restriction and terms for hosted access. Consult its actual terms for the intended use; the code license does not replace them. The [pinned primary license](https://huggingface.co/datalab-to/indra-ocr/blob/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8/LICENSE) controls this attribution summary.

## Static GNU/glibc bridge

The bridge executable includes GNU C Library code through static linking. The LGPL text and glibc copyright notices are carried under `/relink/licenses/`; exact corresponding glibc sources, Debian modifications and application source/build inputs are carried under `/relink`. The collector records installed versions and checks all source archives against the matching `.dsc`.

GNU LGPL 2.1 section 6 addresses the notices and materials enabling a recipient to modify the library and relink the executable. Instructions and scope are in [docs/relinking.md](docs/relinking.md), also copied into `/relink/README.md`. Retain these materials with a distributed image or executable. Do not remove `/relink` while retaining the binary and assume the standalone Apache notice covers the embedded library.

The bundle also preserves the GCC/libgcc package copyright notice, including the GCC Runtime Library Exception, and the exact Rust toolchain's copyright/license files. The [GNU LGPL 2.1](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html) and [GCC Runtime Library Exception 3.1](https://www.gnu.org/licenses/gcc-exception-3.1.html) are primary references; the installed package/toolchain notices are the version-specific materials supplied with the build.

## Python and native dependency notices

The backend copies the complete installed virtual environment, including wheel license directories. Observed files in the verified final root include ONNX Runtime's `LICENSE` and `ThirdPartyNotices.txt`, OpenCV's `LICENSE-3RD-PARTY.txt`, NumPy's bundled license tree, Pillow's license and PDFium's `BUILD_LICENSES` inventory. These files cover bundled native components as documented by their respective distributions and are retained when the environment is copied into the final image.

The pinned Debian 13 distroless root also carries package copyright material, including `/usr/share/doc/libstdc++6/copyright` and `/usr/share/doc/libgcc-s1/copyright`. The backend preserves the required native loader filenames while copying their library bytes. The observed notice paths and hashes are recorded in `validation/backend-rootfs.json` in the source repository; that test evidence is not a required member of the runtime relinking bundle.

The Tika image contains an explicitly identified `3.3.2-indra-trace.1` overlay on Apache Tika 3.3.2. The two modified upstream sources carry modification notices; their original and patched forms, exact patches, extension sources, dependency source jars/POMs and checksums are retained under `/opt/tika/materials`. Runtime SDK dependencies are pinned in `tika-trace/dependencies.lock.tsv`; retain their embedded notices and the accompanying source/POM materials.

Tika's distribution and the Java/Python base images carry their own notices. Retain those files when repackaging. This observed-file inventory is deliberately narrower than a complete transitive license audit or a legal approval for a particular deployment.
