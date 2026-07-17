package com.example;

public class ThinFacts {

    public int process(int x) {
        int a = x ^ 0x5f3759df;
        a = (a << 3) - a;
        return a % 977;
    }
}
