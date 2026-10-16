import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/src/rust/api/system.dart' show DaemonError;

void main() {
  group('mapBridgeError', () {
    test('maps each ErrorKind to its failure type', () {
      const cases = <String, Type>{
        'daemon_unavailable': DaemonUnavailableFailure,
        'network': NetworkFailure,
        'auth_expired': AuthExpiredFailure,
        'auth_failed': AuthFailedFailure,
        'server_rejected': ServerRejectedFailure,
        'not_found': NotFoundFailure,
        'validation': ValidationFailure,
        'storage': StorageFailure,
        'protocol_mismatch': ProtocolMismatchFailure,
        'internal': UnexpectedFailure,
      };

      cases.forEach((kind, type) {
        final failure = mapBridgeError(
          DaemonError(kind: kind, message: 'detail', retryable: true),
        );
        expect(failure.runtimeType, type, reason: kind);
        expect(failure.message, 'detail', reason: kind);
        expect(failure.retryable, isTrue, reason: kind);
        expect(failure.kind, kind, reason: kind);
      });
    });

    test('falls back to UnexpectedFailure for unknown kinds', () {
      final failure = mapBridgeError(
        const DaemonError(kind: 'brand_new', message: '', retryable: false),
      );
      expect(failure, isA<UnexpectedFailure>());
      expect(failure.message, 'Something went wrong');
      expect(failure.retryable, isFalse);
    });

    test('uses the friendly default when the daemon sends no message', () {
      final failure = mapBridgeError(
        const DaemonError(
          kind: 'daemon_unavailable',
          message: '',
          retryable: true,
        ),
      );
      expect(failure.message, 'Sync paused — daemon unavailable');
    });

    test('failures are throwable exceptions', () {
      expect(const DaemonUnavailableFailure(), isA<Exception>());
    });
  });
}
