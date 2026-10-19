/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import io.opentelemetry.api.trace.Span;
import io.opentelemetry.api.trace.SpanContext;
import io.opentelemetry.api.trace.TraceFlags;
import io.opentelemetry.api.trace.TraceState;
import io.opentelemetry.context.Context;

/** Separate from mutable OCR settings; a later X-Tika-OCR provider cannot replace it. */
record RequestTrace(Context context) {
    static final RequestTrace EMPTY = new RequestTrace(Context.root());

    static RequestTrace from(TraceCarrier carrier) {
        if (carrier.parent == null) { return EMPTY; }
        var state = TraceState.builder();
        if (carrier.state != null) {
            // TraceState.put prepends. Reverse insertion preserves wire order.
            String[] members = carrier.state.split(",");
            for (int i = members.length - 1; i >= 0; --i) {
                int equal = members[i].indexOf('=');
                state.put(members[i].substring(0, equal), members[i].substring(equal + 1));
            }
        }
        var remote = SpanContext.createFromRemoteParent(
                carrier.parent.substring(3, 35), carrier.parent.substring(36, 52),
                (Character.digit(carrier.parent.charAt(54), 16) & 1) == 1
                        ? TraceFlags.getSampled() : TraceFlags.getDefault(), state.build());
        return new RequestTrace(Context.root().with(Span.wrap(remote)));
    }

    TraceCarrier carrier() {
        var ids = Span.fromContext(context).getSpanContext();
        if (!ids.isValid()) { return TraceCarrier.EMPTY; }
        StringBuilder state = new StringBuilder(512);
        ids.getTraceState().forEach((key, value) -> {
            if (!state.isEmpty()) { state.append(','); }
            state.append(key).append('=').append(value);
        });
        return TraceCarrier.from("00-" + ids.getTraceId() + "-" + ids.getSpanId()
                + (ids.isSampled() ? "-01" : "-00"), state.toString());
    }
}
