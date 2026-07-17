package com.example;

/** The delegate {@link ReportFacade} only half-uses. */
public class ReportEngine {

    public String header(String title) {
        return "== " + title + " ==\n";
    }

    public String body(String content) {
        StringBuilder sb = new StringBuilder();
        for (String line : content.split("\n")) {
            sb.append("| ").append(line.trim()).append(" |\n");
        }
        return sb.toString();
    }

    public String footer(int pages) {
        return "-- " + pages + " pages --\n";
    }
}
