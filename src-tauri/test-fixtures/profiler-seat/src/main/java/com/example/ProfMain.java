package com.example;

/**
 * Profiler-seat fixture: a plain-java main (the EXECSIM-mirroring launch
 * shape — see run.sh) with one CPU-hot method and one named latency seam.
 */
public final class ProfMain {

    private static volatile double sink;

    public static void main(String[] args) throws Exception {
        long until = System.currentTimeMillis() + 120_000;
        while (System.currentTimeMillis() < until) {
            hotSpot();
            seam("tick");
        }
    }

    /** The CPU hot path — pure computation, no waiting. */
    static void hotSpot() {
        double acc = 0;
        for (int i = 1; i < 220_000; i++) {
            acc += Math.sqrt(i) * Math.log(i + 1);
        }
        sink = acc;
    }

    /** The named seam — latency dominated by WAITING, not computation. */
    static String seam(String input) throws InterruptedException {
        Thread.sleep(8);
        return input.toUpperCase();
    }
}
