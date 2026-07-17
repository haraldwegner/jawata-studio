package com.example;

/**
 * Seeded incomplete delegation: holds a {@link ReportEngine} delegate,
 * forwards two operations — and hand-rolls the third instead of forwarding,
 * duplicating the engine's formatting logic.
 */
public class ReportFacade {

    private final ReportEngine engine = new ReportEngine();

    public String header(String title) {
        return engine.header(title);
    }

    public String footer(int pages) {
        return engine.footer(pages);
    }

    public String body(String content) {
        // Hand-rolled instead of engine.body(content) — the seeded decay.
        StringBuilder sb = new StringBuilder();
        for (String line : content.split("\n")) {
            sb.append("| ").append(line.trim()).append(" |\n");
        }
        return sb.toString();
    }
}
