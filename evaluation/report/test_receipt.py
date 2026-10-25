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


class ObjectReprTests(unittest.TestCase):
    def test_leaked_figure_and_artist_reprs_fail(self):
        # Run 39837656304 printed six of these after the study figures.
        for html in ('<pre><code>&lt;Figure size 1209.6x806.4 with 0 Axes&gt;</code></pre>',
                     '<pre><code>&lt;Figure size 2308.54x883.2 with 3 Axes&gt;</code></pre>',
                     '<code>&lt;seaborn.axisgrid.FacetGrid at 0x7f3a2c1d9e50&gt;</code>',
                     '<code>[&lt;matplotlib.lines.Line2D object at 0x10a2b3c40&gt;]</code>'):
            with self.subTest(html=html), self.assertRaises(study.AdmissionError):
                receipt.verify_no_object_reprs(html)

    def test_figure_captions_and_images_pass(self):
        receipt.verify_no_object_reprs('<figure><img src="data:image/png;base64,YQ=="><figcaption>Figure size and 0 axes are not a repr</figcaption></figure>')


if __name__ == "__main__":
    unittest.main()
