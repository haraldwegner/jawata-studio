package com.example;

/**
 * Order pricing. The bulk-discount, loyalty and clamp branches are
 * deliberately uncovered by the existing test — the test-writer seat's
 * target surface. {@link #scale(int)} is the ambiguous-intent symbol.
 */
public final class Pricing {

    /**
     * Computes the discounted total in cents.
     *
     * @param baseCents unit price in cents, must be non-negative
     * @param quantity number of units
     * @param loyal whether the loyalty rebate applies
     * @return the total after discounts, clamped at zero
     */
    public long discountedCents(long baseCents, int quantity, boolean loyal) {
        if (baseCents < 0) {
            throw new IllegalArgumentException("baseCents must be >= 0");
        }
        long total = baseCents * quantity;
        if (quantity >= 10) {
            total -= total / 10;
        }
        if (loyal) {
            total -= 50;
        }
        if (total < 0) {
            total = 0;
        }
        return total;
    }

    /**
     * Scales the input. TODO: the intended semantics of this scaling are
     * not documented anywhere — an ambiguous-intent target.
     *
     * @param x the input
     * @return the scaled value
     */
    public int scale(int x) {
        return (x * 7) % 13;
    }
}
