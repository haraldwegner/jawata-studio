package com.example;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Characterization tests pinning the state machine BEFORE the dispatched
 * refactoring (replace_type_code_with_class) — the parity evidence: these
 * must pass identically before and after the plan is applied.
 */
class LegacyOrderServiceTest {

    @Test
    void newOrderGetsPaid() {
        LegacyOrderService s = new LegacyOrderService();
        String log = s.processTransition(1, 5000, null);
        assertTrue(log.contains("paid 5000"), log);
        assertEquals(LegacyOrderService.STATUS_PAID, s.getStatus());
    }

    @Test
    void zeroPaymentIsRejected() {
        LegacyOrderService s = new LegacyOrderService();
        String log = s.processTransition(1, 0, null);
        assertTrue(log.contains("reject: zero payment"), log);
        assertEquals(LegacyOrderService.STATUS_NEW, s.getStatus());
    }

    @Test
    void cancellingPaidRefunds() {
        LegacyOrderService s = new LegacyOrderService();
        s.processTransition(1, 700, null);
        String log = s.processTransition(9, 0, null);
        assertTrue(log.contains("refund 700"), log);
        assertEquals(LegacyOrderService.STATUS_CANCELLED, s.getStatus());
    }

    @Test
    void shippedCannotBeCancelled() {
        LegacyOrderService s = new LegacyOrderService();
        s.processTransition(1, 700, null);
        s.processTransition(2, 0, "fragile");
        String log = s.processTransition(9, 0, null);
        assertTrue(log.contains("cannot cancel shipped"), log);
        assertEquals(LegacyOrderService.STATUS_SHIPPED, s.getStatus());
    }
}
