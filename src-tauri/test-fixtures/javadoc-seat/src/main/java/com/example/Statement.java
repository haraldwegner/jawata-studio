package com.example;

import java.util.ArrayList;
import java.util.List;

public class Statement {

    private final List<String> lines = new ArrayList<>();

    public void addLine(String description, long amountCents) {
        lines.add(description + ": " + amountCents);
    }

    public String render(String title) {
        StringBuilder sb = new StringBuilder("== " + title + " ==\n");
        for (String line : lines) {
            sb.append(line).append('\n');
        }
        return sb.toString();
    }

    public int lineCountMatching(String needle) {
        int n = 0;
        for (String line : lines) {
            if (line.contains(needle)) {
                n++;
            }
        }
        return n;
    }

    public void truncateTo(int maxLines) {
        while (lines.size() > maxLines) {
            lines.remove(lines.size() - 1);
        }
    }

    public String lastLineOrDefault(String fallback) {
        return lines.isEmpty() ? fallback : lines.get(lines.size() - 1);
    }
}
