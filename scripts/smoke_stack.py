#!/usr/bin/env python3
"""Assert real Tika/Indra outputs, including recursive-metadata errors."""

import argparse
import json
from pathlib import Path
import struct
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def pdf_document(objects):
    result = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for index, value in enumerate(objects, 1):
        offsets.append(len(result))
        result.extend(f"{index} 0 obj\n".encode() + value + b"\nendobj\n")
    xref = len(result)
    result.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        result.extend(f"{offset:010d} 00000 n \n".encode())
    result.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(result)


def native_pdf():
    stream = b"BT /F1 14 Tf 50 700 Td (Tika native text control 24680) Tj ET\n"
    return pdf_document([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"endstream",
    ])


def scanned_pdf(png):
    """Embed the grayscale smoke PNG losslessly, with no PDF text layer."""
    if png[:8] != b"\x89PNG\r\n\x1a\n" or png[12:16] != b"IHDR":
        raise ValueError("smoke fixture is not a PNG")
    width, height, depth, color, compression, filtering, interlace = struct.unpack(
        ">IIBBBBB", png[16:29]
    )
    if (depth, color, compression, filtering, interlace) != (8, 0, 0, 0, 0):
        raise ValueError("smoke fixture must be an 8-bit non-interlaced grayscale PNG")
    chunks = []
    offset = 8
    while offset + 12 <= len(png):
        length = struct.unpack_from(">I", png, offset)[0]
        if offset + length + 12 > len(png):
            raise ValueError("truncated PNG chunk")
        if png[offset + 4:offset + 8] == b"IDAT":
            chunks.append(png[offset + 8:offset + length + 8])
        offset += length + 12
    if not chunks:
        raise ValueError("smoke fixture has no image data")
    compressed = b"".join(chunks)
    # At the configured 300 DPI, PDFBox renders the original pixel dimensions.
    page_width = width * 72 / 300
    page_height = height * 72 / 300
    stream = f"q {page_width:g} 0 0 {page_height:g} 0 0 cm /Im0 Do Q\n".encode()
    return pdf_document([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_width:g} {page_height:g}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>".encode(),
        f"<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns {width} >> /Length {len(compressed)} >>\nstream\n".encode() + compressed + b"\nendstream",
        b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"endstream",
    ])


def request(base, endpoint, data=None, media=None, accept="text/plain"):
    headers = {"Accept": accept}
    if media:
        headers["Content-Type"] = media
    req = urllib.request.Request(base.rstrip("/") + endpoint, data=data, headers=headers, method="PUT" if data is not None else "GET")
    with urllib.request.urlopen(req, timeout=90) as response:
        if response.status != 200:
            raise RuntimeError(f"{endpoint} returned {response.status}")
        return response.read(4 * 1024 * 1024).decode("utf-8")


def check_recursive(records):
    if not isinstance(records, list) or not records:
        raise RuntimeError("recursive metadata is empty or malformed")
    for record in records:
        if not isinstance(record, dict):
            raise RuntimeError("recursive metadata record is malformed")
        for key, value in record.items():
            if key == "X-TIKA:EXCEPTION:write_limit_reached":
                if str(value).lower() == "true":
                    raise RuntimeError("recursive metadata reports truncated output")
            elif key.startswith("X-TIKA:EXCEPTION") and value:
                raise RuntimeError(f"recursive metadata records extraction failure: {key}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:9998")
    parser.add_argument("--wait", type=int, default=180)
    args = parser.parse_args()
    deadline = time.monotonic() + args.wait
    while True:
        try:
            version = request(args.url, "/version")
            if "3.3.2" not in version:
                raise RuntimeError(f"expected Tika 3.3.2, received {version!r}")
            break
        except (urllib.error.URLError, TimeoutError):
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.5)
    png = (ROOT / "backend/tests/data/smoke.png").read_bytes()
    cases = [
        ("png", png, "image/png", "INVOICE 12345"),
        ("scanned_pdf", scanned_pdf(png), "application/pdf", "INVOICE 12345"),
        ("native_pdf", native_pdf(), "application/pdf", "Tika native text control 24680"),
    ]
    report = {"tika_version": version.strip(), "cases": []}
    for name, data, media, expected in cases:
        text = request(args.url, "/tika", data, media)
        if expected not in " ".join(text.split()):
            raise RuntimeError(f"{name}: expected marker missing")
        records = json.loads(request(args.url, "/rmeta/text", data, media, "application/json"))
        check_recursive(records)
        combined = " ".join(str(record.get("X-TIKA:content", "")) for record in records)
        if expected not in " ".join(combined.split()):
            raise RuntimeError(f"{name}: recursive metadata marker missing")
        report["cases"].append({"name": name, "plain_text": "passed", "recursive_metadata": "passed"})
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
