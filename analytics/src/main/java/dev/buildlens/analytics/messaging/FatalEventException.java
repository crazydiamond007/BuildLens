package dev.buildlens.analytics.messaging;

final class FatalEventException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    FatalEventException(String message) {
        super(message);
    }
}
