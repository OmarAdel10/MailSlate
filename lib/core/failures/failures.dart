import 'package:mailslate/src/rust/api/system.dart' show DaemonError;

/// A user-facing failure derived from a bridge or daemon error.
sealed class AppFailure implements Exception {
  const AppFailure(this.message, {required this.kind, this.retryable = false});

  /// What the user sees; never contains mail content or credentials.
  final String message;

  /// Machine-readable kind (snake_case, mirrors the protocol `ErrorKind`).
  final String kind;

  /// Whether retrying the same action can succeed.
  final bool retryable;
}

/// The daemon is not running or cannot be reached.
final class DaemonUnavailableFailure extends AppFailure {
  const DaemonUnavailableFailure({String? message, bool? retryable})
    : super(
        message ?? 'Sync paused — daemon unavailable',
        kind: 'daemon_unavailable',
        retryable: retryable ?? true,
      );
}

/// Transport failure between the app and the network or daemon.
final class NetworkFailure extends AppFailure {
  const NetworkFailure({String? message, bool? retryable})
    : super(
        message ?? 'Connection problem — check your network',
        kind: 'network',
        retryable: retryable ?? true,
      );
}

/// The account's credentials expired and must be re-authorized.
final class AuthExpiredFailure extends AppFailure {
  const AuthExpiredFailure({String? message, bool? retryable})
    : super(
        message ?? 'Sign-in expired — reconnect this account',
        kind: 'auth_expired',
        retryable: retryable ?? false,
      );
}

/// Credentials were rejected by the provider.
final class AuthFailedFailure extends AppFailure {
  const AuthFailedFailure({String? message, bool? retryable})
    : super(
        message ?? 'Sign-in failed — check your credentials',
        kind: 'auth_failed',
        retryable: retryable ?? false,
      );
}

/// The mail server rejected the operation.
final class ServerRejectedFailure extends AppFailure {
  const ServerRejectedFailure({String? message, bool? retryable})
    : super(
        message ?? 'The mail server rejected the request',
        kind: 'server_rejected',
        retryable: retryable ?? false,
      );
}

/// The referenced object no longer exists.
final class NotFoundFailure extends AppFailure {
  const NotFoundFailure({String? message, bool? retryable})
    : super(
        message ?? 'That item no longer exists',
        kind: 'not_found',
        retryable: retryable ?? false,
      );
}

/// The caller supplied invalid input.
final class ValidationFailure extends AppFailure {
  const ValidationFailure({String? message, bool? retryable})
    : super(
        message ?? 'That input is not valid',
        kind: 'validation',
        retryable: retryable ?? false,
      );
}

/// Local persistence failed.
final class StorageFailure extends AppFailure {
  const StorageFailure({String? message, bool? retryable})
    : super(
        message ?? 'A local storage error occurred',
        kind: 'storage',
        retryable: retryable ?? false,
      );
}

/// App and daemon speak different protocol versions.
final class ProtocolMismatchFailure extends AppFailure {
  const ProtocolMismatchFailure({String? message, bool? retryable})
    : super(
        message ?? 'App and daemon versions do not match — update MailSlate',
        kind: 'protocol_mismatch',
        retryable: retryable ?? false,
      );
}

/// Anything unexpected; the raw detail rides in [message].
final class UnexpectedFailure extends AppFailure {
  const UnexpectedFailure({String? message, bool? retryable})
    : super(
        message ?? 'Something went wrong',
        kind: 'internal',
        retryable: retryable ?? false,
      );
}

/// Maps a bridge error onto the app's failure taxonomy.
AppFailure mapBridgeError(DaemonError error) {
  final message = error.message.isEmpty ? null : error.message;
  return switch (error.kind) {
    'daemon_unavailable' => DaemonUnavailableFailure(
      message: message,
      retryable: error.retryable,
    ),
    'network' => NetworkFailure(message: message, retryable: error.retryable),
    'auth_expired' => AuthExpiredFailure(
      message: message,
      retryable: error.retryable,
    ),
    'auth_failed' => AuthFailedFailure(
      message: message,
      retryable: error.retryable,
    ),
    'server_rejected' => ServerRejectedFailure(
      message: message,
      retryable: error.retryable,
    ),
    'not_found' => NotFoundFailure(
      message: message,
      retryable: error.retryable,
    ),
    'validation' => ValidationFailure(
      message: message,
      retryable: error.retryable,
    ),
    'storage' => StorageFailure(message: message, retryable: error.retryable),
    'protocol_mismatch' => ProtocolMismatchFailure(
      message: message,
      retryable: error.retryable,
    ),
    _ => UnexpectedFailure(message: message, retryable: error.retryable),
  };
}
