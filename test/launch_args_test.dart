import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/routing/launch_args.dart';

void main() {
  group('parseLaunchArgs', () {
    test('parses the space-separated form', () {
      final args = parseLaunchArgs(['--open-thread', 'thread-1']);
      expect(args.openThreadId, 'thread-1');
    });

    test('parses the equals form', () {
      final args = parseLaunchArgs(['--open-thread=thread-2']);
      expect(args.openThreadId, 'thread-2');
    });

    test('ignores unrelated arguments', () {
      expect(
        parseLaunchArgs(['--verbose', 'x', '-d', 'linux']).openThreadId,
        isNull,
      );
      expect(parseLaunchArgs(const []).openThreadId, isNull);
    });

    test('ignores a flag without a usable value', () {
      expect(parseLaunchArgs(['--open-thread']).openThreadId, isNull);
      expect(parseLaunchArgs(['--open-thread=']).openThreadId, isNull);
      expect(
        parseLaunchArgs(['--open-thread', '--other']).openThreadId,
        isNull,
      );
    });

    test('takes the first usable occurrence', () {
      final args = parseLaunchArgs([
        '--open-thread',
        'first',
        '--open-thread=second',
      ]);
      expect(args.openThreadId, 'first');
    });
  });
}
