/* SPDX-License-Identifier: Apache-2.0 */
package io.indra.tika;

import java.time.Duration;
import java.util.Set;
import java.util.concurrent.TimeUnit;

/** Shutdown-only grace for drained child JVMs; normal watchdog recovery is unchanged. */
public final class ChildShutdown {
    private ChildShutdown() { }

    public static void finish(Set<Process> children) {
        finish(children, Duration.ofSeconds(5), Duration.ofSeconds(1));
    }

    static void finish(Set<Process> children, Duration grace, Duration reap) {
        boolean interrupted = false;
        long deadline = System.nanoTime() + grace.toNanos();
        for (Process child : children) { child.destroy(); }
        for (Process child : children) {
            long remaining = deadline - System.nanoTime();
            if (remaining <= 0) { break; }
            try {
                child.waitFor(remaining, TimeUnit.NANOSECONDS);
            } catch (InterruptedException ignored) {
                interrupted = true;
                break;
            }
        }
        for (Process child : children) {
            if (child.isAlive()) { child.destroyForcibly(); }
        }
        deadline = System.nanoTime() + reap.toNanos();
        for (Process child : children) {
            long remaining = deadline - System.nanoTime();
            if (remaining <= 0) { break; }
            try {
                child.waitFor(remaining, TimeUnit.NANOSECONDS);
            } catch (InterruptedException ignored) {
                interrupted = true;
                break;
            }
        }
        if (interrupted) { Thread.currentThread().interrupt(); }
    }
}
