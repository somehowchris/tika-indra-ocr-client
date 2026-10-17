#!/usr/bin/env python3
"""Materialize path-specific raw or Tika experiments without editing defaults."""

import argparse
import hashlib
import json
import shutil
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def write_json(path, data):
    path.write_text(json.dumps(data, indent=2) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--track", choices=("raw", "tika", "native-auto"), required=True)
    parser.add_argument("--budget", type=int, choices=(1, 2), required=True)
    parser.add_argument("--indra-python", type=Path, required=True)
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--backend-script", type=Path)
    parser.add_argument("--tessdata-best", type=Path, required=True)
    parser.add_argument("--tessdata-distro", type=Path, default=Path("/usr/share/tesseract-ocr/5/tessdata"))
    parser.add_argument("--tika-baseline", choices=("best", "distro"), default="best")
    parser.add_argument("--bridge", type=Path)
    parser.add_argument("--jar", type=Path)
    parser.add_argument("--java", type=Path, default=Path("/usr/bin/java"))
    parser.add_argument("--dpi", type=int, choices=(200, 300), default=300)
    parser.add_argument("--workers", type=int, choices=(1, 2), default=1)
    parser.add_argument("--threads", type=int, choices=(1, 2), default=1)
    parser.add_argument("--tesseract-threads", type=int, choices=(1, 2), default=1)
    parser.add_argument("--batch", type=int, default=64)
    parser.add_argument("--math", action="store_true")
    parser.add_argument("--backend-port", type=int, default=18081)
    parser.add_argument("--tika-port", type=int, default=19998)
    args = parser.parse_args()
    if args.tesseract_threads > args.budget:
        parser.error("Tesseract threads must fit the experiment budget")
    if args.workers * args.threads > args.budget:
        parser.error("Indra workers * intra-op threads must fit the experiment budget")
    if not 1 <= args.batch <= 64:
        parser.error("batch must be 1..64")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[1]
    env = {"OMP_THREAD_LIMIT": str(args.tesseract_threads), "OMP_NUM_THREADS": str(args.tesseract_threads), "OPENBLAS_NUM_THREADS": "1",
           "MKL_NUM_THREADS": "1", "NUMEXPR_NUM_THREADS": "1"}
    backend_env = env | {"INDRA_MODEL_DIR": str(args.models.resolve()), "INDRA_HOST": "127.0.0.1",
        "INDRA_PORT": str(args.backend_port), "INDRA_WORKERS": str(args.workers), "INDRA_THREADS": str(args.threads),
        "INDRA_RECOGNITION_BATCH": str(args.batch), "INDRA_BATCH_METRICS": "1", "INDRA_MATH": "1" if args.math else "0",
        "INDRA_QUEUE_CAPACITY": "2", "INDRA_QUEUE_TIMEOUT_SECONDS": "30", "INDRA_REQUEST_TIMEOUT_SECONDS": "120"}
    backend = {"name": "indra", "command": [str(args.indra_python.absolute()), str(args.backend_script.absolute() if args.backend_script else root / "backend/server.py")],
               "env": backend_env, "readiness_url": f"http://127.0.0.1:{args.backend_port}/readyz"}
    engines = []
    images = ["image/png", "image/jpeg", "image/tiff"]
    if args.track == "raw":
        for name, model_dir in (("tesseract-distro", args.tessdata_distro), ("tesseract-best", args.tessdata_best)):
            engines.append({"name": name, "mode": "command", "media_types": images, "env": env,
                            "command": [shutil.which("tesseract") or "tesseract", "{input}", "stdout",
                                        "--tessdata-dir", str(model_dir.resolve()), "-l", "eng", "--oem", "1", "--psm", "3",
                                        "-c", "page_separator="]})
        engines.append({"name": "indra", "mode": "http", "media_types": images, "method": "POST",
                        "url": f"http://127.0.0.1:{args.backend_port}/ocr", "server_command": backend["command"],
                        "env": backend_env, "readiness_url": backend["readiness_url"]})
    else:
        if not args.bridge or not args.jar:
            parser.error("Tika tracks require --bridge and --jar")
        bridge_dir = args.output / "bridge"
        bridge_dir.mkdir()
        (bridge_dir / "tesseract").symlink_to(args.bridge.resolve())
        baseline_name = "tika-tesseract-best" if args.tika_baseline == "best" else "tika-tesseract-stock"
        baseline_directory = args.tessdata_best if args.tika_baseline == "best" else args.tessdata_distro
        for name in (baseline_name, "tika-indra"):
            is_indra = name == "tika-indra"
            config = ET.parse(root / "deploy/tika-benchmark.xml")
            server = config.getroot().find("server")
            server.find("host").text = "127.0.0.1"
            server.find("port").text = str(args.tika_port)
            server.find("javaPath").text = str(args.java.resolve())
            for arg in server.find("forkedJvmArgs"):
                if arg.text.startswith("-XX:ActiveProcessorCount="):
                    arg.text = f"-XX:ActiveProcessorCount={args.budget}"
            for param in config.iter("param"):
                setting = param.attrib.get("name")
                replacements = {"ocrDPI": str(args.dpi), "tesseractPath": str(bridge_dir if is_indra else Path(shutil.which("tesseract")).parent) + "/",
                                "language": "" if is_indra else "eng", "ocrStrategy": "auto" if args.track == "native-auto" else "ocr_only"}
                if setting in replacements:
                    param.text = replacements[setting]
            if not is_indra:
                for element in config.iter("parser"):
                    if element.attrib.get("class", "").endswith("TesseractOCRParser"):
                        ET.SubElement(element.find("params"), "param", name="tessdataPath", type="string").text = str(baseline_directory.resolve())
            config_path = args.output / f"{name}.xml"
            config.write(config_path, encoding="utf-8", xml_declaration=True)
            java = {"name": "tika", "command": [str(args.java.resolve()), "-Xms64m", "-Xmx256m",
                    f"-XX:ActiveProcessorCount={args.budget}", "-XX:+UseSerialGC", "-Djava.awt.headless=true",
                    "-jar", str(args.jar.resolve()), "-c", str(config_path)],
                    "env": env | {"INDRA_ADDR": f"127.0.0.1:{args.backend_port}"},
                    "readiness_url": f"http://127.0.0.1:{args.tika_port}/version"}
            stack_path = args.output / f"{name}-stack.json"
            write_json(stack_path, {"services": [backend, java] if is_indra else [java]})
            engines.append({"name": name, "mode": "http", "url": f"http://127.0.0.1:{args.tika_port}/tika",
                            "method": "PUT", "readiness_url": java["readiness_url"], "ready_timeout_seconds": 180,
                            "server_command": [sys.executable, str(root / "benchmarks/stack.py"), str(stack_path)],
                            "assert_no_indra_ocr": is_indra and args.track == "native-auto"})
    inputs = {"indra_model_manifest": str(root / "backend/models.lock.json"),
              "tesseract_best_eng_sha256": hashlib.sha256((args.tessdata_best / "eng.traineddata").read_bytes()).hexdigest(),
              "tesseract_distro_eng_sha256": hashlib.sha256((args.tessdata_distro / "eng.traineddata").read_bytes()).hexdigest()}
    if args.bridge:
        inputs["bridge_sha256"] = hashlib.sha256(args.bridge.read_bytes()).hexdigest()
    if args.jar:
        inputs["tika_jar_sha256"] = hashlib.sha256(args.jar.read_bytes()).hexdigest()
    output = args.output / "plan.json"
    write_json(output, {"schema_version": 1, "track": args.track, "dpi": args.dpi,
                        "tika_baseline": args.tika_baseline, "inputs": inputs, "engines": engines})
    print(output)


if __name__ == "__main__":
    main()
