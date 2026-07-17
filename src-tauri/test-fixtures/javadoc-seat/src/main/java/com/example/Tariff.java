package com.example;

public class Tariff {

    public static final long BASE_FEE_CENTS = 250;

    private final double ratePercent;
    private final long capCents;

    public Tariff(double ratePercent, long capCents) {
        this.ratePercent = ratePercent;
        this.capCents = capCents;
    }

    public long feeFor(long amountCents) {
        long fee = BASE_FEE_CENTS + Math.round(amountCents * ratePercent / 100.0);
        return Math.min(fee, capCents);
    }

    public boolean capsAt(long amountCents) {
        return feeFor(amountCents) == capCents;
    }

    public double getRatePercent() {
        return ratePercent;
    }

    public long getCapCents() {
        return capCents;
    }
}
