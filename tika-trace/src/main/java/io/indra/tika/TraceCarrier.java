/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import java.util.List;
import java.util.Map;

/** Immutable, bounded W3C v00 transport. No baggage or document metadata. */
final class TraceCarrier {
    static final TraceCarrier EMPTY = new TraceCarrier(null, null);
    final String parent;
    final String state;

    private TraceCarrier(String parent, String state) {
        this.parent = parent;
        this.state = state;
    }

    static TraceCarrier from(String parent, String state) {
        return validParent(parent) ? new TraceCarrier(parent, canonicalState(state)) : EMPTY;
    }

    static TraceCarrier headers(Map<String, ? extends List<String>> headers) {
        String parent = null;
        StringBuilder states = null;
        int parents = 0;
        int fields = 0;
        int stateEntries = 0;
        boolean invalidState = false;
        for (var entry : headers.entrySet()) {
            if (entry.getKey().equalsIgnoreCase("traceparent")) {
                for (String value : entry.getValue()) {
                    if (++parents > 1 || !validParent(value)) {
                        return EMPTY;
                    }
                    parent = value;
                }
            } else if (entry.getKey().equalsIgnoreCase("tracestate") && !invalidState) {
                // A general JAX-RS Map exposes order within one value list,
                // not wire order across distinct case-variant map keys.
                if (++stateEntries > 1) {
                    invalidState = true;
                    continue;
                }
                for (String value : entry.getValue()) {
                    if (++fields > 32 || value == null || value.length() > 512) {
                        invalidState = true;
                        break;
                    }
                    if (states == null) {
                        states = new StringBuilder(512);
                    } else {
                        states.append(',');
                    }
                    if (states.length() + value.length() > 512) {
                        invalidState = true;
                        break;
                    }
                    states.append(value);
                }
            }
        }
        return from(parent, invalidState || states == null ? null : states.toString());
    }

    static boolean validParent(String value) {
        if (value == null || value.length() != 55 || !value.startsWith("00-")
                || value.charAt(35) != '-' || value.charAt(52) != '-') {
            return false;
        }
        boolean trace = false;
        boolean span = false;
        for (int i = 3; i < 55; ++i) {
            if (i == 35 || i == 52) {
                continue;
            }
            char c = value.charAt(i);
            if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) {
                return false;
            }
            if (i < 35 && c != '0') {
                trace = true;
            } else if (i > 35 && i < 52 && c != '0') {
                span = true;
            }
        }
        return trace && span;
    }

    private static boolean keyTail(String key) {
        for (int i = 0; i < key.length(); ++i) {
            char c = key.charAt(i);
            if (!lower(c) && !digit(c) && c != '_' && c != '-' && c != '*' && c != '/') {
                return false;
            }
        }
        return true;
    }

    private static boolean lower(char c) { return c >= 'a' && c <= 'z'; }
    private static boolean digit(char c) { return c >= '0' && c <= '9'; }
    private static boolean ows(char c) { return c == ' ' || c == '\t'; }

    private static boolean validKey(String key) {
        if (key.isEmpty() || key.length() > 256) {
            return false;
        }
        int at = key.indexOf('@');
        if (at < 0) {
            return lower(key.charAt(0)) && keyTail(key);
        }
        String tenant = key.substring(0, at);
        String system = key.substring(at + 1);
        return !tenant.isEmpty() && tenant.length() <= 241
                && !system.isEmpty() && system.length() <= 14
                && (lower(tenant.charAt(0)) || digit(tenant.charAt(0)))
                && lower(system.charAt(0)) && keyTail(tenant) && keyTail(system);
    }

    private static String canonicalState(String raw) {
        if (raw == null || raw.length() > 512) {
            return null;
        }
        String[] keys = new String[32];
        int used = 0;
        int members = 0;
        StringBuilder output = new StringBuilder(raw.length());
        for (int offset = 0; offset <= raw.length();) {
            if (++members > 32) {
                return null;
            }
            int end = raw.indexOf(',', offset);
            if (end < 0) {
                end = raw.length();
            }
            int start = offset;
            offset = end + 1;
            while (start < end && ows(raw.charAt(start))) { ++start; }
            while (end > start && ows(raw.charAt(end - 1))) { --end; }
            if (start == end) {
                continue;
            }
            int equal = raw.indexOf('=', start);
            if (equal < start || equal >= end) {
                return null;
            }
            String key = raw.substring(start, equal);
            if (!validKey(key) || equal + 1 == end || end - equal - 1 > 256) {
                return null;
            }
            for (int i = 0; i < used; ++i) {
                if (keys[i].equals(key)) { return null; }
            }
            for (int i = equal + 1; i < end; ++i) {
                char c = raw.charAt(i);
                if (c < 0x20 || c > 0x7e || c == '=') { return null; }
            }
            keys[used++] = key;
            if (!output.isEmpty()) { output.append(','); }
            output.append(raw, start, end);
        }
        return output.isEmpty() ? null : output.toString();
    }
}
