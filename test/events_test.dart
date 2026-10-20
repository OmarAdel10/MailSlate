import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/ffi/events.dart';

import 'support/fake_ffi_client.dart';

void main() {
  test('fans every event out to all listeners', () async {
    final client = FakeFfiClient()
      ..eventStream = () => Stream<AppEvent>.fromIterable([
        const AppEvent.accountStatus(account: 7, state: 'idle'),
        const AppEvent.syncProgress(
          account: 7,
          mailbox: 'INBOX',
          phase: 'initial',
          done: 1,
          total: 3,
        ),
      ]);
    final broadcast = appEventStream(client);

    final first = broadcast.toList();
    final second = broadcast.toList();
    final results = await Future.wait([first, second]);

    expect(results.first, [
      const AppEvent.accountStatus(account: 7, state: 'idle'),
      const AppEvent.syncProgress(
        account: 7,
        mailbox: 'INBOX',
        phase: 'initial',
        done: 1,
        total: 3,
      ),
    ]);
    expect(results.last, results.first);
  });

  test(
    'cancelling the last listener tears the source connection down',
    () async {
      var sourceCancelled = false;
      final controller = StreamController<AppEvent>(
        onCancel: () => sourceCancelled = true,
      );
      final client = FakeFfiClient()..eventStream = () => controller.stream;
      final broadcast = appEventStream(client);

      final subscription = broadcast.listen(null);
      await Future<void>.delayed(Duration.zero);

      await subscription.cancel();
      await Future<void>.delayed(Duration.zero);

      expect(sourceCancelled, isTrue);
    },
  );

  test('a listener that arrives later dials the source back up', () async {
    var subscriptions = 0;
    StreamController<AppEvent>? active;
    Stream<AppEvent> freshSource() {
      active = StreamController<AppEvent>(onListen: () => subscriptions += 1);
      return active!.stream;
    }

    final client = FakeFfiClient()..eventStream = freshSource;
    final broadcast = appEventStream(client);

    final firstSubscription = broadcast.listen(null);
    await Future<void>.delayed(Duration.zero);
    await firstSubscription.cancel();
    await Future<void>.delayed(Duration.zero);

    final secondSubscription = broadcast.listen(null);
    await Future<void>.delayed(Duration.zero);
    await secondSubscription.cancel();
    await Future<void>.delayed(Duration.zero);

    expect(subscriptions, 2);
    expect(active!.hasListener, isFalse);
  });
}
