package com.example;

/**
 * Debugger-seat fixture with ONE seeded defect. Spec: a cart totalling
 * EXACTLY 10000 cents gets the 500-cent loyalty rebate. The seeded bug:
 * {@link #rebate(long)} uses a strict comparison, so the boundary cart
 * misses its rebate and the grand total comes out 500 too high.
 */
public final class BillingMain {

    /** Spec: rebate applies from 10000 cents INCLUSIVE. */
    static long rebate(long totalCents) {
        if (totalCents > 10_000) { // seeded bug: must be >=
            return 500;
        }
        return 0;
    }

    public static void main(String[] args) throws Exception {
        long[] carts = {2_000, 10_000, 15_000};
        long grand = 0;
        for (long cart : carts) {
            grand += cart - rebate(cart);
        }
        // Spec expects 2000 + 9500 + 14500 = 26000; the bug yields 26500.
        System.out.println("GRAND_TOTAL=" + grand);
        Thread.sleep(120_000); // stay alive for the debugger
    }
}
