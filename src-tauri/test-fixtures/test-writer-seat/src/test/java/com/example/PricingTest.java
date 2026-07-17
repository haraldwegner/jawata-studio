package com.example;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;

/** Happy path only — every branch beyond it is deliberately uncovered. */
class PricingTest {

    @Test
    void plainTotalWithoutDiscounts() {
        assertEquals(500, new Pricing().discountedCents(100, 5, false));
    }
}
