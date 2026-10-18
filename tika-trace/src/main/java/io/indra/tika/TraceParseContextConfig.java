/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import jakarta.ws.rs.core.MultivaluedMap;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.server.core.ParseContextConfig;

/** Stateless SPI provider. No ordering dependency on the standard OCR/PDF providers. */
public final class TraceParseContextConfig implements ParseContextConfig {
    @Override
    public void configure(MultivaluedMap<String, String> headers, Metadata metadata,
                          ParseContext context) {
        context.set(RequestTrace.class, RequestTrace.from(TraceCarrier.headers(headers)));
    }
}
