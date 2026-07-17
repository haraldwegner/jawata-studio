package com.example;

public class Account {

    private final String owner;
    private long balanceCents;

    public Account(String owner, long initialBalanceCents) {
        this.owner = owner;
        this.balanceCents = initialBalanceCents;
    }

    public String getOwner() {
        return owner;
    }

    public long getBalanceCents() {
        return balanceCents;
    }

    public void deposit(long amountCents) {
        if (amountCents < 0) {
            throw new IllegalArgumentException("amountCents must be >= 0");
        }
        balanceCents += amountCents;
    }
}
