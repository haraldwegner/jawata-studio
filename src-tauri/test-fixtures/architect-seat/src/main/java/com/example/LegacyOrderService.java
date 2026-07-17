package com.example;

/**
 * Seeded smells: type-code constants + a long switch-heavy method — the
 * architect seat's detector targets (type_code, long_method,
 * switch_statements; primitive_obsession rides along).
 */
public class LegacyOrderService {

    public static final int STATUS_NEW = 0;
    public static final int STATUS_PAID = 1;
    public static final int STATUS_SHIPPED = 2;
    public static final int STATUS_CANCELLED = 3;

    private int status = STATUS_NEW;
    private long totalCents;
    private String customer;

    public String processTransition(int event, long amountCents, String note) {
        StringBuilder log = new StringBuilder();
        switch (status) {
            case STATUS_NEW:
                if (event == 1) {
                    if (amountCents <= 0) {
                        log.append("reject: zero payment; ");
                        break;
                    }
                    totalCents = amountCents;
                    status = STATUS_PAID;
                    log.append("paid ").append(amountCents).append("; ");
                } else if (event == 9) {
                    status = STATUS_CANCELLED;
                    log.append("cancelled from new; ");
                } else {
                    log.append("ignored event ").append(event).append(" in NEW; ");
                }
                break;
            case STATUS_PAID:
                if (event == 2) {
                    status = STATUS_SHIPPED;
                    log.append("shipped; ");
                    if (note != null && !note.isEmpty()) {
                        log.append("note: ").append(note).append("; ");
                    }
                } else if (event == 9) {
                    status = STATUS_CANCELLED;
                    log.append("refund ").append(totalCents).append("; ");
                    totalCents = 0;
                } else {
                    log.append("ignored event ").append(event).append(" in PAID; ");
                }
                break;
            case STATUS_SHIPPED:
                if (event == 9) {
                    log.append("cannot cancel shipped; ");
                } else {
                    log.append("terminal state; ");
                }
                break;
            case STATUS_CANCELLED:
                log.append("cancelled is final; ");
                break;
            default:
                log.append("unknown status ").append(status).append("; ");
        }
        if (customer != null) {
            log.append("customer=").append(customer);
        }
        return log.toString();
    }

    public void setCustomer(String customer) {
        this.customer = customer;
    }

    public int getStatus() {
        return status;
    }
}
