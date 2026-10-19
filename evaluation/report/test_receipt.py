"""Dependency-free standalone report resource checks; no browser or network."""
import unittest
import receipt
import study


class EmbeddedResourcesTests(unittest.TestCase):
    def test_embedded_images_fonts_styles_and_citation_links(self):
        html = '<img src="data:image/png;base64,YQ=="><style>@font-face {src:url(data:font/woff2;base64,YQ==)}</style><a href="https://example.org/paper">source</a><script>const local = true;</script>'
        self.assertEqual(receipt.verify_embedded_resources(html), 2)

    def test_unembedded_automatic_assets_fail(self):
        for html in ('<script src="https://cdn.example.org/widget.js"></script>', '<img src="chart.png">', '<link rel="stylesheet" href="//fonts.example.org/a.css">', '<style>@import "https://fonts.example.org/a.css";</style>', '<style>body {background:url(https://example.org/a.png)}</style>'):
            with self.subTest(html=html), self.assertRaises(study.AdmissionError):
                receipt.verify_embedded_resources(html)


if __name__ == "__main__":
    unittest.main()
