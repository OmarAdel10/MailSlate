import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/logging/log.dart';

void main() {
  test('log helpers never throw', () {
    logInfo('test_info');
    logWarning(
      'test_warning',
      error: StateError('warn'),
      stackTrace: StackTrace.current,
    );
    logError(
      'test_error',
      error: StateError('boom'),
      stackTrace: StackTrace.current,
    );
  });
}
