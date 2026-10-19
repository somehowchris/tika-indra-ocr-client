/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import java.io.IOException;
import java.io.InputStream;
import org.apache.tika.exception.TikaException;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.parser.Parser;
import org.apache.tika.parser.ParserDecorator;
import org.xml.sax.ContentHandler;
import org.xml.sax.SAXException;

/** Brackets the actual synchronous parser call, including embedded/PDF OCR. */
public final class TracedParser extends ParserDecorator {
    private static final long serialVersionUID = 1L;

    public TracedParser(Parser parser) {
        super(parser);
        TraceRuntime.initialize();
    }

    @Override
    public void parse(InputStream input, ContentHandler handler, Metadata metadata,
                      ParseContext context) throws IOException, SAXException, TikaException {
        RequestTrace previous = context.get(RequestTrace.class);
        try (var call = TraceRuntime.parse(previous, getWrappedParser().getClass())) {
            context.set(RequestTrace.class, call.context());
            try {
                super.parse(input, handler, metadata, context);
            } catch (IOException | SAXException | TikaException | RuntimeException e) {
                call.failed(e);
                throw e;
            } finally {
                // Tika owns one mutable ParseContext per request. No thread-local
                // scope or global environment is used; nested calls restore it.
                context.set(RequestTrace.class, previous);
            }
        }
    }
}
