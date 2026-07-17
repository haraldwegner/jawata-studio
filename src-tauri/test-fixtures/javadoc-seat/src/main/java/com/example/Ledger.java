package com.example;

import java.util.ArrayList;
import java.util.List;

public class Ledger {

    private final List<Long> entries = new ArrayList<>();
    private long openingBalanceCents;

    public Ledger(long openingBalanceCents) {
        this.openingBalanceCents = openingBalanceCents;
    }

    public void post(long amountCents) {
        entries.add(amountCents);
    }

    public long balanceCents() {
        long sum = openingBalanceCents;
        for (long e : entries) {
            sum += e;
        }
        return sum;
    }

    public int entryCount() {
        return entries.size();
    }

    public boolean isEmpty() {
        return entries.isEmpty();
    }

    public long largestEntryCents() {
        long max = Long.MIN_VALUE;
        for (long e : entries) {
            if (e > max) {
                max = e;
            }
        }
        return entries.isEmpty() ? 0 : max;
    }

    public void reset(long newOpeningBalanceCents) {
        entries.clear();
        this.openingBalanceCents = newOpeningBalanceCents;
    }
}
