/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import org.apache.tika.parser.ParseContext;

/** The sole patched OCR hook; command, files and parser settings remain Tika-owned. */
public final class OcrCall implements AutoCloseable {
    private final TraceRuntime.Call call;

    private OcrCall(TraceRuntime.Call call) { this.call = call; }

    public static OcrCall start(ParseContext context, ProcessBuilder process) {
        var call = TraceRuntime.ocr(context.get(RequestTrace.class));
        var carrier = call.context().carrier();
        var environment = process.environment();
        // Never inherit stale deployment-level request context, even if tracing
        // is disabled or this particular HTTP request has no valid parent.
        environment.remove("INDRA_TRACEPARENT");
        environment.remove("INDRA_TRACESTATE");
        if (carrier.parent != null) { environment.put("INDRA_TRACEPARENT", carrier.parent); }
        if (carrier.state != null) { environment.put("INDRA_TRACESTATE", carrier.state); }
        return new OcrCall(call);
    }

    public void failed(Throwable error) { call.failed(error); }
    @Override
    public void close() { call.close(); }
}
