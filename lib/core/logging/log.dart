/// Minimal structured logging for the GUI.
///
/// Never log secrets, tokens, passwords, or message bodies — event names,
/// counts, and identifiers only (ARCHITECTURE §16).
library;

import 'dart:developer' as developer;

const String _loggerName = 'mailslate';

/// Logs an informational event.
void logInfo(String event) {
  developer.log(event, name: _loggerName, level: 800);
}

/// Logs a recoverable problem.
void logWarning(String event, {Object? error, StackTrace? stackTrace}) {
  developer.log(
    event,
    name: _loggerName,
    level: 900,
    error: error,
    stackTrace: stackTrace,
  );
}

/// Logs a failure.
void logError(String event, {Object? error, StackTrace? stackTrace}) {
  developer.log(
    event,
    name: _loggerName,
    level: 1000,
    error: error,
    stackTrace: stackTrace,
  );
}
