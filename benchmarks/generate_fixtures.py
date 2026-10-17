#!/usr/bin/env python3
"""Generate deterministic smoke fixtures; these are NOT a representative corpus."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import random
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont, ImageOps
from reportlab.lib.utils import ImageReader
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas


BODY = [
    "Quarterly operations report",
    "Reference: OCR-2026-0417",
    "A useful system preserves the text people actually wrote.",
    "Small errors in dates, amounts and identifiers matter.",
    "Invoice total: CHF 1,249.50; tax rate: 8.1%.",
    "Delivery date: 17 October 2026. Order number: AB-40921.",
    "Contact: research@example.org | Extension: +41 44 123 45 67",
    "Terms: paid within 30 days; no automatic renewal.",
    "This paragraph uses UPPERCASE, lowercase and MixedCase.",
    "Distinguish 0 from O, 1 from l, and 5 from S.",
    "The quick brown fox jumps over the lazy dog.",
    "Check punctuation: (brackets), [square], + plus, / slash.",
    "Measurements: 12.5 mm; 2,048 samples; revision 3.3.2.",
    "Page reading order must remain stable across columns.",
    "End of report. Retain this line in the extracted text.",
]


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--fonts", type=Path, default=Path("/usr/share/fonts/truetype/dejavu"))
    parser.add_argument("--seed", type=int, default=20261017)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    rng = random.Random(args.seed)
    cases = []
    font_hashes = {}

    def font(name: str, size: int):
        path = args.fonts / name
        font_hashes[name] = sha(path)
        return ImageFont.truetype(str(path), size)

    def make_page(lines: list[str], dpi=200, font_name="DejaVuSans.ttf", point_size=11,
                  background=255, ink=12):
        width, height = round(8.27 * dpi), round(11.69 * dpi)
        image = Image.new("L", (width, height), background)
        draw = ImageDraw.Draw(image)
        regular = font(font_name, round(point_size * dpi / 72))
        title = font("DejaVuSans-Bold.ttf", round(16 * dpi / 72))
        x, y, spacing = round(.58 * dpi), round(.65 * dpi), round(point_size * dpi / 72 * 1.6)
        for index, line in enumerate(lines):
            draw.text((x, y), line, font=title if index == 0 else regular, fill=ink)
            y += round(spacing * 1.7) if index == 0 else spacing
        return image

    def save_case(name: str, image: Image.Image, lines: list[str], kind: str, dpi=200,
                  document_id: str | None = None, extension="png", **save_options):
        path = args.output / f"{name}.{extension}"
        image.save(path, dpi=(dpi, dpi), **save_options)
        truth = args.output / f"{name}.txt"
        truth.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        cases.append({"id": name, "document_id": document_id or name, "input": path.name,
                      "truth": truth.name, "input_sha256": sha(path), "truth_sha256": sha(truth),
                      "media_type": "image/jpeg" if extension == "jpg" else "image/png", "pages": 1,
                      "kind": kind, "dpi": dpi, "synthetic": True})

    clean = make_page(BODY)
    save_case("clean_sans_200", clean, BODY, "clean_print", document_id="operations_report")
    save_case("clean_serif_200", make_page(BODY, font_name="DejaVuSerif.ttf"), BODY,
              "serif_print", document_id="operations_report")
    save_case("small_print_300", make_page(BODY, dpi=300, point_size=8), BODY,
              "small_print", dpi=300, document_id="operations_report")
    save_case("low_contrast_200", make_page(BODY, background=233, ink=160), BODY,
              "low_contrast", document_id="operations_report")
    noise = Image.new("L", clean.size)
    noise.putdata([rng.randrange(0, 24) for _ in range(clean.width * clean.height)])
    # Reproducible bounded additive noise, using a documented blend.
    noisy = Image.blend(clean, noise, .12).filter(ImageFilter.GaussianBlur(.4))
    save_case("noisy_200", noisy, BODY, "noise_blur", document_id="operations_report")
    save_case("skew_200", clean.rotate(2.0, resample=Image.Resampling.BICUBIC, fillcolor=255), BODY,
              "skew_2_degrees", document_id="operations_report")
    save_case("jpeg_200", clean, BODY, "jpeg_compression", document_id="operations_report",
              extension="jpg", quality=40, optimize=False)
    accents = [
        "Multilingual Latin character check",
        "Zürich: Strasse, Grösse, heissen, aussergewöhnlich.",
        "Français: élève, déjà, façade, cœur, numéro 42.",
        "Español: información, año, niño, acción y atención.",
        "Prices: EUR 19.95 / CHF 21.50 / GBP 17.00.",
        "Symbols: naïve café; Ångström; crème brûlée.",
        "This is a challenge slice; the baseline language is eng.",
    ]
    save_case("latin_accents_200", make_page(accents), accents, "multilingual_latin_challenge")
    receipt = ["RECEIPT 2026-1048", "DATE       2026-10-17", "ITEM             QTY      PRICE",
               "Notebook           2       9.80", "Pencil             4       2.40",
               "Folder             1       3.50", "TOTAL CHF                 15.70",
               "CARD **** 1842", "Thank you for your visit."]
    save_case("receipt_200", make_page(receipt, font_name="DejaVuSansMono.ttf"), receipt, "receipt_table")
    columns = Image.new("L", clean.size, 255)
    draw = ImageDraw.Draw(columns)
    col_font = font("DejaVuSans.ttf", 29)
    left = ["FIRST COLUMN", "One paragraph begins here.", "Read the next line below.",
            "Documents have a logical order.", "This is the left column end."]
    right = ["SECOND COLUMN", "Continue with this paragraph.", "Now read the right-hand side.",
             "Do not interleave both columns.", "This is the final sentence."]
    for x, lines in ((110, left), (860, right)):
        for i, line in enumerate(lines):
            draw.text((x, 170 + i * 65), line, font=col_font, fill=10)
    save_case("two_columns_200", columns, left + right, "two_column_reading_order")
    save_case("blank_200", Image.new("L", clean.size, 255), [], "blank")
    save_case("inverted_200", ImageOps.invert(clean), BODY, "inverted_print", document_id="operations_report")

    # Image-only PDFs preserve the same physical page at two render DPI settings.
    for name, dpi, source in (("scan_pdf_200", 200, clean),
                              ("scan_pdf_300", 300, make_page(BODY, dpi=300))):
        target = args.output / f"{name}.pdf"
        pdf = canvas.Canvas(str(target), pagesize=(595.44, 841.68), invariant=1, pageCompression=1)
        pdf.drawImage(ImageReader(source), 0, 0, width=595.44, height=841.68)
        pdf.showPage()
        pdf.save()
        truth = args.output / f"{name}.txt"
        truth.write_text("\n".join(BODY) + "\n", encoding="utf-8")
        cases.append({"id": name, "document_id": "operations_report", "input": target.name,
                      "truth": truth.name, "input_sha256": sha(target), "truth_sha256": sha(truth),
                      "media_type": "application/pdf", "pages": 1, "kind": "image_only_pdf",
                      "source_dpi": dpi, "synthetic": True})

    # Native-text negative control: AUTO should bypass OCR. Keep it out of an
    # OCR-only accuracy claim; a separate auto plan checks the native-text path.
    pdfmetrics.registerFont(TTFont("FixtureSans", str(args.fonts / "DejaVuSans.ttf")))
    target = args.output / "native_pdf.pdf"
    pdf = canvas.Canvas(str(target), pagesize=(595.44, 841.68), invariant=1, pageCompression=1)
    pdf.setFont("FixtureSans", 11)
    for i, line in enumerate(BODY):
        pdf.drawString(42, 792 - 20 * i, line)
    pdf.showPage()
    pdf.save()
    truth = args.output / "native_pdf.txt"
    truth.write_text("\n".join(BODY) + "\n", encoding="utf-8")
    cases.append({"id": "native_pdf", "document_id": "operations_report", "input": target.name,
                  "truth": truth.name, "input_sha256": sha(target), "truth_sha256": sha(truth),
                  "media_type": "application/pdf", "pages": 1, "kind": "native_text_pdf",
                  "expected_ocr_calls_auto": 0, "synthetic": True})
    manifest = {"schema_version": 1, "synthetic": True, "seed": args.seed,
                "description": "Deterministic integration smoke fixtures. Not real scans; not evidence of production OCR accuracy.",
                "fonts_sha256": font_hashes, "cases": cases}
    (args.output / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"manifest": str(args.output / "manifest.json"), "cases": len(cases)}))


if __name__ == "__main__":
    main()
