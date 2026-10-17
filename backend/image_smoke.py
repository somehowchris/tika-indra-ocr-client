"""Execute in the final image: verify native libraries, locked models and OCR."""

from collections import Counter
import argparse
import json
import os

from model_files import verify_models
from server import Config, create_engine, process_image


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--math", action="store_true", help="also load and exercise the optional math profile")
    if parser.parse_args().math:
        os.environ["INDRA_MATH"] = "1"
    import cv2
    import numpy as np
    import onnxruntime
    import pypdfium2  # validates the bundled PDFium loader too

    config = Config.from_env()
    verify_models(config.model_dir, config.manifest, config.math)
    counts = Counter()
    engine = create_engine(config, counts)
    image = np.full((220, 1000), 255, np.uint8)
    cv2.putText(image, "INVOICE 12345", (30, 110), cv2.FONT_HERSHEY_SIMPLEX, 2, 0, 3)
    ok, encoded = cv2.imencode(".png", image)
    if not ok:
        raise RuntimeError("PNG encoder unavailable")
    text, _ = process_image(encoded.tobytes(), config, engine, counts)
    if b"12345" not in text:
        raise RuntimeError("actual OCR smoke check failed")
    print(json.dumps({"native_imports": "passed", "ocr": "passed", "ort": onnxruntime.__version__, "math": config.math}))


if __name__ == "__main__":
    main()
